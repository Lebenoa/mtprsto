#[cfg(test)]
mod type_tests {
    use crate::types::*;
    use crate::serialize::{TLWriter, TLReader, VECTOR};

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
            InputPeer::User { user_id, access_hash } => {
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
            InputPeer::Channel { channel_id, access_hash } => {
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
            Peer::Channel { channel_id: ChannelId(3) },
        ] {
            let mut w = TLWriter::new();
            peer.write_to(&mut w);
            let mut r = TLReader::new(w.as_bytes());
            let parsed = Peer::read_from(&mut r).unwrap();
            assert_eq!(&parsed, peer);
        }
    }

    #[test]
    fn test_keyboard_button_text_roundtrip() {
        let btn = KeyboardButton::Text { text: "Click me".into() };
        let mut w = TLWriter::new();
        btn.write_to(&mut w);
        let mut r = TLReader::new(w.as_bytes());
        assert_eq!(r.read_u32().unwrap(), KEYBOARD_BUTTON);
        assert_eq!(String::from_utf8(r.read_bytes().unwrap()).unwrap(), "Click me");
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
            INPUT_PEER_SELF, INPUT_PEER_USER, INPUT_PEER_USER_FROM_ID,
            INPUT_PEER_CHAT, INPUT_PEER_CHANNEL, INPUT_PEER_CHANNEL_FROM_ID,
        ];
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len(), "duplicate constructor IDs detected");
    }

    #[test]
    fn test_user_read_parses_flags2_before_id() {
        // user#31774388 always serializes TWO flag words before id:long.
        // A bot-auth response typically has flags=0x4000 (bot flag),
        // flags2=0, id=777.
        let mut w = TLWriter::new();
        w.write_u32(USER); // 0x31774388
        w.write_i32(1 << 14); // flags: bot=true
        w.write_i32(0); // flags2 (must be consumed here)
        w.write_i64(777); // id
        // access_hash:flags.0 not set — nothing follows
        let mut r = TLReader::new(w.as_bytes());
        let user = User::read_from(&mut r).unwrap();
        assert_eq!(user.id().0, 777, "flags2 word must be consumed before id");
        assert!(user.is_bot());
    }

    /// SPEC §6: `updates` container decodes inner update constructors.
    #[test]
    fn test_updates_parse_decodes_new_message() {
        let mut w = TLWriter::new();
        w.write_u32(UPDATES);
        w.write_i32(0); // flags
        w.write_i32(1_700_000_000); // date
        w.write_i32(5); // seq
        // updates:Vector<Update> — one updateNewMessage
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(1);
        w.write_u32(UPDATE_NEW_MESSAGE);
        // message:message#3ae56482 — nested object carries its own ctor
        w.write_u32(MESSAGE);
        w.write_i32(0); // message flags
        w.write_i64(77); // id
        w.write_u32(PEER_USER);
        w.write_i64(42); // peer_id user
        w.write_i32(1_700_000_000); // date
        w.write_bytes(b"hi"); // message text
        w.write_i32(10); // pts
        w.write_i32(1); // pts_count
        // chats:Vector<Chat>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);
        // users:Vector<User>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(0);

        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::Updates { updates: list, seq, .. } => {
                assert_eq!(seq, 5);
                assert_eq!(list.len(), 1);
                match &list[0] {
                    Update::NewMessage { message, pts, pts_count } => {
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
        // messages:Vector<int>
        w.write_u32(crate::serialize::VECTOR);
        w.write_i32(2);
        w.write_i32(1);
        w.write_i32(2);
        w.write_i32(1_700_000_000); // date
        w.write_i32(6); // seq

        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::UpdateShort { update: Update::ReadMessages { messages }, seq, .. } => {
                assert_eq!(seq, 6);
                assert_eq!(messages, vec![MsgId(1), MsgId(2)]);
            }
            other => panic!("expected UpdateShort ReadMessages, got {other:?}"),
        }
    }

    /// Unknown inner ctors fall through to Update::Other instead of erroring.
    #[test]
    fn test_updates_unknown_inner_is_other() {
        let mut w = TLWriter::new();
        w.write_u32(UPDATE_SHORT);
        w.write_u32(0xDEADBEEF); // unknown update ctor
        w.write_i32(0); // (opaque inner bytes — decoder keeps them unread)
        w.write_i32(1_700_000_000);
        w.write_i32(0);

        let updates = Updates::parse(&w.into_bytes()).unwrap();
        match updates {
            Updates::UpdateShort { update: Update::Other { constructor }, .. } => {
                assert_eq!(constructor, 0xDEADBEEF);
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

// --- Reply types (SPEC §7) round-trips -------------------------------------

#[test]
fn test_message_entity_roundtrip() {
    use crate::types::{read_message_entities, MessageEntityKind};

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
    assert_eq!(ents[0].kind, MessageEntityKind::Bold);
    assert_eq!(ents[1].kind, MessageEntityKind::TextUrl { url: "https://example.com".into() });
    assert_eq!(ents[1].offset, 5);
    assert_eq!(ents[1].length, 10);
}

#[test]
fn test_inline_reply_markup_roundtrip() {
    use crate::types::{read_reply_markup, KeyboardButtonKind};

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
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].buttons.len(), 2);
            match (&rows[0].buttons[0], &rows[0].buttons[1]) {
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
    use crate::types::{read_document_attributes, DocumentAttribute};

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
    assert_eq!(attrs[0], DocumentAttribute::Filename { file_name: "report.pdf".into() });
    match &attrs[1] {
        DocumentAttribute::Video { duration, w, h, supports_streaming, .. } => {
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
    use crate::types::{read_photo_sizes, PhotoSizeFull};

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
    assert_eq!(
        sizes[0],
        PhotoSizeFull::Size { type_: "x".into(), w: 800, h: 600, size: 123_456 }
    );
    assert_eq!(
        sizes[1],
        PhotoSizeFull::Stripped { type_: "i".into(), bytes: vec![1, 2, 3] }
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
}
