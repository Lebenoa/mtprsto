//! ReplyMarkup and KeyboardButton types.

use super::*;
use crate::serialize::TLWriter;
#[allow(unused_imports)]
use std::fmt;

// §7 Reply markup types
// ===========================================================================

/// Reply markup for messages (inline keyboards, reply keyboards, etc.).
#[derive(Debug, Clone)]
pub enum ReplyMarkup {
    /// No special markup.
    None,
    /// Force reply (forces the client to show a reply UI).
    ForceReply { selective: bool },
    /// Inline keyboard buttons.
    InlineKeyboard { rows: Vec<Vec<KeyboardButton>> },
    /// Reply keyboard (shown above the input field).
    ReplyKeyboard {
        rows: Vec<Vec<KeyboardButton>>,
        resize: bool,
        single_use: bool,
        selective: bool,
        persistent: bool,
    },
}

impl ReplyMarkup {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            ReplyMarkup::None => {}
            ReplyMarkup::ForceReply { selective } => {
                let flags: i32 = if *selective { 1 << 2 } else { 0 };
                w.write_u32(FORCE_REPLY);
                w.write_i32(flags);
            }
            ReplyMarkup::InlineKeyboard { rows } => {
                w.write_u32(inline_keyboard_markup::CONSTRUCTOR_ID);
                w.write_u32(VECTOR);
                w.write_i32(rows.len() as i32);
                for row in rows {
                    w.write_u32(VECTOR);
                    w.write_i32(row.len() as i32);
                    for btn in row {
                        btn.write_to(w);
                    }
                }
            }
            ReplyMarkup::ReplyKeyboard { rows, resize, single_use, selective, persistent } => {
                let mut flags: i32 = 0;
                if *resize { flags |= 1 << 0; }
                if *single_use { flags |= 1 << 1; }
                if *selective { flags |= 1 << 2; }
                if *persistent { flags |= 1 << 4; }
                w.write_u32(REPLY_KEYBOARD_MARKUP);
                w.write_i32(flags);
                w.write_u32(VECTOR);
                w.write_i32(rows.len() as i32);
                for row in rows {
                    w.write_u32(VECTOR);
                    w.write_i32(row.len() as i32);
                    for btn in row {
                        btn.write_to(w);
                    }
                }
            }
        }
    }
}

/// A keyboard button.
#[derive(Debug, Clone)]
pub enum KeyboardButton {
    Text { text: String },
    Url { text: String, url: String },
    Callback { text: String, data: Vec<u8> },
    // Simplified — full surface has ~15 variants
}

impl KeyboardButton {
    pub fn write_to(&self, w: &mut TLWriter) {
        match self {
            KeyboardButton::Text { text } => {
                w.write_u32(KEYBOARD_BUTTON);
                w.write_bytes(text.as_bytes());
            }
            KeyboardButton::Url { text, url } => {
                w.write_u32(KEYBOARD_BUTTON_URL);
                w.write_bytes(text.as_bytes());
                w.write_bytes(url.as_bytes());
            }
            KeyboardButton::Callback { text, data } => {
                w.write_u32(KEYBOARD_BUTTON_CALLBACK);
                w.write_bytes(text.as_bytes());
                w.write_bytes(data);
            }
        }
    }
}

// ===========================================================================
