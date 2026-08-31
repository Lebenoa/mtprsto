//! Channel administration: create, inspect, invite, promote, enumerate.
//!
//! Full channel lifecycle with the typed wrappers:
//! - `create_channel` (broadcast or megagroup)
//! - `get_channels` to re-fetch with fresh access hashes
//! - `invite_to_channel` + `edit_admin` (rights bit-mask + rank)
//! - `get_participants` with the recent/search filters
//! - `leave_channel`
//!
//! **User-session flow.** `channels.createChannel` is a user-only RPC —
//! bots get `BOT_METHOD_INVALID` for it (and for `edit_admin`), so this
//! example must NOT be run with a bot token. Create an authorized user
//! session first with the demo's interactive phone login:
//!
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example demo -- --user-phone +15551234567
//! ```
//!
//! The demo writes its session to `%TEMP%/mtprsto_demo_session.json`.
//! Then run, passing the target user to invite/promote:
//!
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... //!   cargo run --example channel_admin -- @someuser
//! ```
//!
//! (An explicit session path can be passed instead of relying on the
//! demo's default location.)
//!
//! The target user must be contactable (username, or a shared chat).

use mtprsto::Client;
use mtprsto::rpc::ChannelParticipantsFilter;
use mtprsto::types::{AccessHash, ChannelId, InputChannel, InputUser};

/// Default to the demo's saved user session when no path is given:
/// the demo's `--user-phone` login writes it to %TEMP%/mtprsto_demo_session.json.
fn default_session_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mtprsto_demo_session.json")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut args = std::env::args().skip(1);
    // Only treat the first argument as a session path if it is not
    // the target handle itself (handles start with "@").
    let (session_path, user) = match args.next() {
        Some(first) if !first.starts_with("@") => {
            let user = args.next().unwrap_or_else(|| panic!("missing <USER>"));
            (std::path::PathBuf::from(first), user)
        }
        first => (default_session_path(), first.expect("missing <USER>")),
    };

    // The API ID/hash must match the ones the session was created with
    // (they are baked into initConnection on every request).
    let api_id: i32 = std::env::var("TELEGRAM_API_ID")?.parse()?;
    let api_hash = std::env::var("TELEGRAM_API_HASH")?;

    // A user-authorized session file — defaults to the demo's saved
    // session (see prerequisite above). Fail fast with instructions
    // instead of a confusing auth failure mid-run.
    if !session_path.exists() {
        return Err(format!(
            "no user session at {} — create one first:
               cargo run --example demo -- --user-phone +15551234567",
            session_path.display()
        )
        .into());
    }
    let mut client = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session(&session_path)
        .build()?;
    client.connect().await?;
    println!("connected with user session");

    // 1. Create a megagroup.
    let chats = client
        .create_channel(
            "mtprsto demo",
            "Created by the channel_admin example",
            false,
            true,
        )
        .await?;
    let Some(created) = chats.iter().find_map(|c| match c {
        mtprsto::types::Chat::Channel {
            id, access_hash, ..
        } => Some((id.0, access_hash.map(|h| h.0).unwrap_or(0))),
        _ => None,
    }) else {
        return Err("create_channel returned no channel".into());
    };
    println!("created channel id={}", created.0);

    // 2. Re-fetch it (fresh access hash straight from the server).
    let channel = InputChannel::Channel {
        channel_id: ChannelId(created.0),
        access_hash: AccessHash(created.1),
    };
    let fetched = client.get_channels(std::slice::from_ref(&channel)).await?;
    for chat in &fetched {
        if let mtprsto::types::Chat::Channel {
            title, username, ..
        } = chat
        {
            println!("get_channels: \"{title}\" username={username:?}");
        }
    }

    // 3. Invite the user, then promote them with a rank.
    let input_user = match client.resolve_username(&user).await? {
        mtprsto::types::InputPeer::User {
            user_id,
            access_hash,
        } => InputUser::User {
            user_id,
            access_hash,
        },
        other => return Err(format!("expected a user peer, got {other:?}").into()),
    };
    match client
        .invite_to_channel(&channel, std::slice::from_ref(&input_user))
        .await
    {
        Ok(_) => println!("invited {user}"),
        Err(e) => println!("invite failed ({e}) — the user may block invites"),
    }

    // Admin rights bit-mask (ChatAdminRights flags): change_info, post
    // messages, edit messages, delete messages, invite users.
    const RIGHTS: i32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 5) | (1 << 10);
    match client
        .edit_admin(&channel, &input_user, RIGHTS, "co-admin")
        .await
    {
        Ok(()) => println!("promoted {user} to co-admin"),
        Err(e) => println!("edit_admin failed: {e}"),
    }

    // 4. Enumerate recent participants.
    let (count, _raw) = client
        .get_participants(&channel, &ChannelParticipantsFilter::Recent, 0, 20, 0)
        .await?;
    println!("get_participants: {count} total (raw page returned)");

    // 5. Leave the channel — set KEEP_CHANNEL to keep it.
    if std::env::var("KEEP_CHANNEL").is_err() {
        client.leave_channel(&channel).await?;
        println!("left and abandoned the demo channel");
    } else {
        println!("KEEP_CHANNEL set — channel {} kept", ChannelId(created.0).0);
    }

    Ok(())
}
