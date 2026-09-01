//! Send messages as a bot using the high-level builder API.
//!
//! Demonstrates `client.send`, the fluent message builder
//! (`.reply_to().silent()`), and typed error handling with retries.
//!
//! Run:
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example bot_send -- 123456:BOT_TOKEN @someuser "hello"
//! ```

use mtprsto::Client;
use mtprsto::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut args = std::env::args().skip(1);
    let bot_token = args
        .next()
        .expect("usage: bot_send <BOT_TOKEN> <PEER> [TEXT]");
    let peer = args.next().expect("missing <PEER>");
    let text = args.next().unwrap_or_else(|| "Hello from mtprsto!".into());

    let api_id: i32 = std::env::var("TELEGRAM_API_ID")?.parse()?;
    let api_hash = std::env::var("TELEGRAM_API_HASH")?;

    let client = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session("bot.session")
        .build()?;

    client.connect().await?;
    client.authorize_bot(&bot_token).await?;
    println!("authorized");

    // Plain send — peer can be a user/chat/channel ID or @username.
    let msg_id = client.send(&peer, &text).await?;
    println!("sent message {msg_id:?}");

    // Fluent builder: reply to it, silently, without a link preview.
    let reply = client
        .message(&peer, "got it")
        .await? // resolves the peer; `?` surfaces a bad peer
        .reply_to(msg_id)
        .silent()
        .no_webpage()
        .send()
        .await?;
    println!("sent reply {reply:?}");

    // Typed errors: FLOOD_WAIT_X surfaces as Error::FloodWait with the
    // retry deadline — retry loops are one `is_transient()` check.
    match client.send(&peer, "ping").await {
        Ok(_) => println!("ping sent"),
        Err(Error::FloodWait { seconds, .. }) => {
            println!("flood-waited for {seconds}s — try again later");
        }
        Err(e) if e.is_transient() => println!("transient failure, can retry: {e}"),
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
