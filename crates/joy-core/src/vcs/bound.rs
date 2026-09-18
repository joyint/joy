// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! joy's own bound on a call the operating system does not bound
//! (design D1.9, JOY-02A7-A2).
//!
//! `vcs::forge::bound_forge_waits` sets libgit2's two socket bounds,
//! `GIT_OPT_SET_SERVER_CONNECT_TIMEOUT` and `GIT_OPT_SET_SERVER_TIMEOUT`.
//! They are stored in `git_socket_stream__connect_timeout` and
//! `git_socket_stream__timeout` (settings.c:435-465), and exactly two
//! places read them: `streams/socket.c:380-381` and
//! `transports/ssh_libssh2.c:551-552`. On Windows the https transport
//! is WinHTTP, which opens no socket stream at all: it sets its own
//! timeouts with `WinHttpSetTimeouts(handle, TIMEOUT_INFINITE,
//! DEFAULT_CONNECT_TIMEOUT, TIMEOUT_INFINITE, TIMEOUT_INFINITE)` from
//! two local variables (winhttp.c:381-382, :423, :785-786, :856), where
//! `TIMEOUT_INFINITE` is -1 and nothing reads joy's option. A forge that
//! accepts the connection and then says nothing therefore holds a joy
//! contact on Windows for ever, and the callbacks cannot end it either:
//! libgit2 calls none of them while it waits for the first byte.
//!
//! So joy bounds the call from the outside. The work runs on a thread of
//! its own, the caller waits for it, and a caller that waited out the
//! bound gives up on the thread rather than on the process. Giving up
//! leaves the thread inside the operating system call, which is the
//! price of a call that cannot be cancelled, so it is counted: a key
//! (one forge host, or the ssh agent) may leave at most
//! [`ABANDONED_PER_KEY`] such threads behind, and the next call for that
//! key answers "no answer" at once instead of adding another. A poll
//! loop against a black hole therefore costs two threads, not one per
//! second.
//!
//! The bound is a SILENCE bound, not a cap on the work as a whole, which
//! is what the two libgit2 options are and what a clone of a large
//! repository needs. The work reports that it is still being answered
//! through [`heartbeat`], which every libgit2 callback joy installs
//! calls; while the heartbeat moves, the bound does not run out.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many threads one key may leave behind inside a call that never
/// returned. The next call for that key is answered without one.
const ABANDONED_PER_KEY: usize = 2;

/// How often the waiting caller looks at the heartbeat and runs its own
/// tick. Small enough to carry a clone's progress at a rate a person
/// reads as live, large enough to cost nothing.
const TICK: Duration = Duration::from_millis(100);

/// What the waiting caller reads while the work runs: how often the
/// work said it is alive, and whether it is waiting for a PERSON right
/// now, which no bound may cut short.
#[derive(Default)]
struct Beat {
    ticks: AtomicU64,
    holds: AtomicUsize,
}

thread_local! {
    /// The heartbeat of the work running on THIS thread, when it runs
    /// under [`within_silence`]. libgit2 calls its callbacks
    /// synchronously on the thread that made the call, which is exactly
    /// the scope this cell needs.
    static HEARTBEAT: RefCell<Option<Arc<Beat>>> = const { RefCell::new(None) };
}

/// A person is being asked something on this thread, so the bound is
/// held open until the guard is dropped.
///
/// A Git Credential Manager window and joy's own host key question both
/// sit inside a libgit2 callback and both are answered by a person, in
/// their own time. Neither is silence, and a bound that cut them off
/// would throw away the answer the person was typing (design D1.3's
/// prompt deadline is the bound that belongs to those, and it is the
/// helper runner's own).
pub fn hold() -> Hold {
    HEARTBEAT.with(|cell| {
        let beat = cell.borrow().clone();
        if let Some(beat) = beat.as_ref() {
            beat.holds.fetch_add(1, Ordering::SeqCst);
        }
        Hold(beat)
    })
}

/// The guard of [`hold`]: the wait is open while it lives.
pub struct Hold(Option<Arc<Beat>>);

impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(beat) = self.0.as_ref() {
            beat.holds.fetch_sub(1, Ordering::SeqCst);
            beat.ticks.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The work on this thread is still being answered: whoever waits for it
/// starts the silence bound again.
///
/// Called from every libgit2 callback joy installs. Off a bounded thread
/// it costs one thread local read and does nothing.
pub fn heartbeat() {
    HEARTBEAT.with(|cell| {
        if let Some(beat) = cell.borrow().as_ref() {
            beat.ticks.fetch_add(1, Ordering::Relaxed);
        }
    });
}

/// What the work says to the caller while it runs, and what it hears
/// back.
///
/// The one caller is the clone of D4.3: its progress callback belongs to
/// the person who started the clone, it is not `Send` and it may not be
/// moved to another thread, and its answer STOPS the download, so it has
/// to be asked and answered while the transfer waits. Every count
/// therefore crosses back to the caller's thread and the work waits for
/// the verdict, which is the same order libgit2's own callback has.
pub struct Reporting<M, R> {
    notes: std::sync::mpsc::Sender<Note<M, R>>,
}

enum Note<M, R> {
    Aside(M, std::sync::mpsc::SyncSender<R>),
    Finished,
}

impl<M, R> Reporting<M, R> {
    /// Say `note` to the caller and wait for its answer. `None` when
    /// nobody is listening any more, which is what a caller that gave up
    /// on this work leaves behind.
    pub fn say(&self, note: M) -> Option<R> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.notes.send(Note::Aside(note, reply)).ok()?;
        answer.recv().ok()
    }
}

/// Run `work` on a thread of its own and wait for it until it has been
/// silent for `silence`.
///
/// `None` means joy gave up: either the work was silent for the whole
/// bound, or `key` had already left [`ABANDONED_PER_KEY`] threads behind
/// and this call was not made at all. Both are the same answer to the
/// caller ("nobody answered"), and both are logged with the reason.
///
/// A panic in `work` is carried back and raised on the caller's thread,
/// so the bound changes where the work runs and nothing else.
pub fn within_silence<T: Send + 'static>(
    key: &str,
    what: &'static str,
    silence: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    reporting(key, what, silence, |()| (), move |_| work())
}

/// [`within_silence`] for work that has something to say while it runs.
///
/// `aside` runs on the CALLER's thread for every note the work sends and
/// its answer goes back to the work, which waits for it. That is how a
/// clone's progress reaches the person who started it, and their "stop"
/// reaches the transfer, without the callback ever leaving the thread it
/// belongs to.
pub fn reporting<T, M, R>(
    key: &str,
    what: &'static str,
    silence: Duration,
    mut aside: impl FnMut(M) -> R,
    work: impl FnOnce(Reporting<M, R>) -> T + Send + 'static,
) -> Option<T>
where
    T: Send + 'static,
    M: Send + 'static,
    R: Send + 'static,
{
    if !may_start(key) {
        tracing::warn!(
            key,
            what,
            abandoned = ABANDONED_PER_KEY,
            "not contacted: earlier calls to it never returned and joy holds no more threads for it"
        );
        return None;
    }
    let beat = Arc::new(Beat::default());
    let given_up = Arc::new(AtomicBool::new(false));
    let (sender, answers) = std::sync::mpsc::channel();
    let (say, notes) = std::sync::mpsc::channel();
    let span = tracing::Span::current();
    let worker = {
        let beat = beat.clone();
        let given_up = given_up.clone();
        let key = key.to_string();
        let finished = say.clone();
        std::thread::Builder::new()
            .name(format!("joy-{what}"))
            .spawn(move || {
                let _entered = span.enter();
                HEARTBEAT.with(|cell| *cell.borrow_mut() = Some(beat));
                let reporting = Reporting { notes: say };
                let answer =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(reporting)));
                let _ = sender.send(answer);
                // The answer is announced on the SAME channel the notes
                // travel on, so the caller waits on one thing and hears
                // both the moment they happen.
                let _ = finished.send(Note::Finished);
                // Whoever sets the flag second is the one that has to
                // put the slot back: exactly one of the two sides sees
                // `true` here.
                if given_up.swap(true, Ordering::SeqCst) {
                    release(&key);
                }
            })
    };
    if worker.is_err() {
        // Nothing was counted yet: `may_start` only reads the map, the
        // slot is taken when a call is given up on.
        tracing::warn!(key, what, "no thread for this call");
        return None;
    }
    let started = Instant::now();
    let mut last_beat = 0;
    let mut last_move = Instant::now();
    loop {
        match notes.recv_timeout(TICK) {
            Ok(Note::Aside(note, reply)) => {
                // The work is being answered, and it is waiting for
                // this: not silence, and no bound may run out on it.
                last_move = Instant::now();
                let _ = reply.send(aside(note));
            }
            Ok(Note::Finished) => {
                return match answers.recv() {
                    Ok(Ok(answer)) => Some(answer),
                    Ok(Err(panic)) => std::panic::resume_unwind(panic),
                    Err(_) => None,
                }
            }
            // The thread went away without an answer, which a panic
            // already covers; there is nothing left to wait for.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let now = beat.ticks.load(Ordering::Relaxed);
                if beat.holds.load(Ordering::SeqCst) > 0 {
                    // A person is answering something; that is not
                    // silence, and it has a bound of its own.
                    last_beat = now;
                    last_move = Instant::now();
                } else if now != last_beat {
                    last_beat = now;
                    last_move = Instant::now();
                } else if last_move.elapsed() >= silence {
                    if !given_up.swap(true, Ordering::SeqCst) {
                        abandoned(key);
                    }
                    tracing::warn!(
                        key,
                        what,
                        waited_ms = started.elapsed().as_millis() as u64,
                        "no answer within the bound; the call was given up on"
                    );
                    return None;
                }
            }
        }
    }
}

/// Keys whose calls are still running after joy gave up on them.
static ABANDONED: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

fn with_abandoned<T>(body: impl FnOnce(&mut HashMap<String, usize>) -> T) -> T {
    let mut guard = ABANDONED.lock().unwrap_or_else(|e| e.into_inner());
    body(guard.get_or_insert_with(HashMap::new))
}

/// Whether a call for this key may be made at all.
fn may_start(key: &str) -> bool {
    with_abandoned(|held| held.get(key).copied().unwrap_or(0) < ABANDONED_PER_KEY)
}

fn abandoned(key: &str) {
    with_abandoned(|held| *held.entry(key.to_string()).or_insert(0) += 1);
}

/// A call joy gave up on came back after all, or was never made: the
/// key gets its slot back.
fn release(key: &str) {
    with_abandoned(|held| {
        if let Some(count) = held.get_mut(key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                held.remove(key);
            }
        }
    });
}

/// For the tests: forget which keys have threads outstanding.
#[cfg(test)]
fn forget_abandoned() {
    with_abandoned(|held| held.clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One at a time: the abandoned map is process state.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn work_that_answers_is_answered() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let answer = within_silence("answers", "test", Duration::from_secs(30), || 41 + 1);
        assert_eq!(answer, Some(42));
    }

    /// Silence for the whole bound is given up on, and the caller is
    /// back long before the work is.
    #[test]
    fn silence_for_the_whole_bound_is_given_up_on() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let started = Instant::now();
        let answer = within_silence("silent", "test", Duration::from_millis(300), || {
            std::thread::sleep(Duration::from_secs(30));
            7
        });
        assert_eq!(answer, None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the caller waited for the work"
        );
    }

    /// A heartbeat starts the bound again, so slow but living work is
    /// not cut off: the work below takes six times the bound and is
    /// answered.
    #[test]
    fn a_heartbeat_keeps_the_bound_from_running_out() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let answer = within_silence("living", "test", Duration::from_millis(200), || {
            for _ in 0..12 {
                std::thread::sleep(Duration::from_millis(100));
                heartbeat();
            }
            "done"
        });
        assert_eq!(answer, Some("done"));
    }

    /// A person answering a question is not silence either, and the
    /// work that waits for them says nothing at all while they think.
    #[test]
    fn a_hold_keeps_the_bound_from_running_out() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let answer = within_silence("asking", "test", Duration::from_millis(200), || {
            let _hold = hold();
            std::thread::sleep(Duration::from_millis(900));
            "answered"
        });
        assert_eq!(answer, Some("answered"));
    }

    /// What the work says while it runs is answered on the CALLER's
    /// thread, and the work waits for that answer: this is what carries
    /// a clone's progress to the person who started it and their "stop"
    /// back to the transfer, in that order.
    #[test]
    fn a_note_is_answered_on_the_callers_thread_and_the_work_waits() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let caller = std::thread::current().id();
        let mut seen = Vec::new();
        let answer = reporting(
            "reporting",
            "test",
            Duration::from_secs(30),
            |count: u32| {
                seen.push((count, std::thread::current().id()));
                // "stop after the third"
                count < 3
            },
            |say| {
                let mut said = 0;
                for count in 1.. {
                    said = count;
                    if say.say(count) != Some(true) {
                        break;
                    }
                }
                said
            },
        );
        assert_eq!(answer, Some(3), "the answer stopped the work at once");
        assert_eq!(
            seen.iter().map(|(count, _)| *count).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(
            seen.iter().all(|(_, thread)| *thread == caller),
            "the notes were answered somewhere else"
        );
    }

    /// A key that has left its threads behind is not given another: the
    /// third call is refused without one, and the refusal is the same
    /// "nobody answered" the bound gives.
    #[test]
    fn a_key_leaves_at_most_two_threads_behind() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let stuck = Arc::new(AtomicBool::new(true));
        for _ in 0..ABANDONED_PER_KEY {
            let stuck = stuck.clone();
            assert_eq!(
                within_silence(
                    "black-hole",
                    "test",
                    Duration::from_millis(150),
                    move || {
                        while stuck.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                ),
                None
            );
        }
        let started = Instant::now();
        let answer = within_silence("black-hole", "test", Duration::from_millis(150), || {});
        assert_eq!(answer, None, "the third call gets no thread");
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "and it is answered at once, not after the bound"
        );
        // The threads come back, the slots come back with them.
        stuck.store(false, Ordering::Relaxed);
        let mut waited = 0;
        while with_abandoned(|held| held.contains_key("black-hole")) && waited < 200 {
            std::thread::sleep(Duration::from_millis(10));
            waited += 1;
        }
        assert_eq!(
            within_silence("black-hole", "test", Duration::from_secs(30), || 1),
            Some(1)
        );
    }

    /// A panic inside the work is raised where the caller is, so the
    /// bound moves where the work runs and changes nothing else.
    #[test]
    fn a_panic_reaches_the_caller() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_abandoned();
        let raised = std::panic::catch_unwind(|| {
            within_silence("panicking", "test", Duration::from_secs(30), || {
                panic!("the work gave up")
            })
        });
        assert!(raised.is_err(), "the panic did not reach the caller");
    }
}
