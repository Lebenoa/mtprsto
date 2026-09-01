//! Advanced RPC techniques: raw invoke, request ordering, suppression,
//! low-level service queries, and transient-error retry loops.
//!
//! Covers the escape hatches the high-level helpers don't:
//! - `invoke_raw` for TL payloads the typed surface doesn't cover yet
//! - `invokeAfterMsg` to order dependent RPCs server-side
//! - `invokeWithoutUpdates` to silence the update stream for a call
//! - `SenderPool::get_future_salts` / `query_msgs_state` service messages
//! - the `is_transient()` retry pattern for FLOOD_WAIT / dropped answers
//!
//! Run:
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example advanced_rpc -- 123456:BOT_TOKEN
//! ```

use mtprsto::Client;
use mtprsto::error::Error;
use mtprsto::serialize::TLWriter;

/// Retry a fallible op with backoff while errors are transient
/// (FLOOD_WAIT, network drops, dropped answers).
async fn with_retries<T, F, Fut>(mut op: F) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
{
    for attempt in 0..5u32 {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if e.is_transient() && attempt < 4 => {
                let backoff = 1u64 << attempt; // 1, 2, 4, 8 s
                println!("transient failure ({e}); retrying in {backoff}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let bot_token = std::env::args()
        .nth(1)
        .expect("usage: advanced_rpc <BOT_TOKEN>");
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

    // 1. Raw invoke: any TL method the typed surface doesn't wrap yet.
    //    Here: `users.getUsers` with an `inputUserSelf` to fetch our own
    //    bot account (raw bytes in, typed decode out).
    let own = with_retries(|| async {
        let mut w = TLWriter::new();
        w.write_u32(0x0d91a548); // users.getUsers#0d91a548
        w.write_u32(0x1cb5c415); // Vector<InputUser>
        w.write_i32(1);
        w.write_u32(0xf7c1b13f); // inputUserSelf#f7c1b13f
        client.invoke_raw(w.into_bytes()).await
    })
    .await?;
    println!("users.getUsers answered with {} bytes", own.len());

    // 2. Ordered RPCs: `invokeAfterMsg` holds the second call until the
    //    server has processed the first one — use it when two calls must
    //    not race (e.g. create-then-configure).
    let first = client.send("@lebenoa", "step 1 (ordered)").await?;
    let _second = client
        .message("@lebenoa", "step 2 (after step 1 was processed)")
        .await
        .after_msg(first)
        .send()
        .await?;
    println!("ordered pair sent ({} -> ...)", first.0);

    // 3. Suppressed updates: the reply to this call won't be pushed to
    //    the update stream — useful for fire-and-forget logging calls.
    let _ = client
        .message("@lebenoa", "this send had no update push")
        .await
        .without_updates()
        .send()
        .await?;

    // 4. Low-level service queries straight through the pool.
    let pool = client.pool();
    let (req_id, server_now, windows) = pool.get_future_salts(3).await?;
    println!(
        "future_salts for msg {req_id}: server_now={server_now}, {} salt window(s)",
        windows.len()
    );

    // 5. Typed invoke with the generic `TlResult` decode path + typed
    //    error introspection.
    let state = with_retries(|| client.get_state()).await?;
    println!(
        "update state: pts={} qts={} seq={} unread={}",
        state.pts, state.qts, state.seq, state.unread_count
    );

    // 6. Non-transient errors surface directly — no retry can fix them.
    match client.send("@definitely_not_a_real_peer_xyz", "hi").await {
        Ok(_) => unreachable!(),
        Err(
            e @ Error::Rpc {
                error_code: 400, ..
            },
        ) => {
            println!("expected 400 for a bad peer: {e}");
            println!("  is_transient = {}", e.is_transient());
        }
        Err(e) => return Err(e.into()),
    }

    Ok(())
}
