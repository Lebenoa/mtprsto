//! List the account's available peers.
//!
//! Connects with a saved user session and prints `users` and `chats`
//! (ids, names, usernames, access hashes) for everything the account
//! currently has conversations with.
//!
//! **User-session flow.** Create an authorized session first with the
//! demo's interactive phone login:
//!
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example demo -- --user-phone +15551234567
//! ```
//!
//! Then run (session defaults to the demo's saved path):
//!
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... cargo run --example list_peers
//! ```
//!
//! NOTE: handles both `messages.dialogs` and `messages.dialogsSlice`
//! responses, including the live `dialog#fc89f7f3` shape (with
//! `unread_poll_votes_count` and `draftMessageEmpty` drafts).

use mtprsto::Client;

/// Default to the demo's saved user session (see `channel_admin`).
fn default_session_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mtprsto_demo_session.json")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let api_id: i32 = std::env::var("TELEGRAM_API_ID")?.parse()?;
    let api_hash = std::env::var("TELEGRAM_API_HASH")?;

    let client = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session(default_session_path())
        .build()?;
    client.connect().await?;
    println!("connected");

    match client.get_me().await {
        Ok(me) => println!(
            "me: id={} first={:?} phone={:?} username={:?}",
            me.id().0,
            me.first_name(),
            me.phone(),
            me.username()
        ),
        Err(e) => println!("get_me failed: {e}"),
    }

    let dialogs = client.get_dialogs().await?;
    println!(
        "{} dialogs, {} users, {} chats",
        dialogs.dialogs.len(),
        dialogs.users.len(),
        dialogs.chats.len()
    );

    println!("\n=== USERS (username / access hash) ===");
    for u in &dialogs.users {
        println!(
            "user id={:<12} first={:<15?} phone={:<15} username={:<20?} access_hash={:?}",
            u.id().0,
            u.first_name(),
            u.phone().unwrap_or(""),
            u.username(),
            u.access_hash().map(|h| h.0)
        );
    }

    println!("\n=== CHATS/CHANNELS ===");
    for c in &dialogs.chats {
        match c {
            mtprsto::types::Chat::Channel {
                id,
                title,
                username,
                access_hash,
                megagroup,
                ..
            } => {
                // Print the Bot-API-style -100 id — exactly the form
                // `send_to_channel` (and resolve_peer) accepts.
                let send_id = format!("-100{}", id.0);
                let hash_ok = access_hash.is_some();
                println!(
                    "channel send_id={:<16} title={:<25?} username={:<20?} megagroup={} hash_persisted={}",
                    send_id,
                    title,
                    username,
                    megagroup,
                    hash_ok
                );
            }
            mtprsto::types::Chat::Chat {
                id,
                title,
                participants_count,
                ..
            } => {
                println!(
                    "basic chat id={} title={:?} members={}",
                    id.0, title, participants_count
                );
            }
            other => println!("other chat: {other:?}"),
        }
    }

    Ok(())
}
