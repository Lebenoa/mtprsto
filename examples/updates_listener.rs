//! Listen to real-time updates via the background pump.
//!
//! `Client::updates(poll_interval)` starts the pump: it polls
//! `updates.getState`, runs pts/seq/qts gap detection, recovers missed
//! updates via `updates.getDifference`, resyncs channels flagged by
//! `UpdateChannelTooLong` via `updates.getChannelDifference`, and streams
//! decoded `Update` events to the returned `mpsc` receiver.
//!
//! Run:
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example updates_listener -- 123456:BOT_TOKEN
//! ```

use mtprsto::Client;
use mtprsto::types::Update;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let bot_token = std::env::args()
        .nth(1)
        .expect("usage: updates_listener <BOT_TOKEN>");

    let api_id: i32 = std::env::var("TELEGRAM_API_ID")?.parse()?;
    let api_hash = std::env::var("TELEGRAM_API_HASH")?;

    let client = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session("bot.session")
        .build()?;

    client.connect().await?;
    client.authorize_bot(&bot_token).await?;
    println!("authorized — listening for updates (Ctrl-C to stop)");

    // Start the pump; poll server state every 15 s. Returns None if the
    // pump is already running or the client is not connected.
    let mut rx = client.updates(15).expect("updates pump did not start");

    while let Some(update) = rx.recv().await {
        match update {
            Update::NewMessage { message, .. } => {
                let from = message
                    .from_id()
                    .and_then(|p| p.user_id())
                    .map(|u| u.0.to_string())
                    .unwrap_or_else(|| "?".into());
                println!("[{from}] {}", message.text());
            }
            Update::EditMessage { message, .. } => {
                println!("[edited] {}", message.text());
            }
            Update::DeleteMessages { messages, .. } => {
                println!("[deleted] {} message(s)", messages.len());
            }
            Update::ChannelTooLong { channel_id, .. } => {
                // The pump resyncs this channel automatically; the event is
                // still delivered so listeners can react if they want to.
                println!("[resync] channel {}", channel_id.0);
            }
            other => {
                println!("[update] {other:?}");
            }
        }
    }

    Ok(())
}
