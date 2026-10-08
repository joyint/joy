//! Records a team chat with an AI member in it, the way this release
//! stored one. Run by tests/fixtures/chat-from-before/record.sh of a
//! later release, against a checkout of THIS one.
use chrono::{Duration, Utc};
use joy_chat_store::chats;
use joy_core::member_ref::MemberRef;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (root, person, passphrase, ai) = (
        std::path::Path::new(&args[1]),
        args[2].as_str(),
        args[3].as_str(),
        args[4].as_str(),
    );
    let project = joy_core::store::load_project(root).expect("the project");
    let member = project.member_by_key(person).expect("the person");
    let unlocked = joy_core::auth::unlock_identity(member, passphrase).expect("the passphrase");
    joy_chat_store::writer::set_seed(Some(unlocked.seed));

    let at = Utc::now() - Duration::days(2);
    let me = MemberRef::new(person);
    let ai_ref = MemberRef::new(ai);
    let mut chat = chats::open_chat(
        root,
        vec![me.clone(), ai_ref.clone()],
        Some("From before".into()),
        at,
    )
    .expect("the chat");
    chats::append_message(
        root,
        &mut chat,
        me.clone(),
        "a first line from before",
        at + Duration::seconds(1),
    )
    .expect("the line");
    chats::append_ai_reply(
        root,
        &mut chat,
        ai_ref.clone(),
        "an answer from before",
        at + Duration::seconds(5),
        None,
        Some(person.to_string()),
        Some(1200),
        Some(0),
        None,
    )
    .expect("the answer");
    chats::set_ai_session(
        root,
        &mut chat,
        &ai_ref,
        "acp-from-before",
        at + Duration::seconds(6),
    )
    .expect("the session");
    println!("{}", chat.id);
}
