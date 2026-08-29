//! Offline protocol demo and live authorization smoke tests.
//!
//! Usage:
//!   cargo run --example demo -- --demo            (offline crypto checks, no network)
//!   cargo run --example demo -- --bot-token <TOKEN>
//!   cargo run --example demo -- --user-phone <PHONE>
//!
//! Live modes need TELEGRAM_API_ID and TELEGRAM_API_HASH from
//! https://my.telegram.org.

use mtprsto::api::TelegramClient;
use mtprsto::crypto;
use mtprsto::mtproto::MtProtoSession;
use num_bigint::BigUint;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "--demo" => run_demo(),
        "--bot-token" => {
            if args.len() < 3 {
                eprintln!("Error: --bot-token requires a token argument");
                std::process::exit(1);
            }
            let token = &args[2];
            bot_auth(token).await?;
        }
        "--user-phone" => {
            if args.len() < 3 {
                eprintln!("Error: --user-phone requires a phone number");
                std::process::exit(1);
            }
            let phone = &args[2];
            user_auth(phone).await?;
        }
        "--help" | "-h" => print_usage(),
        _ => {
            eprintln!("Unknown argument: {}", args[1]);
            print_usage();
            std::process::exit(1);
        }
    }

    Ok(())
}

fn print_usage() {
    println!("Telegram MTProto 2.0 demo");
    println!();
    println!("Usage:");
    println!("  cargo run --example demo -- --demo              Offline crypto tests");
    println!("  cargo run --example demo -- --bot-token <TOKEN>  Authorize as a bot");
    println!("  cargo run --example demo -- --user-phone <PHONE> Authorize as a user");
    println!();
    println!("Environment variables:");
    println!("  TELEGRAM_API_ID      API ID from https://my.telegram.org");
    println!("  TELEGRAM_API_HASH    API Hash from https://my.telegram.org");
}
fn run_demo() {
    println!("=== MTProto 2.0 Offline Demo ===\n");

    // 1. Show crypto primitives
    println!("1. Crypto Primitives:");
    let data = b"Hello, MTProto 2.0!";
    let hash256 = crypto::sha256(data);
    println!("   SHA-256:  {}", hex_str(&hash256));
    let hash1 = crypto::sha1(data);
    println!("   SHA-1:    {}", hex_str(&hash1));
    let hash_md5 = crypto::md5(data);
    println!("   MD5:      {}", hex_str(&hash_md5));
    let crc = crypto::crc32(b"vector t:Type # [ t ] = Vector t");
    println!("   CRC32:    {:#010x} (expected 0x1cb5c415)", crc);

    // 2. AES-256-IGE round-trip
    println!("\n2. AES-256-IGE:");
    let key = [0x42u8; 32];
    let iv = [0x24u8; 32];
    let mut plaintext = b"This is a test message for AES-IGE encryption.".to_vec();
    let pad = 16 - (plaintext.len() % 16);
    if pad != 16 {
        plaintext.resize(plaintext.len() + pad, 0);
    }
    let mut ciphertext = plaintext.clone();
    crypto::aes_ige_encrypt(&mut ciphertext, &key, &iv).unwrap();
    println!("   Plaintext:  {}", String::from_utf8_lossy(&plaintext));
    println!("   Ciphertext: {}", hex_str(&ciphertext));
    crypto::aes_ige_decrypt(&mut ciphertext, &key, &iv).unwrap();
    assert_eq!(ciphertext, plaintext);
    println!("   Decrypted:  {}", String::from_utf8_lossy(&ciphertext));
    println!("   ✓ Round-trip successful");

    // 3. Diffie-Hellman
    println!("\n3. Diffie-Hellman Key Exchange:");
    let (g_a, a) = crypto::dh_client_generate();
    let (g_b, b) = crypto::dh_server_generate();
    let client_key = crypto::dh_client_complete(g_b.clone(), a.clone());
    let server_key = crypto::dh_server_complete(g_a.clone(), b);
    println!("   g_a ({} bits): {}...", g_a.bits(), &hex_str(&client_key)[..32]);
    println!("   g_b ({} bits): {}...", g_b.bits(), &hex_str(&server_key)[..32]);
    assert_eq!(client_key, server_key);
    println!("   ✓ Keys match!");

    // 4. DH parameter verification
    println!("\n4. DH Parameter Verification:");
    let p = crypto::dh_prime();
    let g = BigUint::from(2u32);
    let g_a_test = g.modpow(&a, &p);
    match crypto::verify_dh_params(2, &g_a_test, &p) {
        Ok(()) => println!("   ✓ DH parameters verified"),
        Err(e) => println!("   ✗ Verification failed: {e}"),
    }

    // 5. Server key fingerprints
    println!("\n5. Server Key Fingerprints:");
    for (i, key) in crypto::known_server_keys().iter().enumerate() {
        println!("   Key {}: {:#018x}", i + 1, key.fingerprint());
    }

    // 6. Auth key creation flow (offline, without network)
    println!("\n6. Auth Key Creation Flow:");
    let auth_key = crypto::random_bytes(256);
    let server_salt: u64 = 0x0102030405060708;
    let mut session = MtProtoSession::new(auth_key.clone(), server_salt);
    println!("   auth_key_id: {:#018x}", session.auth_key_id);
    println!("   session_id:  {:#018x}", session.session_id);
    println!("   server_salt: {:#018x}", session.server_salt);

    // 7. Message encryption/decryption
    println!("\n7. Message Encryption/Decryption:");
    let msg_id = session.next_msg_id();
    let seq_no = session.next_seq_no(true);
    let payload = b"Hello from MTProto!";
    let encrypted = session.encrypt_message(payload, msg_id, seq_no);
    println!("   Original:     {}", String::from_utf8_lossy(payload));
    println!("   Encrypted:    {} bytes", encrypted.len());
    let (dec_id, dec_payload) = session.decrypt_message_with_x(&encrypted, 0).unwrap();
    println!("   Decrypted:    {}", String::from_utf8_lossy(&dec_payload));
    assert_eq!(dec_payload, payload);
    assert_eq!(dec_id, msg_id);
    println!("   ✓ Round-trip successful");

    // 8. Auth key verification
    println!("\n8. Auth Key Verification:");
    let computed_id = crypto::auth_key_id(&auth_key);
    println!("   Expected: {:#018x}", computed_id);
    println!("   Got:      {:#018x}", session.auth_key_id);
    assert_eq!(computed_id, session.auth_key_id);
    println!("   ✓ Auth key ID matches");

    // 9. Session persistence
    println!("\n9. Session Persistence:");
    let session_path = std::env::temp_dir().join("mtprsto_demo_session.json");
    let data = mtprsto::session::SessionData::from_auth_key(&auth_key, server_salt, 2);
    let mut store = mtprsto::session::SessionStore::new(&session_path);
    store.save(&data).unwrap();
    println!("   ✓ Saved session to {}", session_path.display());

    let mut store2 = mtprsto::session::SessionStore::new(&session_path);
    let loaded = store2.load().unwrap().unwrap();
    let decoded_key = loaded.decode_auth_key().unwrap();
    assert_eq!(decoded_key, auth_key);
    println!("   ✓ Loaded and verified session from disk");

    // 10. Typed errors
    println!("\n10. Typed Errors:");
    let flood_err = mtprsto::error::Error::FloodWait {
        seconds: 30,
        retry_after: std::time::Instant::now(),
    };
    println!("   FloodWait: {} (transient={})", flood_err, flood_err.is_transient());
    let rpc_err = mtprsto::error::Error::Rpc {
        error_code: 400,
        error_message: "PEER_ID_INVALID".into(),
    };
    println!("   Rpc: {} (transient={})", rpc_err, rpc_err.is_transient());
    let migration_err = mtprsto::error::Error::Migration { dc_id: 2 };
    println!("   Migration: {} (dc_id={:?})", migration_err, migration_err.dc_id());
    println!("   ✓ Error types working correctly");

    // Cleanup
    std::fs::remove_file(&session_path).ok();

    println!("\n=== All offline tests passed! ===");
    println!("\nTo authorize with a real Telegram account:");
    println!("  1. Get API ID and API hash from https://my.telegram.org");
    println!("  2. Set environment variables:");
    println!("     export TELEGRAM_API_ID=your_api_id");
    println!("     export TELEGRAM_API_HASH=your_api_hash");
    println!("  3. For bot auth:");
    println!("     cargo run --example demo -- --bot-token 123456:ABC-DEF...");
    println!("  4. For user auth:");
    println!("     cargo run --example demo -- --user-phone +1234567890");
}

async fn bot_auth(token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let api_id = env::var("TELEGRAM_API_ID")
        .map(|v| v.parse::<i32>().expect("TELEGRAM_API_ID must be an integer"))
        .unwrap_or(0);
    let api_hash = env::var("TELEGRAM_API_HASH").unwrap_or_default();

    println!("=== Bot Authorization ===");
    println!("Connecting to Telegram DC2...");

    let mut client = TelegramClient::new(2, Some(api_id), Some(api_hash));

    // Step 1: Create auth key (DH handshake)
    println!("Creating authorization key (Diffie-Hellman)...");
    client.create_auth_key().await?;
    println!("✓ Authorization key created");

    // Step 2: Authorize with bot token
    println!("Authorizing with bot token...");
    client.authorize_bot(token).await?;
    println!("✓ Bot authorization successful!");

    println!("\nYou can now make API calls using the client.");
    Ok(())
}

async fn user_auth(phone: &str) -> Result<(), Box<dyn std::error::Error>> {
    let api_id = env::var("TELEGRAM_API_ID")
        .map(|v| v.parse::<i32>().expect("TELEGRAM_API_ID must be an integer"))
        .unwrap_or(0);
    let api_hash = env::var("TELEGRAM_API_HASH").unwrap_or_default();

    println!("=== User Authorization ===");
    println!("Connecting to Telegram DC2...");

    let mut client = TelegramClient::new(2, Some(api_id), Some(api_hash));

    // Step 1: Create auth key
    println!("Creating authorization key (Diffie-Hellman)...");
    client.create_auth_key().await?;
    println!("✓ Authorization key created");

    // Step 2: Send code
    println!("Sending verification code to {phone}...");
    let sent = client.auth_send_code(phone).await?;
    println!("✓ Code sent (type: {:?})", sent_code_name(sent.sent_code_type));

    // Step 3: Read code from stdin
    println!("Enter the verification code:");
    let mut code = String::new();
    std::io::stdin().read_line(&mut code)?;
    let code = code.trim();

    // Step 4: Sign in
    println!("Signing in...");
    match client.auth_sign_in(phone, &sent.phone_code_hash, code).await {
        Ok(()) => {
            println!("✓ Sign-in successful!");
        }
        Err(mtprsto::error::Error::Protocol(msg)) if msg.contains("Sign up required") => {
            println!("New account detected. Please enter your first and last name:");
            let mut first_name = String::new();
            let mut last_name = String::new();
            print!("First name: ");
            std::io::stdin().read_line(&mut first_name)?;
            print!("Last name: ");
            std::io::stdin().read_line(&mut last_name)?;

            client.auth_sign_up(
                phone,
                &sent.phone_code_hash,
                first_name.trim(),
                last_name.trim(),
            ).await?;
            println!("✓ Sign-up successful!");
        }
        Err(e) => return Err(e.into()),
    }

    println!("\nYou are now authenticated. You can make API calls.");
    Ok(())
}

fn sent_code_name(code_type: u32) -> &'static str {
    match code_type {
        0x3dbb5986 => "SMS (App)",
        0xc004bac7 => "SMS",
        0x5353e5a7 => "Call",
        0xab036752 => "Flash Call",
        0x741cd2ee => "SMS or Call",
        _ => "Unknown",
    }
}

fn hex_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
