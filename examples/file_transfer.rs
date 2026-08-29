//! File upload/download with configurable parallelism.
//!
//! Upload: `saveFilePart`/`saveBigFilePart` across parallel workers
//! (`.workers(n)`). Download: parallel contiguous ranges once the file
//! exceeds `DownloadConfig::parallel_threshold` (default 8 MiB / 4 ranges,
//! tuned here via `.download_config(...)`).
//!
//! Run:
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example file_transfer -- 123456:BOT_TOKEN @someuser ./video.mp4
//! ```

use mtprsto::client::Client;
use mtprsto::file::DownloadConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut args = std::env::args().skip(1);
    let bot_token = args.next().expect("usage: file_transfer <BOT_TOKEN> <PEER> <PATH>");
    let peer = args.next().expect("missing <PEER>");
    let path = args.next().expect("missing <PATH>");

    let api_id: i32 = std::env::var("TELEGRAM_API_ID")?.parse()?;
    let api_hash = std::env::var("TELEGRAM_API_HASH")?;

    // Tune parallel download: fetch files ≥ 4 MiB as 6 concurrent ranges
    // across the pool's aux connections.
    let mut client = Client::builder()
        .api_id(api_id)
        .api_hash(api_hash)
        .session("bot.session")
        .download_config(DownloadConfig {
            parallel_threshold: 4 * 1024 * 1024,
            parallel_count: 6,
        })
        .build()?;

    client.connect().await?;
    client.authorize_bot(&bot_token).await?;
    println!("authorized");

    // Upload with 4 parallel workers and send it as a document.
    let msg_id = client
        .send_file(&peer, &path)
        .await
        .caption("uploaded by mtprsto")
        .workers(4)
        .send()
        .await?;
    println!("uploaded and sent as message {msg_id:?}");

    // Download it back. The media's file location and size come from the
    // sent message; here we just demonstrate the client-level API — pass
    // the FileLocation from a fetched message's Document/Photo and its size.
    println!(
        "download config in effect: threshold={} bytes, {} parallel ranges",
        client.download_config().parallel_threshold,
        client.download_config().parallel_count,
    );

    // Clean up the sent message.
    client.delete_messages(&[msg_id]).await?;
    println!("cleaned up");

    Ok(())
}
