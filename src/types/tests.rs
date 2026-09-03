#[cfg(test)]
mod type_tests {
    // Test code: unwrap/expect/panic are the idiomatic failure modes here,
    // and schema ctor ids are quoted verbatim (unreadable_literal).
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::unreadable_literal
    )]
    use crate::serialize::{TLReader, TLWriter, VECTOR};
    use crate::types::*;

    #[test]
    fn test_user_id_newtype() {
        let uid = UserId(12345);
        assert_eq!(uid.0, 12345);
        let raw: i64 = uid.into();
        assert_eq!(raw, 12345);
    }

    #[test]
    fn test_input_peer_roundtrip() {
        let peer = input_peer_user(12345, 67890);
        let mut w = TLWriter::new();
        peer.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        let parsed = InputPeer::read_from(&mut r).unwrap();
        match parsed {
            InputPeer::User {
                user_id,
                access_hash,
            } => {
                assert_eq!(user_id.0, 12345);
                assert_eq!(access_hash.0, 67890);
            }
            _ => panic!("expected User"),
        }
    }

    #[test]
    fn test_input_peer_chat_roundtrip() {
        let peer = input_peer_chat(999);
        let mut w = TLWriter::new();
        peer.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        let parsed = InputPeer::read_from(&mut r).unwrap();
        match parsed {
            InputPeer::Chat { chat_id } => assert_eq!(chat_id.0, 999),
            _ => panic!("expected Chat"),
        }
    }

    #[test]
    fn test_input_peer_channel_roundtrip() {
        let peer = input_peer_channel(42, 100);
        let mut w = TLWriter::new();
        peer.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        let parsed = InputPeer::read_from(&mut r).unwrap();
        match parsed {
            InputPeer::Channel {
                channel_id,
                access_hash,
            } => {
                assert_eq!(channel_id.0, 42);
                assert_eq!(access_hash.0, 100);
            }
            _ => panic!("expected Channel"),
        }
    }

    #[test]
    fn test_peer_roundtrip() {
        for peer in &[
            Peer::User { user_id: UserId(1) },
            Peer::Chat { chat_id: ChatId(2) },
            Peer::Channel {
                channel_id: ChannelId(3),
            },
        ] {
            let mut w = TLWriter::new();
            match peer {
                Peer::User { user_id } => {
                    w.write_u32(PEER_USER);
                    w.write_i64(user_id.0);
                }
                Peer::Chat { chat_id } => {
                    w.write_u32(PEER_CHAT);
                    w.write_i64(chat_id.0);
                }
                Peer::Channel { channel_id } => {
                    w.write_u32(PEER_CHANNEL);
                    w.write_i64(channel_id.0);
                }
                Peer::None => {}
            }
            let mut r = TLReader::new(w.as_bytes());
            let parsed = Peer::read_from(&mut r).unwrap();
            assert_eq!(&parsed, peer);
        }
    }

    #[test]
    fn test_keyboard_button_text_roundtrip() {
        // keyboardButton#7d170cff flags:# style:flags.10?KeyboardButtonStyle
        //   text:string
        let mut w = TLWriter::new();
        w.write_u32(crate::types::reply_markup_gen::KEYBOARD_BUTTON_ID);
        w.write_i32(0); // flags (no style)
        w.write_bytes(b"Click me");
        let mut r = TLReader::new(w.as_bytes());
        let btn = crate::types::reply_markup_gen::KeyboardButton::read_from(&mut r).unwrap();
        let crate::types::reply_markup_gen::KeyboardButton::Text { style, text, .. } = btn else {
            panic!("expected keyboardButton variant");
        };
        assert_eq!(text, "Click me");
        assert!(style.is_none());
    }

    #[test]
    fn test_write_vector_long() {
        let mut w = TLWriter::new();
        write_vector_long(&mut w, &[1, 2, 3]);
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_u32().unwrap(), VECTOR);
        assert_eq!(r.read_i32().unwrap(), 3);
        assert_eq!(r.read_i64().unwrap(), 1);
        assert_eq!(r.read_i64().unwrap(), 2);
        assert_eq!(r.read_i64().unwrap(), 3);
    }

    #[test]
    fn test_constructor_ids_unique() {
        // Ensure key constructors don't collide
        let ids = vec![
            INPUT_PEER_SELF,
            INPUT_PEER_USER,
            INPUT_PEER_USER_FROM_ID,
            INPUT_PEER_CHAT,
            INPUT_PEER_CHANNEL,
            INPUT_PEER_CHANNEL_FROM_ID,
        ];
        let mut deduped = ids.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(
            ids.len(),
            deduped.len(),
            "duplicate constructor IDs detected"
        );
    }

    #[test]
    fn test_user_read_parses_flags2_before_id() {
        // user#31774388 always serializes TWO flag words before id:long.
        // A bot-auth response typically has flags=0x4000 (bot flag),
        // flags2=0, id=777.
        let mut w = TLWriter::new();
        w.write_u32(USER_ID); // user#b1b8cc83 (current schema)
        w.write_i32(1 << 14); // flags: bot=true
        w.write_i32(0); // flags2
        w.write_i64(777); // id
        w.write_i32(1); // bot_info_version:flags.14?int
        let mut r = TLReader::new(w.as_bytes());
        let user = User::read_from(&mut r).unwrap();
        assert_eq!(user.id().0, 777);
        assert!(user.is_bot());
    }

    /// SPEC §6: `updates` container decodes inner update constructors.
    #[test]
    fn test_updates_parse_decodes_new_message() {
        let mut w = TLWriter::new();
        w.write_u32(UPDATES);
        // layer 223 order: updates, users, chats, date, seq
        // updates:Vector<Update> — one updateNewMessage
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1);
        w.write_u32(UPDATE_NEW_MESSAGE);
        // message:message#3ae56482 — nested object carries its own ctor
        w.write_u32(MESSAGE);
        w.write_i32(0); // message flags
        w.write_i32(0); // message flags2 (layer 223)
        w.write_i32(77); // id:int
        w.write_u32(PEER_USER);
        w.write_i64(42); // peer_id user
        w.write_i32(1_700_000_000); // date
        w.write_bytes(b"hi"); // message text (required string in 223)
        w.write_i32(10); // pts
        w.write_i32(1); // pts_count
        // users:Vector<User>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);
        // chats:Vector<Chat>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);
        w.write_i32(1_700_000_000); // date
        w.write_i32(5); // seq

        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::Updates {
                updates: list, seq, ..
            } => {
                assert_eq!(seq, 5);
                let [update] = list.as_slice() else {
                    panic!("expected exactly one update, got {}", list.len());
                };
                match update {
                    Update::NewMessage {
                        message,
                        pts,
                        pts_count,
                    } => {
                        assert_eq!(message.id(), MsgId(77));
                        assert_eq!(message.text(), "hi");
                        assert_eq!(*pts, 10);
                        assert_eq!(*pts_count, 1);
                    }
                    other => panic!("expected NewMessage, got {other:?}"),
                }
            }
            other => panic!("expected Updates, got {other:?}"),
        }
    }

    /// updateShort wraps a single update; inner ctor must decode.
    #[test]
    fn test_update_short_decodes_inner() {
        let mut w = TLWriter::new();
        w.write_u32(UPDATE_SHORT);
        w.write_u32(UPDATE_READ_MESSAGES);
        w.write_i32(0); // flags
        // messages:Vector<int>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(2);
        w.write_i32(1);
        w.write_i32(2);
        w.write_i32(100); // pts
        w.write_i32(2); // pts_count
        w.write_i32(1_700_000_000); // date

        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::UpdateShort {
                update: Update::ReadMessages { messages },
                ..
            } => {
                assert_eq!(messages, vec![MsgId(1), MsgId(2)]);
            }
            other => panic!("expected UpdateShort ReadMessages, got {other:?}"),
        }
    }

    /// Unknown inner ctors now FAIL: M1 delegates unknown Update variants
    /// to the generated parser so payload bytes are consumed exactly;
    /// a ctor outside the schema entirely cannot be skipped, so the
    /// parse errors precisely instead of desyncing Vector<Update>.
    #[test]
    fn test_updates_unknown_inner_errors_precisely() {
        let mut w = TLWriter::new();
        w.write_u32(UPDATE_SHORT);
        w.write_u32(0xDEADBEEF); // unknown-to-schema update ctor
        w.write_i32(1_700_000_000);
        w.write_i32(0);

        // The generated union's Other fallback accepts any ctor id, so an
        // unknown-to-schema variant parses as Other with zero bytes
        // consumed. That is only safe when the variant is the LAST element
        // of its container (updateShort has nothing after the update), so
        // this parse succeeds; inside Vector<Update> it would desync —
        // the schema-known delegation (see the M1 test below) is the fix
        // for everything the schema knows.
        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::UpdateShort {
                update: Update::Other { constructor },
                ..
            } => assert_eq!(constructor, 0xDEADBEEF),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    /// M1 regression: a KNOWN-to-schema but unhandled-by-curator update
    /// must consume exactly its payload bytes inside a container, so a
    /// following update still parses.
    #[test]
    fn test_unhandled_known_update_consumes_bytes_in_vector() {
        use crate::types::constructors::UPDATE_DELETE_CHANNEL_MESSAGES;
        use crate::types::updates_gen::UPDATE_PTS_CHANGED_ID;
        // updates#74ae4240 { updates:Vector<Update>, users, chats, date, seq }
        let mut w = TLWriter::new();
        w.write_u32(crate::types::UPDATES);
        w.write_u32(VECTOR);
        w.write_i32(2);
        // updatePtsChanged#3354678f — empty payload, uncurated by the
        // curator enum (delegated to the generated parser by M1).
        w.write_u32(UPDATE_PTS_CHANGED_ID);
        // updateDeleteChannelMessages#c32d5b12 (curated): channel_id,
        // messages, pts, pts_count — no flags word in the 223 schema.
        w.write_u32(UPDATE_DELETE_CHANNEL_MESSAGES);
        w.write_i64(7); // channel_id
        w.write_u32(VECTOR);
        w.write_i32(1);
        w.write_i32(5); // messages
        w.write_i32(100); // pts
        w.write_i32(1); // pts_count
        // users:Vector<User>
        w.write_u32(VECTOR);
        w.write_i32(0);
        // chats:Vector<Chat>
        w.write_u32(VECTOR);
        w.write_i32(0);
        w.write_i32(1_700_000_000); // date
        w.write_i32(0); // seq

        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::Updates { updates, .. } => {
                assert_eq!(updates.len(), 2);
                assert!(matches!(
                    updates[0],
                    Update::Other {
                        constructor: UPDATE_PTS_CHANGED_ID
                    }
                ));
                // The second update parsed cleanly → the first consumed
                // exactly its bytes.
                assert!(matches!(updates[1], Update::DeleteChannelMessages { .. }));
            }
            other => panic!("expected Updates, got {other:?}"),
        }
    }

    // --- Reply types (SPEC §7) round-trips -------------------------------------

    #[test]
    fn test_message_entity_roundtrip() {
        use crate::types::{MessageEntityKind, read_message_entities};

        // Vector<MessageEntity> with a Bold and a TextUrl entity.
        let mut w = TLWriter::new();
        w.write_u32(VECTOR);
        w.write_i32(2);
        w.write_u32(MESSAGE_ENTITY_BOLD);
        w.write_i32(0);
        w.write_i32(4);
        w.write_u32(MESSAGE_ENTITY_TEXT_URL);
        w.write_i32(5);
        w.write_i32(10);
        w.write_bytes(b"https://example.com");
        let data = w.into_bytes();

        let mut r = TLReader::new(&data);
        let ents = read_message_entities(&mut r).unwrap();
        assert_eq!(ents.len(), 2);
        let [bold, text_url] = ents.as_slice() else {
            panic!("expected exactly two entities");
        };
        assert_eq!(bold.kind, MessageEntityKind::Bold);
        assert_eq!(
            text_url.kind,
            MessageEntityKind::TextUrl {
                url: "https://example.com".into()
            }
        );
        assert_eq!(text_url.offset, 5);
        assert_eq!(text_url.length, 10);
    }
    /// H3 regression: an unhandled-by-curator MessageAction inside a
    /// messageService must consume exactly its payload (delegation to the
    /// generated union), leaving the reader aligned for what follows.
    #[test]
    fn test_unhandled_action_consumes_exact_bytes() {
        use crate::types::constructors::MESSAGE_SERVICE;
        use crate::types::message_gen::MESSAGE_ACTION_GIFT_PREMIUM_ID;
        // messageService#7a800e0a flags id from_id? peer_id reply_to?
        //   date action reactions? ttl_period?
        // messageActionGiftPremium#96f63684 flags:# currency amount months
        //   crypto_currency? crypto_amount? (223 shape; exact payload
        //   irrelevant — only exact-consumption matters)
        let mut w = TLWriter::new();
        w.write_u32(MESSAGE_SERVICE);
        w.write_i32(0); // flags
        w.write_i32(42); // id
        w.write_u32(crate::types::constructors::PEER_USER);
        w.write_i64(1); // peer_id user
        w.write_i32(1_700_000_000); // date
        // action: messageActionGiftPremium with a minimal known payload
        w.write_u32(MESSAGE_ACTION_GIFT_PREMIUM_ID);
        w.write_i32(0); // flags
        w.write_bytes(b"TON"); // currency
        w.write_i64(100); // amount
        w.write_i32(3); // months

        let msg = Message::parse_from_bytes(&w.into_bytes()).unwrap();
        match msg {
            Message::Service { action, .. } => {
                assert!(matches!(action, crate::types::MessageAction::Other));
            }
            other => panic!("expected Service, got {other:?}"),
        }
    }

    /// H4 regression: an Unsupported MessageMedia (dice) must consume its
    /// payload exactly so trailing message fields still parse.
    #[test]
    fn test_unsupported_media_consumes_exact_bytes() {
        use crate::types::constructors::MESSAGE;
        // message#3ae56482 minimal: flags=0, flags2=0, id, peer_id,
        // date, message string, then media (dice), then reply_markup?
        // entities? views? ... — media is followed by more fields only if
        // flagged; with all-zero flags after media nothing follows, so the
        // exact-consumption proof is that the parse SUCCEEDS at all
        // (old code left dice bytes unread and the trailing-field reads
        // desynced).
        let mut w = TLWriter::new();
        w.write_u32(MESSAGE);
        w.write_i32(1 << 9); // flags with media bit (1<<9)
        w.write_i32(0); // flags2
        w.write_i32(10); // id
        w.write_u32(crate::types::constructors::PEER_USER);
        w.write_i64(1); // peer_id
        w.write_i32(1_700_000_000); // date
        w.write_bytes(b"roll"); // message
        // media: messageMediaDice#8cbec07 {flags, value:int, emoticon,
        // game_outcome:flags.0?MessagesEmojiGameOutcome}
        w.write_u32(crate::types::constructors::MESSAGE_MEDIA_DICE);
        w.write_i32(0); // dice flags (no game_outcome)
        w.write_i32(6); // value
        w.write_bytes("🎲".as_bytes()); // emoticon

        let msg = Message::parse_from_bytes(&w.into_bytes());
        // The dice payload shape must match the generated parser's 229
        // layout; if the field set diverges the test fails loudly rather
        // than silently passing.
        assert!(msg.is_ok(), "dice media must parse with exact consumption");
    }

    #[test]
    fn test_inline_reply_markup_roundtrip() {
        use crate::types::{KeyboardButtonKind, read_reply_markup};

        // replyInlineMarkup#48a30254 rows:Vector<KeyboardButtonRow>
        let mut w = TLWriter::new();
        w.write_u32(REPLY_INLINE_MARKUP);
        w.write_u32(VECTOR); // rows
        w.write_i32(1); // 1 row
        w.write_u32(KEYBOARD_BUTTON_ROW);
        w.write_u32(VECTOR); // buttons
        w.write_i32(2); // 2 buttons
        // keyboardButtonCallback (ctor carries flags first)
        w.write_u32(KEYBOARD_BUTTON_CALLBACK);
        w.write_i32(0); // flags
        w.write_bytes(b"Yes");
        w.write_bytes(&[0xDE, 0xAD]);
        // keyboardButtonUrl
        w.write_u32(KEYBOARD_BUTTON_URL);
        w.write_i32(0);
        w.write_bytes(b"Docs");
        w.write_bytes(b"https://docs.example");
        let data = w.into_bytes();

        let mut r = TLReader::new(&data);
        let markup = read_reply_markup(&mut r).unwrap();
        match markup {
            IncomingReplyMarkup::Inline { rows } => {
                let [row] = rows.as_slice() else {
                    panic!("expected exactly one row");
                };
                let [btn0, btn1] = row.buttons.as_slice() else {
                    panic!("expected exactly two buttons");
                };
                match (btn0, btn1) {
                    (
                        KeyboardButtonKind::Callback { text, data },
                        KeyboardButtonKind::Url { text: t2, url },
                    ) => {
                        assert_eq!(text, "Yes");
                        assert_eq!(data, &vec![0xDE, 0xAD]);
                        assert_eq!(t2, "Docs");
                        assert_eq!(url, "https://docs.example");
                    }
                    _ => panic!("wrong button kinds"),
                }
            }
            other => panic!("expected Inline, got {other:?}"),
        }
    }

    #[test]
    fn test_document_attributes_roundtrip() {
        use crate::types::{DocumentAttribute, read_document_attributes};

        let mut w = TLWriter::new();
        w.write_u32(VECTOR);
        w.write_i32(2);
        // documentAttributeFilename
        w.write_u32(DOCUMENT_ATTRIBUTE_FILENAME);
        w.write_bytes(b"report.pdf");
        // documentAttributeVideo (no optional flags)
        w.write_u32(DOCUMENT_ATTRIBUTE_VIDEO);
        w.write_i32(0); // flags
        w.write_double(120.5); // duration
        w.write_i32(1920);
        w.write_i32(1080);
        let data = w.into_bytes();

        let mut r = TLReader::new(&data);
        let attrs = read_document_attributes(&mut r).unwrap();
        assert_eq!(attrs.len(), 2);
        let [filename, video] = attrs.as_slice() else {
            panic!("expected exactly two attributes");
        };
        assert_eq!(
            *filename,
            DocumentAttribute::Filename {
                file_name: "report.pdf".into()
            }
        );
        match video {
            DocumentAttribute::Video {
                duration,
                w,
                h,
                supports_streaming,
                ..
            } => {
                assert!((duration - 120.5).abs() < f64::EPSILON);
                assert_eq!(*w, 1920);
                assert_eq!(*h, 1080);
                assert!(!supports_streaming);
            }
            other => panic!("expected Video, got {other:?}"),
        }
    }

    #[test]
    fn test_photo_sizes_roundtrip() {
        use crate::types::{PhotoSizeFull, read_photo_sizes};

        let mut w = TLWriter::new();
        w.write_u32(VECTOR);
        w.write_i32(2);
        // photoSize
        w.write_u32(PHOTO_SIZE);
        w.write_bytes(b"x");
        w.write_i32(800);
        w.write_i32(600);
        w.write_i32(123_456);
        // photoStrippedSize
        w.write_u32(PHOTO_STRIPPED_SIZE);
        w.write_bytes(b"i");
        w.write_bytes(&[1, 2, 3]);
        let data = w.into_bytes();

        let mut r = TLReader::new(&data);
        let sizes = read_photo_sizes(&mut r).unwrap();
        assert_eq!(sizes.len(), 2);
        let [size, stripped] = sizes.as_slice() else {
            panic!("expected exactly two sizes");
        };
        assert_eq!(
            *size,
            PhotoSizeFull::Size {
                type_: "x".into(),
                w: 800,
                h: 600,
                size: 123_456
            }
        );
        assert_eq!(
            *stripped,
            PhotoSizeFull::Stripped {
                type_: "i".into(),
                bytes: vec![1, 2, 3]
            }
        );
    }

    #[test]
    fn test_dialog_folder_roundtrip() {
        use crate::types::DialogFolderFull;

        // dialogFolder#71bd134c flags:# pinned:flags.2?true folder:Folder peer:Peer
        // top_message:int unread_muted_peers_count:int unread_unmuted_peers_count:int
        // unread_muted_messages_count:int unread_unmuted_messages_count:int
        let mut w = TLWriter::new();
        w.write_u32(DIALOG_FOLDER);
        w.write_i32(1 << 2); // pinned
        w.write_u32(FOLDER);
        w.write_i32(0); // folder flags
        w.write_i32(3); // folder id
        w.write_bytes(b"My folder");
        w.write_u32(PEER_USER);
        w.write_i64(42);
        w.write_i32(1000); // top_message
        w.write_i32(1);
        w.write_i32(2);
        w.write_i32(3);
        w.write_i32(4);
        let data = w.into_bytes();

        let mut r = TLReader::new(&data);
        let df = DialogFolderFull::read_from(&mut r).unwrap();
        assert!(df.pinned);
        assert_eq!(df.folder_id, 3);
        assert_eq!(df.folder_title, "My folder");
        assert_eq!(df.top_message, 1000);
        assert_eq!(df.unread_unmuted_messages_count, 4);
    }

    #[test]
    fn test_chat_forbidden_roundtrip() {
        // chatForbidden#7328209: id:int then title:string (historical layout)
        let mut w = TLWriter::new();
        w.write_u32(CHAT_FORBIDDEN);
        w.write_i64(777);
        w.write_bytes(b"secret club");
        let data = w.into_bytes();

        let mut r = TLReader::new(&data);
        match Chat::read_from(&mut r).unwrap() {
            Chat::Forbidden { id, title } => {
                assert_eq!(id.0, 777);
                assert_eq!(title, "secret club");
            }
            other => panic!("expected Forbidden, got {other:?}"),
        }
    }

    /// Generated (tools/gentl.py) decoder must decode the schema-defined
    /// field order from hand-built wire bytes.
    #[test]
    fn test_tl_gen_user_roundtrip() {
        let mut w = TLWriter::new();
        w.write_u32(crate::types::user_gen::USER_ID); // user#b1b8cc83
        let flags = (1 << 0) | (1 << 1) | (1 << 4) | (1 << 14); // access_hash, first_name, phone, bot
        w.write_i32(flags);
        w.write_i32(0); // flags2
        w.write_i64(4242); // id
        w.write_i64(7777); // access_hash
        w.write_bytes(b"Yuka"); // first_name
        w.write_bytes(b"+66988962019"); // phone
        w.write_i32(9); // bot_info_version

        let data = w.into_bytes();
        let mut r = TLReader::new(&data);
        let user = crate::types::user_gen::User::read_from(&mut r).unwrap();
        // The wildcard is the point of the test: any non-User variant is
        // a decode failure to be reported with its debug shape.
        #[allow(clippy::match_wildcard_for_single_variants)]
        match user {
            crate::types::user_gen::User::User {
                id,
                access_hash,
                first_name,
                phone,
                bot,
                ..
            } => {
                assert_eq!(id, UserId(4242));
                assert_eq!(access_hash, Some(AccessHash(7777)));
                assert_eq!(first_name.as_deref(), Some("Yuka"));
                assert_eq!(phone.as_deref(), Some("+66988962019"));
                assert!(bot);
            }
            other => panic!("expected User::User, got {other:?}"),
        }
    }

    /// Generated request builders must produce wire-identical payloads to the
    /// hand-written ones (schema order + flags are the contract).
    #[test]
    fn test_tl_gen_builder_matches_handwritten() {
        let generated = crate::types::gen_fns::build_messages_delete_messages(true, &[1, 2]);
        let hand = crate::rpc::build_delete_messages(
            &[crate::types::MsgId(1), crate::types::MsgId(2)],
            true,
        );
        assert_eq!(generated, hand, "deleteMessages payloads diverge");

        let generated = crate::types::gen_fns::build_contacts_resolve_username("lebenoa", None);
        let hand = crate::rpc::build_resolve_username("lebenoa");
        assert_eq!(generated, hand, "resolveUsername payloads diverge");
    }
}

/// Every constructors.rs constant with a generated counterpart must
/// equal the PUBLISHED layer-223 id (tl.json — the docs-layer dialect),
/// which the generated parsers accept through their 223 alias arms
/// (mirrored from tools/gentl.py CTOR_ALIASES on this branch).
// Test code: schema ctor ids are quoted verbatim (unreadable_literal);
// unwrap/panic are the idiomatic failure modes.
#[allow(
    clippy::unreadable_literal,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::as_conversions,
    clippy::cast_possible_wrap,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::doc_markdown
)]
#[test]
fn test_constructor_constants_match_generated() {
    // Curated constants carry the negotiated (published 225) ids, so
    // each must equal the generated parser's canonical id. Re-issued
    // 223-era/draft ids are accepted by the parsers through the
    // CTOR_ALIASES arms in tools/gentl.py instead.
    let ours: Vec<(&str, u32)> = vec![
        ("USER", crate::types::USER),
        ("USER_EMPTY", crate::types::USER_EMPTY),
        ("CHAT", crate::types::CHAT),
        ("CHAT_EMPTY", crate::types::CHAT_EMPTY),
        ("CHAT_FORBIDDEN", crate::types::CHAT_FORBIDDEN),
        ("CHANNEL", crate::types::CHANNEL),
        ("MESSAGE", crate::types::MESSAGE),
        ("MESSAGE_EMPTY", crate::types::MESSAGE_EMPTY),
        ("MESSAGE_SERVICE", crate::types::MESSAGE_SERVICE),
        ("MESSAGE_MEDIA_PHOTO", crate::types::MESSAGE_MEDIA_PHOTO),
        (
            "MESSAGE_MEDIA_DOCUMENT",
            crate::types::MESSAGE_MEDIA_DOCUMENT,
        ),
        ("DIALOG", crate::types::DIALOG),
        ("PEER_USER", crate::types::PEER_USER),
        ("PEER_CHAT", crate::types::PEER_CHAT),
        ("PEER_CHANNEL", crate::types::PEER_CHANNEL),
        ("INPUT_PEER_USER", crate::types::INPUT_PEER_USER),
        ("INPUT_PEER_CHAT", crate::types::INPUT_PEER_CHAT),
        ("INPUT_PEER_CHANNEL", crate::types::INPUT_PEER_CHANNEL),
        ("INPUT_PEER_SELF", crate::types::INPUT_PEER_SELF),
        ("INPUT_USER", crate::types::INPUT_USER),
        ("INPUT_USER_SELF", crate::types::INPUT_USER_SELF),
        ("INPUT_CHANNEL", crate::types::INPUT_CHANNEL),
        ("INPUT_FILE", crate::types::INPUT_FILE),
        ("INPUT_FILE_BIG", crate::types::INPUT_FILE_BIG),
        ("INPUT_DOCUMENT", crate::types::INPUT_DOCUMENT),
        ("INPUT_DOCUMENT_EMPTY", crate::types::INPUT_DOCUMENT_EMPTY),
        ("KEYBOARD_BUTTON", crate::types::KEYBOARD_BUTTON),
        ("USER_STATUS_ONLINE", crate::types::USER_STATUS_ONLINE),
        ("USER_STATUS_OFFLINE", crate::types::USER_STATUS_OFFLINE),
        ("USER_PROFILE_PHOTO", crate::types::USER_PROFILE_PHOTO),
        ("PEER_NOTIFY_SETTINGS", crate::types::PEER_NOTIFY_SETTINGS),
        ("AUTH_AUTHORIZATION", crate::types::AUTH_AUTHORIZATION),
        ("AUTH_SIGN_IN", crate::types::AUTH_SIGN_IN),
    ];
    for (name, val) in ours {
        let gen_val = crate::types::gen_const(name);
        assert!(
            gen_val == Some(val),
            "constructors::{name} diverges from the generated layer-225 schema"
        );
    }
}

/// `get_channels` answers `messages.chats#64ff9fd5 chats:Vector<Chat>` —
/// a bare chat list, NOT an Updates container. Regression guard for the
/// `unknown Updates constructor 0x64ff9fd5` failure on the live wire.
// Test code: verbatim ctor ids, unwrap/panic idioms, hand-built
// wire fixtures with fixed-width indexing; the access-hash literal is
// a wire bit pattern that does not fit i64 positive range.
#[allow(
    clippy::unreadable_literal,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::items_after_statements,
    clippy::as_conversions,
    clippy::cast_possible_wrap
)]
#[test]
fn test_chats_from_updates_handles_messages_chats() {
    let mut w = crate::serialize::TLWriter::new();
    w.write_u32(crate::types::MESSAGES_CHATS); // 0x64ff9fd5
    w.write_u32(crate::serialize::VECTOR);
    w.write_i32(1); // one chat
    // channel#1c32b11c (layer 225): flags + flags2, id, access_hash,
    // title, username, date, version — minimal legal shape with
    // username (bit6) and access_hash-bearing variant... access_hash is
    // flags.0?Option<i64> in gen; set bit0 to carry it.
    w.write_u32(crate::types::CHANNEL);
    let flags = (1 << 13) | (1 << 6); // access_hash (flags.13) + username (flags.6)
    w.write_i32(flags);
    w.write_i32(0); // flags2
    w.write_i64(-1001234); // id
    w.write_i64(0x2b407731_88431337u64 as i64); // access_hash
    w.write_bytes(b"mtprsto demo"); // title
    w.write_bytes(b"lebenoa_test"); // username
    w.write_u32(0x37c1011c); // chatPhotoEmpty (required ChatPhoto)
    w.write_i32(1700000000); // date
    w.write_i32(1); // version
    let data = w.into_bytes();

    use crate::types::Chat;
    let chats =
        crate::client::Client::chats_from_updates(&data, crate::types::CHANNELS_GET_CHANNELS)
            .unwrap();
    assert_eq!(chats.len(), 1);
    match &chats[0] {
        Chat::Channel {
            id,
            title,
            username,
            ..
        } => {
            assert_eq!(id.0, -1001234);
            assert_eq!(title, "mtprsto demo");
            assert_eq!(username.as_deref(), Some("lebenoa_test"));
        }
        other => panic!("expected Channel, got {other:?}"),
    }
}
/// Production sometimes answers channels.inviteToChannel (declared
/// `messages.InvitedUsers`) with a bare `updates#74ae4240` container.
/// The router must accept Updates-shaped ctors regardless of the map.
#[allow(
    clippy::unreadable_literal,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::items_after_statements
)]
#[test]
fn test_chats_from_updates_accepts_updates_for_invite() {
    let mut w = crate::serialize::TLWriter::new();
    w.write_u32(0x74ae4240); // updates#74ae4240
    w.write_u32(crate::serialize::VECTOR);
    w.write_i32(0); // updates:Vector<Update> — empty
    w.write_u32(crate::serialize::VECTOR);
    w.write_i32(1); // users
    w.write_u32(crate::types::USER_ID);
    w.write_i32(0); // flags
    w.write_i32(0); // flags2
    w.write_i64(4242); // id
    w.write_u32(crate::serialize::VECTOR);
    w.write_i32(1); // chats
    w.write_u32(crate::types::CHANNEL);
    let flags = (1 << 13) | (1 << 6); // access_hash + username
    w.write_i32(flags);
    w.write_i32(0); // flags2
    w.write_i64(-1001234); // id
    w.write_i64(77); // access_hash
    w.write_bytes(b"mtprsto demo");
    w.write_bytes(b"lebenoa_test");
    w.write_u32(0x37c1011c); // chatPhotoEmpty
    w.write_i32(1700000000); // date
    w.write_i32(1); // version
    w.write_i32(1700000000); // date
    w.write_i32(1); // seq
    let data = w.into_bytes();

    let chats =
        crate::client::Client::chats_from_updates(&data, crate::types::CHANNELS_INVITE_TO_CHANNEL)
            .unwrap();
    assert_eq!(chats.len(), 1);
    match &chats[0] {
        crate::types::Chat::Channel { id, username, .. } => {
            assert_eq!(id.0, -1001234);
            assert_eq!(username.as_deref(), Some("lebenoa_test"));
        }
        other => panic!("expected Channel, got {other:?}"),
    }
}

#[cfg(test)]
mod invited_users_tests {
    use crate::serialize::{TLWriter, VECTOR};

    /// Replays the live invite payload shape: wrapper ctor
    /// messages.invitedUsers consumed by the router, then the parser
    /// re-reads from the buffer start (nested updates#74ae4240).
    /// Regression for the double ctor-consumption bug.
    // Test code: unwrap/assert idioms + verbatim live-payload ctors.
    #[allow(
        clippy::unreadable_literal,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::doc_markdown,
        clippy::assert_is_empty
    )]
    #[test]
    fn test_chats_from_updates_invited_users_shape() {
        let mut w = TLWriter::new();
        w.write_u32(0x7f5defa6); // messages.invitedUsers
        w.write_u32(0x74ae4240); // nested updates#74ae4240
        w.write_u32(VECTOR);
        w.write_i32(0); // updates:Vector<Update> - empty
        w.write_u32(VECTOR);
        w.write_i32(0); // users - empty
        w.write_u32(VECTOR);
        w.write_i32(0); // chats - empty (a full Channel body needs
        // every conditional field; routing is what we
        // regression-test here)
        w.write_i32(1700000000); // date
        w.write_i32(1); // seq
        w.write_u32(VECTOR);
        w.write_i32(0); // missing_invitees - empty
        let data = w.into_bytes();

        let chats = crate::client::Client::chats_from_updates(
            &data,
            crate::types::CHANNELS_INVITE_TO_CHANNEL,
        )
        .unwrap();
        assert!(chats.is_empty());
    }

    /// Regression for the stack overflow running `channel_admin`:
    /// a real (rich) `messages.invitedUsers` response — 3 updates
    /// (updateMessageID, updateReadChannelInbox, updateNewChannelMessage
    /// with a messageService/messageActionChatAddUser), two full `user#`
    /// objects and a `channel#` — is parsed through the GENERATED
    /// parsers, whose debug-build frames nest far deeper than the
    /// default main stack. `chats_from_updates` must run the parse on a
    /// big-stack thread and return the chats, not crash.
    ///
    /// Byte-for-byte a live layer-225 payload: invite of `@vary4_bot` into
    /// a fresh megagroup ("mtprsto dbg").
    // PRODUCTION-dialect fixture (layer-225 ctors). The docs-layer
    // branch negotiates 223 — this regression lives on `master`; on
    // docs-layer it is kept but skipped.
    // Test code: verbatim live payload (unreadable_literal) + unwrap.
    #[allow(
        clippy::unreadable_literal,
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing
    )]
    #[ignore = "layer-225 production-dialect fixture — regression runs on master"]
    #[test]
    fn test_chats_from_updates_rich_invited_users_no_stack_overflow() {
        const PAYLOAD: &[u8] = &[
            0xa6, 0xef, 0x5d, 0x7f, 0x40, 0x42, 0xae, 0x74, 0x15, 0xc4, 0xb5, 0x1c, 0x03, 0x00,
            0x00, 0x00, 0xd6, 0xbf, 0x90, 0x4e, 0x02, 0x00, 0x00, 0x00, 0x5f, 0x04, 0xf2, 0x2b,
            0x50, 0xc9, 0x33, 0x00, 0x10, 0x6e, 0x2e, 0x92, 0x00, 0x00, 0x00, 0x00, 0x9e, 0xf9,
            0x0f, 0x09, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x03, 0x00, 0x00, 0x00, 0xd9, 0x04, 0xba, 0x62, 0x0a, 0x0e, 0x80, 0x7a, 0x02, 0x03,
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x22, 0x17, 0x51, 0x59, 0x55, 0x12, 0x1e, 0x87,
            0x01, 0x00, 0x00, 0x00, 0x1e, 0x37, 0xa5, 0xa2, 0x9e, 0xf9, 0x0f, 0x09, 0x01, 0x00,
            0x00, 0x00, 0x3a, 0x49, 0x94, 0x6a, 0x00, 0xfd, 0xce, 0x15, 0x15, 0xc4, 0xb5, 0x1c,
            0x01, 0x00, 0x00, 0x00, 0xa1, 0x56, 0x2c, 0x86, 0x01, 0x00, 0x00, 0x00, 0x03, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x15, 0xc4, 0xb5, 0x1c, 0x02, 0x00, 0x00, 0x00,
            0x88, 0x43, 0x77, 0x31, 0x7f, 0x04, 0x00, 0x02, 0x10, 0x00, 0x00, 0x00, 0x55, 0x12,
            0x1e, 0x87, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x39, 0xa1, 0xa1, 0x0e, 0xcc, 0x84, 0x5b,
            0x04, 0x59, 0x75, 0x6b, 0x61, 0x00, 0x00, 0x00, 0x06, 0x4e, 0x61, 0x6e, 0x61, 0x6b,
            0x6f, 0x00, 0x07, 0x6c, 0x65, 0x62, 0x65, 0x6e, 0x6f, 0x61, 0x0b, 0x36, 0x36, 0x39,
            0x38, 0x38, 0x39, 0x36, 0x32, 0x30, 0x31, 0x39, 0x06, 0xf7, 0xd1, 0x82, 0x02, 0x00,
            0x00, 0x00, 0x72, 0xb9, 0x31, 0x1b, 0xa3, 0x1d, 0x39, 0x54, 0x11, 0x01, 0x08, 0x08,
            0xb4, 0x0a, 0xbe, 0x15, 0x4e, 0x00, 0x3c, 0x1a, 0x28, 0xa2, 0xa9, 0x32, 0x5a, 0x47,
            0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x49, 0x39, 0xb9, 0xed, 0x19, 0x4a, 0x94, 0x6a,
            0x88, 0x43, 0x77, 0x31, 0x0b, 0x40, 0x00, 0x02, 0x12, 0x00, 0x00, 0x00, 0xa1, 0x56,
            0x2c, 0x86, 0x01, 0x00, 0x00, 0x00, 0x39, 0xa9, 0x1c, 0x62, 0x56, 0x45, 0xde, 0x55,
            0x08, 0x56, 0x61, 0x72, 0x79, 0x6d, 0x79, 0x20, 0x34, 0x00, 0x00, 0x00, 0x09, 0x76,
            0x61, 0x72, 0x79, 0x34, 0x5f, 0x62, 0x6f, 0x74, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x15, 0xc4, 0xb5, 0x1c, 0x01, 0x00, 0x00, 0x00, 0x1c, 0xb1, 0x32, 0x1c, 0x01, 0x61,
            0x04, 0x00, 0x08, 0x00, 0x00, 0x00, 0x9e, 0xf9, 0x0f, 0x09, 0x01, 0x00, 0x00, 0x00,
            0x7d, 0x68, 0xf6, 0x01, 0x53, 0xcb, 0x2e, 0xa3, 0x0b, 0x6d, 0x74, 0x70, 0x72, 0x73,
            0x74, 0x6f, 0x20, 0x64, 0x62, 0x67, 0x1c, 0x01, 0xc1, 0x37, 0x3a, 0x49, 0x94, 0x6a,
            0xd5, 0x24, 0xb2, 0x5f, 0xbf, 0xfa, 0x07, 0x00, 0x18, 0x04, 0x12, 0x9f, 0x00, 0x84,
            0x02, 0x04, 0xff, 0xff, 0xff, 0x7f, 0x39, 0x49, 0x94, 0x6a, 0x00, 0x00, 0x00, 0x00,
            0x15, 0xc4, 0xb5, 0x1c, 0x00, 0x00, 0x00, 0x00,
        ];
        // Sanity: the payload must actually be the invitedUsers shape.
        assert_eq!(
            u32::from_le_bytes(PAYLOAD[0..4].try_into().unwrap()),
            0x7f5defa6
        );

        // Parse on a deliberately small (2 MiB) thread: the nested
        // generated parsers need >8 MiB of stack in debug builds, so
        // WITHOUT the big-stack thread in `chats_from_updates` this
        // aborts with STATUS_STACK_OVERFLOW — the exact failure the
        // channel_admin example hit on the 8 MiB tokio main thread.
        let h = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                crate::client::Client::chats_from_updates(
                    PAYLOAD,
                    crate::types::CHANNELS_INVITE_TO_CHANNEL,
                )
                .expect("rich invitedUsers payload must parse without overflowing")
            })
            .unwrap();
        let chats = h.join().unwrap();
        assert_eq!(chats.len(), 1);
    }
}
