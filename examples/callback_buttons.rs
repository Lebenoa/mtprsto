//! Press inline keyboard buttons on a bot's messages (user-session flow).
//!
//! Bots cannot press buttons — `messages.getBotCallbackAnswer` is a
//! **user** RPC. This example therefore expects an authorized user
//! session. Create one first with the demo's interactive phone login:
//!
//! ```sh
//! TELEGRAM_API_ID=12345 TELEGRAM_API_HASH=abcdef... \
//!   cargo run --example demo -- --user-phone +15551234567
//! ```
//!
//! The demo writes its session to `%TEMP%/mtprsto_demo_session.json`.
//! Then open any bot with inline buttons (e.g. @BotFather) and run:
//!
//! ```sh
//! cargo run --example callback_buttons -- "$TMP/mtprsto_demo_session.json" @BotFather
//! ```
//!
//! The example reads the bot's recent messages, lists the inline buttons
//! found on the newest one, and presses the first callback button.

use mtprsto::types::{IncomingReplyMarkup, KeyboardButtonKind};
use mtprsto::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut args = std::env::args().skip(1);
    let session_path = args.next().expect("usage: callback_buttons <SESSION> <BOT>");
    let bot = args.next().expect("missing <BOT>");

    // A user-authorized session file (see prerequisite above).
    let mut client = Client::builder().session(&session_path).build()?;
    client.connect().await?;
    println!("connected with user session");

    // Read the newest messages from the bot chat.
    let messages = client.messages(&bot).await.page_size(10).collect(10).await?;
    println!("fetched {} message(s) from {bot}", messages.len());

    // Find the newest message with an inline keyboard.
    for msg in &messages {
        let Some(markup) = &msg.reply_markup else { continue };
        let IncomingReplyMarkup::Inline { rows } = markup else {
            continue; // reply keyboards can't be "pressed"
        };

        println!("found inline keyboard on message {}", msg.id.0);
        let mut first_callback: Option<(String, Vec<u8>)> = None;
        for (y, row) in rows.iter().enumerate() {
            for (x, btn) in row.buttons.iter().enumerate() {
                match btn {
                    KeyboardButtonKind::Callback { text, data } => {
                        println!("  [{x},{y}] callback \"{text}\" ({} bytes)", data.len());
                        if first_callback.is_none() {
                            first_callback = Some((text.clone(), data.clone()));
                        }
                    }
                    KeyboardButtonKind::Url { text, url } => {
                        println!("  [{x},{y}] url \"{text}\" -> {url}");
                    }
                    KeyboardButtonKind::Text { text } => {
                        println!("  [{x},{y}] \"{text}\"");
                    }
                    other => println!("  [{x},{y}] {other:?}"),
                }
            }
        }

        // Press the first callback button (user-side action).
        if let Some((text, data)) = first_callback {
            println!("pressing \"{text}\"...");
            let answer = client
                .get_bot_callback_answer(&bot, msg.id, &data)
                .await?;
            println!(
                "bot answered: alert={} message={:?} url={:?}",
                answer.alert, answer.message, answer.url
            );
        } else {
            println!("no callback buttons on this message");
        }
        return Ok(());
    }
    println!("no inline keyboards found in the last {} messages", messages.len());
    Ok(())
}
