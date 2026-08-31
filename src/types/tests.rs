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
        // keyboardButton#2f67a72f flags:# style:flags.10?KeyboardButtonStyle
        //   text:string type:ButtonType
        let mut w = TLWriter::new();
        w.write_u32(crate::types::reply_markup_gen::KEYBOARD_BUTTON_ID);
        w.write_i32(0); // flags (no style)
        w.write_bytes(b"Click me");
        w.write_u32(0xc9dd90e9); // buttonTypeDefault#c9dd90e9
        let mut r = TLReader::new(w.as_bytes());
        let btn = crate::types::reply_markup_gen::KeyboardButton::read_from(&mut r).unwrap();
        assert_eq!(btn.text, "Click me");
        assert!(btn.style.is_none());
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
            Updates::UpdateShort { update: Update::ReadMessages { messages }, .. } => {
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
    assert_eq!(df.unread_unmuted_messages_count, 4);}

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
    match user {
        crate::types::user_gen::User::User { id, access_hash, first_name, phone, bot, .. } => {
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
#[test]
fn test_constructor_constants_match_generated() {
    // Layer-223 published ids (from tl.json) for constants whose ctor
    // was re-issued by layer 229 — the docs-layer dialect values.
    let layer223: &[(&str, u32)] = &[
        ("MESSAGE", 0x3AE56482),
        ("DIALOG", 0xD58A08C6),
        ("CHANNEL", 0x1C32B11C),
        ("CHANNEL_FULL", 0xE4E0B29D),
        ("MESSAGE_MEDIA_PHOTO", 0x695150D7),
        ("MESSAGE_MEDIA_POLL", 0x4BD6E798),
        ("MESSAGE_REPLY_HEADER", 0x6917560B),
        ("REPLY_INLINE_MARKUP", 0x48A30254),
        ("KEYBOARD_BUTTON", 0x7D170CFF),
        ("INPUT_REPLY_TO_MESSAGE", 0x869FBE10),
        ("CONTACTS_SEARCH", 0x11F812D8),
        ("MESSAGES_SEND_MESSAGE", 0x545CD15A),
        ("MESSAGES_EDIT_MESSAGE", 0x51E842E1),
        ("USER", 0x31774388),                 // user#31774388 (223 era)
    ];
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
        ("MESSAGE_MEDIA_DOCUMENT", crate::types::MESSAGE_MEDIA_DOCUMENT),
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
        // The generated parsers are built from the 229 schema; a curated
        // constant carrying the 223 id must appear in that type's alias
        // arms (all layer223 entries above are aliased in the generated
        // code), while unreissued ctors must match the canonical id.
        let is_223_reissue = layer223.iter().any(|(n, id)| *n == name && *id == val);
        assert!(
            gen_val.is_some() && (gen_val == Some(val) || is_223_reissue),
            "constructors::{name} diverges from the published layer-223 schema"
        );
    }


}

    /// get_channels answers `messages.chats#64ff9fd5 chats:Vector<Chat>` —
    /// a bare chat list, NOT an Updates container. Regression guard for the
    /// `unknown Updates constructor 0x64ff9fd5` failure on the live wire.
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
        let chats = crate::client::Client::chats_from_updates(&data, crate::types::CHANNELS_GET_CHANNELS).unwrap();
        assert_eq!(chats.len(), 1);
        match &chats[0] {
            Chat::Channel { id, title, username, .. } => {
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

    let chats = crate::client::Client::chats_from_updates(
        &data,
        crate::types::CHANNELS_INVITE_TO_CHANNEL,
    )
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
    /// Byte-for-byte a live layer-225 payload: invite of @vary4_bot into
    /// a fresh megagroup ("mtprsto dbg").
    // PRODUCTION-dialect fixture (layer-225 ctors). The docs-layer
    // branch negotiates 223 — this regression lives on `master`; on
    // docs-layer it is kept but skipped.
    #[ignore = "layer-225 production-dialect fixture — regression runs on master"]
    #[test]
    fn test_chats_from_updates_rich_invited_users_no_stack_overflow() {
        const PAYLOAD: &[u8] = &[
            0xa6, 0xef, 0x5d, 0x7f, 0x40, 0x42, 0xae, 0x74, 0x15, 0xc4, 0xb5, 0x1c,
            0x03, 0x00, 0x00, 0x00, 0xd6, 0xbf, 0x90, 0x4e, 0x02, 0x00, 0x00, 0x00,
            0x5f, 0x04, 0xf2, 0x2b, 0x50, 0xc9, 0x33, 0x00, 0x10, 0x6e, 0x2e, 0x92,
            0x00, 0x00, 0x00, 0x00, 0x9e, 0xf9, 0x0f, 0x09, 0x01, 0x00, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00,
            0xd9, 0x04, 0xba, 0x62, 0x0a, 0x0e, 0x80, 0x7a, 0x02, 0x03, 0x00, 0x00,
            0x02, 0x00, 0x00, 0x00, 0x22, 0x17, 0x51, 0x59, 0x55, 0x12, 0x1e, 0x87,
            0x01, 0x00, 0x00, 0x00, 0x1e, 0x37, 0xa5, 0xa2, 0x9e, 0xf9, 0x0f, 0x09,
            0x01, 0x00, 0x00, 0x00, 0x3a, 0x49, 0x94, 0x6a, 0x00, 0xfd, 0xce, 0x15,
            0x15, 0xc4, 0xb5, 0x1c, 0x01, 0x00, 0x00, 0x00, 0xa1, 0x56, 0x2c, 0x86,
            0x01, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x15, 0xc4, 0xb5, 0x1c, 0x02, 0x00, 0x00, 0x00, 0x88, 0x43, 0x77, 0x31,
            0x7f, 0x04, 0x00, 0x02, 0x10, 0x00, 0x00, 0x00, 0x55, 0x12, 0x1e, 0x87,
            0x01, 0x00, 0x00, 0x00, 0x3c, 0x39, 0xa1, 0xa1, 0x0e, 0xcc, 0x84, 0x5b,
            0x04, 0x59, 0x75, 0x6b, 0x61, 0x00, 0x00, 0x00, 0x06, 0x4e, 0x61, 0x6e,
            0x61, 0x6b, 0x6f, 0x00, 0x07, 0x6c, 0x65, 0x62, 0x65, 0x6e, 0x6f, 0x61,
            0x0b, 0x36, 0x36, 0x39, 0x38, 0x38, 0x39, 0x36, 0x32, 0x30, 0x31, 0x39,
            0x06, 0xf7, 0xd1, 0x82, 0x02, 0x00, 0x00, 0x00, 0x72, 0xb9, 0x31, 0x1b,
            0xa3, 0x1d, 0x39, 0x54, 0x11, 0x01, 0x08, 0x08, 0xb4, 0x0a, 0xbe, 0x15,
            0x4e, 0x00, 0x3c, 0x1a, 0x28, 0xa2, 0xa9, 0x32, 0x5a, 0x47, 0x00, 0x00,
            0x05, 0x00, 0x00, 0x00, 0x49, 0x39, 0xb9, 0xed, 0x19, 0x4a, 0x94, 0x6a,
            0x88, 0x43, 0x77, 0x31, 0x0b, 0x40, 0x00, 0x02, 0x12, 0x00, 0x00, 0x00,
            0xa1, 0x56, 0x2c, 0x86, 0x01, 0x00, 0x00, 0x00, 0x39, 0xa9, 0x1c, 0x62,
            0x56, 0x45, 0xde, 0x55, 0x08, 0x56, 0x61, 0x72, 0x79, 0x6d, 0x79, 0x20,
            0x34, 0x00, 0x00, 0x00, 0x09, 0x76, 0x61, 0x72, 0x79, 0x34, 0x5f, 0x62,
            0x6f, 0x74, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x15, 0xc4, 0xb5, 0x1c,
            0x01, 0x00, 0x00, 0x00, 0x1c, 0xb1, 0x32, 0x1c, 0x01, 0x61, 0x04, 0x00,
            0x08, 0x00, 0x00, 0x00, 0x9e, 0xf9, 0x0f, 0x09, 0x01, 0x00, 0x00, 0x00,
            0x7d, 0x68, 0xf6, 0x01, 0x53, 0xcb, 0x2e, 0xa3, 0x0b, 0x6d, 0x74, 0x70,
            0x72, 0x73, 0x74, 0x6f, 0x20, 0x64, 0x62, 0x67, 0x1c, 0x01, 0xc1, 0x37,
            0x3a, 0x49, 0x94, 0x6a, 0xd5, 0x24, 0xb2, 0x5f, 0xbf, 0xfa, 0x07, 0x00,
            0x18, 0x04, 0x12, 0x9f, 0x00, 0x84, 0x02, 0x04, 0xff, 0xff, 0xff, 0x7f,
            0x39, 0x49, 0x94, 0x6a, 0x00, 0x00, 0x00, 0x00, 0x15, 0xc4, 0xb5, 0x1c,
            0x00, 0x00, 0x00, 0x00,
        ];
        // Sanity: the payload must actually be the invitedUsers shape.
        assert_eq!(u32::from_le_bytes(PAYLOAD[0..4].try_into().unwrap()), 0x7f5defa6);

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





