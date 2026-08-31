//! Send messages and files to a channel/supergroup addressed by its
//! Bot-API-style **-100 id** (e.g. `-1001234567890`).
//!
//! The channel's access hash is resolved automatically: first from the
//! session's persisted id→hash cache, then via a `channels.getChannels`
//! bootstrap (works for any channel the account is a member/admin of).
//!
//! Before anything is sent the example prints the target and asks
//! **"Can I send to this channel?"** — proceed only after answering `y`.
//! Pass `--yes` to skip the prompt (scripted runs).
//!
//! Usage:
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example send_to_channel -- [--yes] -1001234567890 "hello channel" [FILE]
//! ```
//!
//! **User-session flow.** Create an authorized session first with the
//! demo's interactive phone login (see `channel_admin.rs`).

use mtprsto::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let yes = args.first().map(String::as_str) == Some("--yes");
    if yes {
        args.remove(0);
    }
    let channel_id = args.first().expect("usage: send_to_channel [--yes] <CHANNEL_ID> [TEXT] [FILE]").clone();
    let text = args.get(1).cloned().unwrap_or_else(|| "hello from mtprsto".into());
    let file = args.get(2).cloned();

    if !channel_id.starts_with("-100") {
        return Err(format!("expected a -100… channel id, got {channel_id}").into());
    }

    let api_id: i32 = std::env::var("TELEGRAM_API_ID")?.parse()?;
    let api_hash = std::env::var("TELEGRAM_API_HASH")?;

    let session = default_session_path();
    if !session.exists() {
        return Err(format!(
            "no user session at {} — create one first with the demo's --user-phone login",
            session.display()
        )
        .into());
    }

    let mut client = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session(&session)
        .build()?;
    client.connect().await?;
    println!("connected");

    // SAFETY GATE: confirm the target before anything is sent.
    println!("target channel : {channel_id}");
    println!("text           : {text:?}");
    if let Some(f) = &file {
        println!("file           : {f:?}");
    }
    if !yes {
        print!("Can I send to this channel? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut ans = String::new();
        std::io::stdin().read_line(&mut ans)?;
        let ans = ans.trim().to_ascii_lowercase();
        if ans != "y" && ans != "yes" {
            println!("aborted — nothing sent");
            return Ok(());
        }
    }

    // The -100 id resolves through the session hash cache or a
    // channels.getChannels bootstrap inside resolve_peer.
    let msg = client.send(&channel_id, &text).await?;
    println!("sent message id {:?}", msg);

    if let Some(path) = file {
        let id = client.send_file(&channel_id, &path).await.send().await?;
        println!("sent file {:?} as message id {:?}", path, id);
    }

    Ok(())
}

/// Default to the demo's saved user session (see `channel_admin.rs`).
fn default_session_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mtprsto_demo_session.json")
}
