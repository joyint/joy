// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: MIT

//! Commit-message suggestion for the `prepare-commit-msg` hook (JOY-01B1-FF).
//!
//! Pure logic: given the staged item IDs, the current in-progress items, the
//! changed code crates, and the acting identity, build the text Joy pre-fills
//! into the commit editor. The CLI side gathers these inputs from git and the
//! project; this module never touches git or the filesystem so it stays fully
//! unit-testable.
//!
//! Design contract (see the item): produce either a *complete* subject the
//! user can just save, or an *empty* subject with candidates offered as
//! commented-out ready lines. Never a half-filled placeholder that has to be
//! cursored over and deleted. Comment (`#`) lines are stripped by git's
//! default editor cleanup, so they never reach the stored commit.

use crate::model::item::ItemType;

/// Canonical `Co-Authored-By:` value for an AI member id, or `None` when the
/// member is not a known tool. Maps `ai:claude@joy` -> the `claude` tool line.
/// Owned here (not in the AI template layer) so commit-message building stays
/// AI-free at the crate level (ADR-043). Brand names are allowed in this
/// trailer and only here; the rest of the project uses the Joy member ID.
pub fn coauthor_line_for_member(member_id: &str) -> Option<&'static str> {
    let tool = member_id
        .strip_prefix("ai:")
        .and_then(|rest| rest.split('@').next())?;
    let line = coauthor_line_for_tool(tool);
    (!line.is_empty()).then_some(line)
}

/// `<Brand> <email>` co-author value for an AI tool (empty for an unknown
/// tool), matching each tool's own standalone-commit form.
pub fn coauthor_line_for_tool(tool: &str) -> &'static str {
    match tool {
        "claude" => "Claude <noreply@anthropic.com>",
        "copilot" => "Copilot <copilot@github.com>",
        "qwen" => "Qwen-Coder <qwen-coder@alibabacloud.com>",
        "vibe" => "Mistral Vibe <vibe@mistral.ai>",
        _ => "",
    }
}

/// A candidate item for the message (id + type + title).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRef {
    pub id: String,
    pub item_type: ItemType,
    pub title: String,
}

/// Who is committing, as resolved by joy-core identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Committer {
    /// Member id (email or `ai:tool@joy`).
    pub member: String,
    /// The delegating human, present only for an authenticated AI session.
    pub delegated_by: Option<String>,
}

/// Inputs for [`build_suggestion`].
#[derive(Debug, Clone)]
pub struct Inputs {
    /// Items whose `.joy/items/*.yaml` files are staged in this commit.
    /// These are exact matches (the id comes from the filename).
    pub staged_items: Vec<ItemRef>,
    /// All items currently in `in-progress` (used only when nothing is staged).
    pub in_progress: Vec<ItemRef>,
    /// Distinct crate names among the staged *code* changes (e.g. "joy-cli").
    /// A scope is only emitted when there is exactly one.
    pub changed_crates: Vec<String>,
    /// The acting committer, for trailers.
    pub committer: Committer,
}

/// Conventional-commit type for an item type.
fn conventional_type(t: &ItemType) -> &'static str {
    match t {
        ItemType::Bug => "fix",
        ItemType::Rework => "rework",
        ItemType::Story | ItemType::Epic | ItemType::Task => "feat",
        ItemType::Decision | ItemType::Idea => "docs",
        ItemType::Job => "chore",
    }
}

/// Map a crate name to a conventional scope (`joy-cli` -> `cli`,
/// `joy-core` -> `core`, otherwise the crate name unchanged).
fn scope_for_crate(krate: &str) -> String {
    krate.strip_prefix("joy-").unwrap_or(krate).to_string()
}

/// The trailing block: trailers only under an AI delegation, never for a plain
/// human commit (where Delegated-By would be meaningless and Co-Authored-By
/// wrong). Returns lines without a leading blank line.
fn trailer_lines(c: &Committer) -> Vec<String> {
    let mut out = Vec::new();
    // Trailers only under an AI delegation (ai:* member with a delegating
    // operator). A plain human commit gets none.
    if let (true, Some(op)) = (c.member.starts_with("ai:"), &c.delegated_by) {
        if let Some(coauthor) = coauthor_line_for_member(&c.member) {
            out.push(format!("Co-Authored-By: {coauthor}"));
        }
        out.push(format!("Delegated-By: {op}"));
    }
    out
}

/// Build a complete subject line for one or more items.
fn subject_for(items: &[ItemRef], changed_crates: &[String]) -> String {
    // Type from the first item (when several are staged they usually share a
    // change); conventional commits allow only one type.
    let ty = conventional_type(&items[0].item_type);
    let scope = if changed_crates.len() == 1 {
        format!("({})", scope_for_crate(&changed_crates[0]))
    } else {
        String::new()
    };
    let ids: String = items
        .iter()
        .map(|i| format!("[{}]", i.id))
        .collect::<Vec<_>>()
        .join(" ");
    // Title from the first item; extra IDs still get referenced.
    format!("{ty}{scope}: {} {ids}", items[0].title)
}

/// Assemble the final message text given a subject (may be empty) and any
/// comment lines. `subject` is line 1; trailers follow after a blank line;
/// comments go last (git strips them).
fn assemble(subject: &str, trailers: &[String], comments: &[String]) -> String {
    let mut s = String::new();
    s.push_str(subject);
    s.push('\n');
    if !trailers.is_empty() {
        s.push('\n');
        for t in trailers {
            s.push_str(t);
            s.push('\n');
        }
    }
    if !comments.is_empty() {
        s.push('\n');
        for c in comments {
            s.push_str("# ");
            s.push_str(c);
            s.push('\n');
        }
    }
    s
}

/// Build the suggested commit-message text.
///
/// Priority (staged-before-status):
/// 1. staged item file(s) -> complete subject with their id(s).
/// 2. else exactly one in-progress item -> complete subject with its id.
/// 3. else ambiguous/none -> empty subject + candidates as commented lines.
pub fn build_suggestion(inp: &Inputs) -> String {
    let trailers = trailer_lines(&inp.committer);

    // 1. staged item file(s): exact.
    if !inp.staged_items.is_empty() {
        let subject = subject_for(&inp.staged_items, &inp.changed_crates);
        return assemble(&subject, &trailers, &[]);
    }

    // 2. exactly one in-progress item.
    if inp.in_progress.len() == 1 {
        let subject = subject_for(&inp.in_progress, &inp.changed_crates);
        return assemble(&subject, &trailers, &[]);
    }

    // 3. ambiguous or none: empty subject + candidate comment lines.
    let mut comments = Vec::new();
    if inp.in_progress.is_empty() {
        comments.push("joy: no in-progress item and no .joy/ item staged.".to_string());
        comments.push("add a [<ID>] to the subject, or use [no-item].".to_string());
    } else {
        comments.push(
            "joy: several in-progress items. Uncomment one line, or write your own:".to_string(),
        );
        for it in &inp.in_progress {
            comments.push(subject_for(std::slice::from_ref(it), &inp.changed_crates));
        }
    }
    assemble("", &trailers, &comments)
}

/// A commit message that references no Joy item (D3.3 of the forge
/// connection NG design). Carries what the ADR-015 diagnostic needs, so
/// the text is built once and every caller prints the same thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMsgError {
    /// The project acronym the message had to name.
    pub acronym: String,
    /// The first line of the message, quoted back in the diagnostic.
    pub subject: String,
}

impl std::fmt::Display for CommitMsgError {
    /// The ADR-015 diagnostic the bash hook prints, so a caller that
    /// refuses can print this and nothing else.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { acronym, subject } = self;
        writeln!(f, "commit message must reference a Joy item")?;
        writeln!(f, "  |")?;
        writeln!(f, "  | {subject}")?;
        writeln!(f, "  | ^ no {acronym}-XXXX item ID found")?;
        writeln!(f, "  |")?;
        writeln!(
            f,
            "  = help: add an item ID to your commit message (e.g. {acronym}-0001)"
        )?;
        write!(
            f,
            "  = note: use [no-item] tag for infrastructure commits without a Joy item"
        )
    }
}

impl std::error::Error for CommitMsgError {}

/// The item reference rule of `.joy/hooks/commit-msg`, in process (D3.3).
///
/// libgit2 runs no hooks, so every commit joy writes itself passes this
/// instead of the bash hook. The rule is the hook's, verbatim: a
/// `[no-item]` anywhere bypasses it, an empty acronym is no rule at all
/// (the hook exits 0 when it finds none), and an item id is the acronym
/// followed by four hex digits and an optional two more, matched anywhere
/// in the message.
///
/// The merge exemption is the CALLER's: the hook reads `MERGE_HEAD` from
/// the git directory, and this function is given a message and nothing
/// else. joy's own commit paths never write a merge commit through it.
pub fn validate(message: &str, acronym: &str) -> Result<(), CommitMsgError> {
    let acronym = acronym.trim();
    if acronym.is_empty() || message.contains("[no-item]") || references_item(message, acronym) {
        return Ok(());
    }
    Err(CommitMsgError {
        acronym: acronym.to_string(),
        subject: message.lines().next().unwrap_or_default().to_string(),
    })
}

/// D3.3 for joy's OWN automatic commits: warn and proceed.
///
/// A refusal here would strand the write joy has already made with
/// uncommitted `.joy` changes and a message the person never typed and
/// cannot fix. One line, not the full ADR-015 diagnostic, for the same
/// reason: the diagnostic belongs to the paths that refuse, which are
/// the commands a person ran.
pub fn warn_unless_referenced(message: &str, acronym: &str) {
    if let Err(e) = validate(message, acronym) {
        eprintln!(
            "Warning: the commit joy just wrote references no {} item: {}",
            e.acronym, e.subject
        );
    }
}

/// `ACRONYM-[0-9A-Fa-f]{4}(-[0-9A-Fa-f]{2})?` anywhere in `message`. The
/// optional suffix changes nothing about whether the message matches (a
/// four digit id is already a match), so only the first group is checked,
/// exactly as the hook's `grep -E` decides it.
fn references_item(message: &str, acronym: &str) -> bool {
    let needle = format!("{acronym}-");
    message.match_indices(&needle).any(|(at, _)| {
        message[at + needle.len()..]
            .chars()
            .take(4)
            .filter(|c| c.is_ascii_hexdigit())
            .count()
            == 4
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, t: ItemType, title: &str) -> ItemRef {
        ItemRef {
            id: id.to_string(),
            item_type: t,
            title: title.to_string(),
        }
    }

    fn human() -> Committer {
        Committer {
            member: "horst@joydev.com".into(),
            delegated_by: None,
        }
    }

    fn ai() -> Committer {
        Committer {
            member: "ai:claude@joy".into(),
            delegated_by: Some("horst@joydev.com".into()),
        }
    }

    #[test]
    fn staged_item_gives_complete_subject_no_placeholder() {
        let inp = Inputs {
            staged_items: vec![item(
                "JOY-01B1-FF",
                ItemType::Rework,
                "pre-fill commit message",
            )],
            in_progress: vec![],
            changed_crates: vec!["joy-cli".into()],
            committer: human(),
        };
        let msg = build_suggestion(&inp);
        let first = msg.lines().next().unwrap();
        assert_eq!(first, "rework(cli): pre-fill commit message [JOY-01B1-FF]");
        // human commit: no trailers
        assert!(!msg.contains("Delegated-By"));
        assert!(!msg.contains("Co-Authored-By"));
        // no placeholder text to delete
        assert!(!msg.contains("<type>"));
        assert!(!msg.contains("<describe"));
    }

    #[test]
    fn single_in_progress_used_when_nothing_staged() {
        let inp = Inputs {
            staged_items: vec![],
            in_progress: vec![item("JOY-0042-AB", ItemType::Bug, "fix the thing")],
            changed_crates: vec!["joy-core".into()],
            committer: human(),
        };
        let msg = build_suggestion(&inp);
        assert_eq!(
            msg.lines().next().unwrap(),
            "fix(core): fix the thing [JOY-0042-AB]"
        );
    }

    #[test]
    fn multiple_in_progress_offers_commented_candidates_empty_subject() {
        let inp = Inputs {
            staged_items: vec![],
            in_progress: vec![
                item("JOY-0001-AA", ItemType::Task, "task one"),
                item("JOY-0002-BB", ItemType::Bug, "bug two"),
            ],
            changed_crates: vec![],
            committer: human(),
        };
        let msg = build_suggestion(&inp);
        // subject line is empty
        assert_eq!(msg.lines().next().unwrap(), "");
        // candidates present as comment lines (stripped by git on save)
        assert!(msg.contains("# feat: task one [JOY-0001-AA]"));
        assert!(msg.contains("# fix: bug two [JOY-0002-BB]"));
    }

    #[test]
    fn no_candidate_gives_hint_only() {
        let inp = Inputs {
            staged_items: vec![],
            in_progress: vec![],
            changed_crates: vec![],
            committer: human(),
        };
        let msg = build_suggestion(&inp);
        assert_eq!(msg.lines().next().unwrap(), "");
        assert!(msg.contains("# joy: no in-progress item"));
        assert!(msg.contains("[no-item]"));
    }

    #[test]
    fn scope_omitted_when_multiple_crates() {
        let inp = Inputs {
            staged_items: vec![item("JOY-0003-CC", ItemType::Story, "spanning change")],
            in_progress: vec![],
            changed_crates: vec!["joy-cli".into(), "joy-core".into()],
            committer: human(),
        };
        let msg = build_suggestion(&inp);
        assert_eq!(
            msg.lines().next().unwrap(),
            "feat: spanning change [JOY-0003-CC]"
        );
    }

    #[test]
    fn ai_delegation_adds_trailers() {
        let inp = Inputs {
            staged_items: vec![item("JOY-0004-DD", ItemType::Task, "do it")],
            in_progress: vec![],
            changed_crates: vec!["joy-cli".into()],
            committer: ai(),
        };
        let msg = build_suggestion(&inp);
        assert!(msg.contains("Co-Authored-By: Claude <noreply@anthropic.com>"));
        assert!(msg.contains("Delegated-By: horst@joydev.com"));
    }

    #[test]
    fn multiple_staged_items_reference_all_ids() {
        let inp = Inputs {
            staged_items: vec![
                item("JOY-0005-EE", ItemType::Task, "first"),
                item("JOY-0006-FF", ItemType::Task, "second"),
            ],
            in_progress: vec![],
            changed_crates: vec!["joy-cli".into()],
            committer: human(),
        };
        let msg = build_suggestion(&inp);
        let first = msg.lines().next().unwrap();
        assert!(first.contains("[JOY-0005-EE]"));
        assert!(first.contains("[JOY-0006-FF]"));
    }

    /// D3.3: the in-process rule answers exactly what the bash hook
    /// answers, for the cases the hook enumerates.
    #[test]
    fn the_item_rule_matches_the_hook() {
        assert!(validate("feat: [JOY-0005-EE] do it", "JOY").is_ok());
        assert!(validate("feat: JOY-0005 do it", "JOY").is_ok());
        assert!(validate("chore: bump the app [no-item]", "JOY").is_ok());
        // An id somewhere in the body counts, as `grep -E` over the whole
        // message counts it.
        assert!(validate("joy: add a task\n\nrefs JOY-00ab", "JOY").is_ok());
        // No acronym, no rule: the hook exits 0 on a project without one.
        assert!(validate("joy: anything", "").is_ok());

        // Too few digits, the wrong acronym, and nothing at all.
        assert!(validate("joy: add a task", "JOY").is_err());
        assert!(validate("feat: JOY-12 do it", "JOY").is_err());
        assert!(validate("feat: [ANO-0005-EE] do it", "JOY").is_err());
    }

    /// The refusal carries the ADR-015 diagnostic, so a caller only has
    /// to print it.
    #[test]
    fn the_refusal_reads_like_the_hook() {
        let err = validate("joy: auth passphrase", "ANO").unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("commit message must reference a Joy item"),
            "{text}"
        );
        assert!(text.contains("| joy: auth passphrase"), "{text}");
        assert!(text.contains("no ANO-XXXX item ID found"), "{text}");
        assert!(text.contains("use [no-item] tag"), "{text}");
    }
}
