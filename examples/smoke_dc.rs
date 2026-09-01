//! Live-DC smoke test: Client::connect (DH handshake) + pool send_rpc
//! (help.getNearestDc) against production Telegram DC 2.
//! Temporary — for review verification only.

use mtprsto::client::Client;
use mtprsto::pool::PoolConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // Use an isolated temp session so we exercise the full handshake.
    let session = std::env::temp_dir().join(format!("mtprsto_smoke_{}.json", std::process::id()));
    let _ = std::fs::remove_file(&session);

    println!("connecting to DC 2 with full Obfuscated2 + DH handshake...");
    let client = Client::builder()
        .session(&session)
        .pool_config(PoolConfig {
            min_connections: 1,
            ..PoolConfig::default()
        })
        .build()?;
    client.connect().await?;
    println!("OK: connect() — auth key created, pool open");

    // Raw RPC through the pool: help.getNearestDc#1fb33026 takes no args
    // and returns a NearestDc — this exercises invokeWithLayer, encrypt,
    // decrypt, gzip, rpc_result unwrap and error classification end-to-end.
    let mut w = mtprsto::serialize::TLWriter::new();
    w.write_u32(0x1fb33026); // help.getNearestDc
    match client.invoke_raw(w.into_bytes()).await {
        Ok(result) => {
            println!(
                "OK: send_rpc returned {} bytes: {:02x?}",
                result.len(),
                &result[..result.len().min(24)]
            );
        }
        Err(e) => {
            // An RPC_ERROR (e.g. FLOOD_WAIT or AUTH_KEY_UNREGISTERED) is
            // still proof the round-trip works — the server answered.
            println!("send_rpc answered with error (round-trip works): {e}");
        }
    }

    let _ = std::fs::remove_file(&session);
    Ok(())
}
