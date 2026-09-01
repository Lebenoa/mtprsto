//! Live wire sweep — client-level mtprsto functions, verified against real
//! Telegram servers.
//!
//! This is the file the layout tests cannot be: layout tests prove the
//! bytes we *think* we serialize; this sweep proves the server *accepts*
//! them. Both constructor mistakes that shipped (the dead
//! `inputPhotoFileLocation` id, the legacy `channels.getMessages`) were
//! invisible until a live run.
//!
//! # Running
//!
//! ```text
//! MTPRSTO_LIVE=1 \
//! MTPRSTO_API_ID=… MTPRSTO_API_HASH=… \
//! MTPRSTO_SESSION_FILE=target/live-session.json \
//! cargo nextest run --run-ignored only -E 'test(live_sweep)' --nocapture
//! ```
//!
//! Without a session file only Tier 0 runs (unauthorized: connect,
//! handshake, nearest-DC, ping). With one, Tier 1 sweeps the authorized
//! surface — including a write phase confined to a throwaway channel the
//! sweep creates (`mtprsto-live-<unix ts>`), wiped and left behind at the
//! end. Nothing outside that channel is touched; profile-mutating calls
//! (`update_profile_photo`, `delete_photos`) and interactive flows (phone
//! login, invite links, bot callbacks) are deliberately not swept.
//!
//! `MTPRSTO_BOT_TOKEN` optionally adds a second client authorized as a bot
//! (own session file, never mixes with the user one).

#![allow(clippy::unwrap_used, clippy::expect_used)] // test: failures panic loudly by design

use mtprsto::client::Client;
use mtprsto::error::Error;
use mtprsto::rpc::{self, ChannelParticipantsFilter, TypingAction};
use mtprsto::types::{AccessHash, FileLocation, InputChannel, InputPeer, MsgId, UserId};

use std::sync::atomic::{AtomicUsize, Ordering};

static PASS: AtomicUsize = AtomicUsize::new(0);
static FAIL: AtomicUsize = AtomicUsize::new(0);
static SKIP: AtomicUsize = AtomicUsize::new(0);

fn pass(name: &str, detail: &str) {
    PASS.fetch_add(1, Ordering::Relaxed);
    let detail = detail.trim();
    if detail.is_empty() {
        println!("  PASS  {name}");
    } else {
        println!("  PASS  {name} — {detail}");
    }
}

fn fail(name: &str, why: String) {
    FAIL.fetch_add(1, Ordering::Relaxed);
    println!("  FAIL  {name}: {why}");
}

fn skip(name: &str, why: &str) {
    SKIP.fetch_add(1, Ordering::Relaxed);
    println!("  SKIP  {name}: {why}");
}

/// Records `r`: Ok → PASS, Err → FAIL carrying the server's message.
fn step<T>(name: &str, detail: fn(&T) -> String, r: mtprsto::Result<T>) -> Option<T> {
    match r {
        Ok(v) => {
            pass(name, &detail(&v));
            Some(v)
        }
        Err(e) => {
            fail(name, e.to_string());
            None
        }
    }
}

/// A 70-byte valid 1x1 grayscale PNG — a real image, small enough to send
/// as a compressed photo for the photo round-trip.
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn peer_kind(p: &InputPeer) -> &'static str {
    match p {
        InputPeer::Self_ => "self",
        InputPeer::User { .. } => "user",
        InputPeer::Chat { .. } => "chat",
        InputPeer::Channel { .. } => "channel",
        _ => "other",
    }
}

/// `channels.deleteMessages` answers `messages.affectedMessages#84c1f4e6`
/// — the response ctor is exactly the hex the old request constant was
/// confused with.
const AFFECTED_MESSAGES: u32 = 0x84c1_f4e6;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live network sweep — set MTPRSTO_LIVE=1 and provide credentials"]
async fn live_sweep() {
    if env_var("MTPRSTO_LIVE").as_deref() != Some("1") {
        panic!(
            "live_sweep refuses to run without MTPRSTO_LIVE=1 (belt and braces next to #[ignore])"
        );
    }
    let api_id: i32 = env_var("MTPRSTO_API_ID")
        .expect("MTPRSTO_API_ID required")
        .parse()
        .expect("MTPRSTO_API_ID must be an integer");
    let api_hash = env_var("MTPRSTO_API_HASH").expect("MTPRSTO_API_HASH required");
    let session_file =
        env_var("MTPRSTO_SESSION_FILE").unwrap_or_else(|| "target/live-session.json".into());
    let has_session = std::path::Path::new(&session_file).is_file();

    println!(
        "== mtprsto live sweep == api_id {api_id}, session file: {}",
        if has_session {
            session_file.as_str()
        } else {
            "(absent — Tier 0 only)"
        }
    );

    // ------------------------------------------------------------------
    // Tier 0 — unauthorized: transport, handshake, session bootstrap.
    // ------------------------------------------------------------------
    let client = Client::builder()
        .api_id(api_id)
        .api_hash(&api_hash)
        .session(&session_file)
        .build()
        .expect("client builds");

    let started = std::time::Instant::now();
    if let Err(e) = client.connect().await {
        fail("connect (DH handshake + auth key)", e.to_string());
        report_and_finish();
    } else {
        pass(
            "connect (DH handshake + auth key)",
            &format!(
                "{:.1}s, DC {}",
                started.elapsed().as_secs_f32(),
                client.dc_id()
            ),
        );
    }

    let pong = async {
        let mut w = mtprsto::serialize::TLWriter::new();
        w.write_u32(0x7abe_77ec); // ping#7abe77ec ping_id:long
        w.write_i64(0x5ca1_ab1e);
        let raw = client.invoke_raw(w.into_bytes()).await?;
        let ctor = u32::from_le_bytes(
            raw[..4]
                .try_into()
                .map_err(|_| Error::Protocol("ping answer shorter than a constructor".into()))?,
        );
        if ctor == 0x3477_73c5 {
            Ok("pong#347773c5".to_string())
        } else {
            Err(Error::Protocol(format!("expected pong, got {ctor:#x}")))
        }
    };
    let _ = step(
        "ping/pong (invoke_raw round trip)",
        |s: &String| s.clone(),
        pong.await,
    );

    if !has_session {
        skip(
            "Tier 1 (authorized surface)",
            "no session file — produce one from a signed-in account",
        );
        report_and_finish();
    }

    // ------------------------------------------------------------------
    // Tier 1 — authorized read surface.
    // ------------------------------------------------------------------
    let me = client.get_me().await;
    if let Ok(u) = &me {
        println!(
            "        me: {} (@{}) id {}",
            u.full_name(),
            u.username().unwrap_or("-"),
            u.id().0
        );
    }
    let _me = step("get_me (users.getFullUser)", |_| String::new(), me);

    let state = client.get_state().await;
    let _ = step(
        "get_state (updates.getState)",
        |s: &mtprsto::types::State| format!("pts {}, seq {}", s.pts, s.seq),
        state,
    );

    let dialogs = client.get_dialogs().await;
    let _ = step(
        "get_dialogs (messages.getDialogs)",
        |d: &mtprsto::types::Dialogs| format!("{} chats, {} users", d.chats.len(), d.users.len()),
        dialogs,
    );

    let me_peer = client.resolve_peer("me").await;
    let _ = step(
        "resolve_peer(\"me\")",
        |p: &InputPeer| peer_kind(p).to_string(),
        me_peer,
    );

    let durov = client.resolve_username("durov").await;
    let _ = step(
        "resolve_username(\"durov\") (contacts.resolveUsername)",
        |p: &InputPeer| peer_kind(p).to_string(),
        durov,
    );

    if let (Some(me), Ok(())) = (&_me, Ok::<(), Error>(())) {
        let _ = me;
    }
    // users.getUsers needs the self InputUser with the real access hash,
    // which get_me does not carry — Self_ answers for the own account.
    let users = client.get_users(&[mtprsto::types::InputUser::Self_]).await;
    let _ = step(
        "get_users(Self_) (users.getUsers)",
        |us: &Vec<mtprsto::types::User>| format!("{} user(s) back", us.len()),
        users,
    );

    let photos = client
        .get_user_photos(&mtprsto::types::InputUser::Self_, 0, 0, 1)
        .await;
    match photos {
        Ok(bytes) if bytes.is_empty() => skip("get_user_photos", "account has no photos"),
        Ok(bytes) => pass(
            "get_user_photos (photos.getUserPhotos)",
            &format!("newest photo payload {} bytes", bytes.len()),
        ),
        Err(e) => fail("get_user_photos (photos.getUserPhotos)", e.to_string()),
    }

    // ------------------------------------------------------------------
    // Tier 1 write phase — confined to a throwaway channel.
    // ------------------------------------------------------------------
    let title = format!("mtprsto-live-{}", chrono_stamp());
    let created = client
        .create_channel(&title, "temporary live-sweep scratch", false, true)
        .await;
    let channel: Option<InputChannel> = match created {
        Ok(chats) => {
            let found = chats.iter().find_map(|c| match c {
                mtprsto::types::Chat::Channel {
                    id: mtprsto::types::ChatId(cid),
                    access_hash: Some(h),
                    ..
                } => Some(InputChannel::Channel {
                    channel_id: mtprsto::types::ChannelId(*cid),
                    access_hash: *h,
                }),
                _ => None,
            });
            match &found {
                Some(_) => pass("create_channel (channels.createChannel)", &title),
                None => fail(
                    "create_channel (channels.createChannel)",
                    "created, but no Channel object in response".into(),
                ),
            }
            found
        }
        Err(e) => {
            fail("create_channel (channels.createChannel)", e.to_string());
            None
        }
    };

    if channel.is_none() {
        skip(
            "Tier 1 write phase",
            "channel creation failed — downstream steps need it",
        );
    }

    if let Some(channel) = &channel {
        let chan_str = format!("-100{}", channel_channel_id(channel));
        // String-form wrappers resolve "-100…" through the peer cache
        // persisted at create time; fall back to the object form where a
        // wrapper demands InputChannel/InputPeer directly.
        let peer = InputPeer::Channel {
            channel_id: mtprsto::types::ChannelId(channel_channel_id(channel)),
            access_hash: AccessHash(channel_access_hash(channel)),
        };

        let got_channels = client.get_channels(std::slice::from_ref(channel)).await;
        let _ = step(
            "get_channels (channels.getChannels)",
            |cs: &Vec<mtprsto::types::Chat>| format!("{} chat(s) back", cs.len()),
            got_channels,
        );

        // Text send → edit → fetch round trip. The fetch is the
        // channels.getMessages#ad8c9a23 path that replaced the dropped
        // legacy constructor.
        let sent = client
            .send_to_peer(&peer, "mtprsto live sweep — text")
            .await;
        let text_id: Option<MsgId> = match sent {
            Ok(id) => {
                pass(
                    "send_to_peer (messages.sendMessage)",
                    &format!("id {}", id.0),
                );
                Some(id)
            }
            Err(e) => {
                fail("send_to_peer (messages.sendMessage)", e.to_string());
                None
            }
        };

        if let Some(id) = text_id {
            let edited = client
                .edit_message(
                    &chan_str,
                    i32::try_from(id.0).unwrap_or(0),
                    "mtprsto live sweep — edited",
                )
                .await;
            let _ = step(
                "edit_message (messages.editMessage)",
                |(): &()| String::new(),
                edited,
            );
        }

        let typing = client
            .set_typing(&chan_str, text_id, TypingAction::Cancel)
            .await;
        let _ = step(
            "set_typing (messages.setTyping)",
            |(): &()| String::new(),
            typing,
        );

        // Document round trip: exact bytes in, exact bytes out.
        let doc_bytes: Vec<u8> = (0..64 * 1024u32).map(|i| (i % 251) as u8).collect();
        let doc_path = std::env::temp_dir().join("mtprsto-sweep-doc.bin");
        std::fs::write(&doc_path, &doc_bytes).expect("scratch doc written");
        let doc_send = async {
            let builder = client.send_file(&chan_str, &doc_path).await?;
            builder.caption("sweep document").send().await
        };
        let doc_id: Option<MsgId> = match doc_send.await {
            Ok(id) => {
                pass(
                    "send_file document (upload + messages.sendMedia)",
                    &format!("id {}", id.0),
                );
                Some(id)
            }
            Err(e) => {
                fail(
                    "send_file document (upload + messages.sendMedia)",
                    e.to_string(),
                );
                None
            }
        };

        // Photo round trip: PNG in → server re-encode → photo location →
        // download. Exercises inputPhotoFileLocation#40181ffe end to end.
        let png_path = std::env::temp_dir().join("mtprsto-sweep.png");
        std::fs::write(&png_path, TINY_PNG).expect("scratch png written");
        let photo_send = async {
            let builder = client.send_file(&chan_str, &png_path).await?;
            builder.as_photo().send().await
        };
        let photo_id: Option<MsgId> = match photo_send.await {
            Ok(id) => {
                pass(
                    "send_file as_photo (inputMediaUploadedPhoto)",
                    &format!("id {}", id.0),
                );
                Some(id)
            }
            Err(e) => {
                fail(
                    "send_file as_photo (inputMediaUploadedPhoto)",
                    e.to_string(),
                );
                None
            }
        };

        // Fetch everything back through channels.getMessages.
        let fetch_ids: Vec<MsgId> = [text_id, doc_id, photo_id].into_iter().flatten().collect();
        let fetched = if fetch_ids.is_empty() {
            Ok(Vec::new())
        } else {
            client.get_messages(&peer, &fetch_ids).await
        };
        let mut fetched_msgs: Vec<mtprsto::types::Message> = Vec::new();
        match &fetched {
            Ok(msgs) => {
                let text_ok = text_id.is_none_or(|id| {
                    msgs.iter()
                        .any(|m| m.id() == id && m.text().contains("live sweep"))
                });
                let doc_ok = doc_id
                    .is_none_or(|id| msgs.iter().any(|m| m.id() == id && m.document().is_some()));
                let photo_ok = photo_id
                    .is_none_or(|id| msgs.iter().any(|m| m.id() == id && m.photo().is_some()));
                if text_ok && doc_ok && photo_ok {
                    pass(
                        "get_messages (channels.getMessages#ad8c9a23)",
                        &format!("{} message(s) verified", msgs.len()),
                    );
                } else {
                    fail(
                        "get_messages (channels.getMessages#ad8c9a23)",
                        format!(
                            "round trip mismatch: text={text_ok} doc={doc_ok} photo={photo_ok}"
                        ),
                    );
                }
                fetched_msgs = msgs.clone();
            }
            Err(e) => fail(
                "get_messages (channels.getMessages#ad8c9a23)",
                e.to_string(),
            ),
        }

        // Download the sent document through the fetched location and
        // compare bytes — upload.getFile + the range machinery.
        let doc_msg = doc_id.and_then(|id| {
            fetched_msgs
                .iter()
                .find(|m| m.id() == id)
                .and_then(|m| m.document())
        });
        match doc_msg
            .as_ref()
            .and_then(mtprsto::types::Document::location)
        {
            Some(loc) => {
                let size = doc_msg
                    .as_ref()
                    .map_or(0u64, |d| u64::try_from(document_size(d)).unwrap_or(0));
                match client.download(&loc, Some(size)).await {
                    Ok(got) if got == doc_bytes => {
                        pass("download document (upload.getFile)", "byte-identical");
                    }
                    Ok(got) => fail(
                        "download document (upload.getFile)",
                        format!("{} bytes back, want {}", got.len(), doc_bytes.len()),
                    ),
                    Err(e) => fail("download document (upload.getFile)", e.to_string()),
                }
            }
            None => skip("download document", "no FileLocation on fetched document"),
        }

        let recent = client.get_recent_messages(&peer, 10).await;
        let _ = step(
            "get_recent_messages (messages.getHistory)",
            |ms: &Vec<mtprsto::types::Message>| format!("{} recent message(s)", ms.len()),
            recent,
        );

        let hits = client.search(&chan_str, "sweep", 10).await;
        let _ = step(
            "search (messages.search)",
            |ms: &Vec<mtprsto::types::Message>| format!("{} hit(s)", ms.len()),
            hits,
        );

        if let Some(id) = doc_id {
            let fwd = client
                .forward_messages(&chan_str, &[id], &chan_str, false, true)
                .await;
            let _ = step(
                "forward_messages (messages.forwardMessages)",
                |m: &Option<MsgId>| {
                    m.map(|i| format!("forwarded as id {}", i.0))
                        .unwrap_or_default()
                },
                fwd,
            );
        }

        if let Some(id) = text_id {
            let pinned = client.pin_message(&chan_str, id, true, false).await;
            let _ = step(
                "pin_message (messages.updatePinnedMessage)",
                |m: &Option<MsgId>| m.map(|i| format!("id {}", i.0)).unwrap_or_default(),
                pinned,
            );
            let unpinned = client.pin_message(&chan_str, id, true, true).await;
            let _ = step(
                "unpin_message (messages.updatePinnedMessage)",
                |m: &Option<MsgId>| m.map(|i| format!("id {}", i.0)).unwrap_or_default(),
                unpinned,
            );
        }

        let read = client.read_history(&chan_str, 0).await;
        let _ = step(
            "read_history (messages.readHistory)",
            |a: &mtprsto::client_wrappers::AffectedMessages| format!("pts {}", a.pts),
            read,
        );

        let parts = client
            .get_participants(channel, &ChannelParticipantsFilter::Recent, 0, 10, 0)
            .await;
        let _ = step(
            "get_participants (channels.getParticipants)",
            |(count, _): &(i32, Vec<u8>)| format!("{count} participant(s)"),
            parts,
        );

        // Delete everything the sweep put in the channel. THIS is the
        // first live exercise of channels.deleteMessages#84c1fd4e, whose
        // constant shipped with transposed hex digits. There is no Client
        // wrapper yet — the public builder through invoke_raw is the
        // honest path, and it is what ii-drive's delete flow needs next.
        let doomed: Vec<MsgId> = fetch_ids.clone();
        let deleted = async {
            let payload = rpc::build_channels_delete_messages(channel, &doomed);
            let raw = client.invoke_raw(payload).await?;
            let ctor =
                u32::from_le_bytes(raw[..4].try_into().map_err(|_| {
                    Error::Protocol("delete answer shorter than a constructor".into())
                })?);
            if ctor == AFFECTED_MESSAGES {
                Ok("affectedMessages back".to_string())
            } else {
                Err(Error::Protocol(format!(
                    "expected messages.affectedMessages, got {ctor:#x}"
                )))
            }
        };
        let _ = step(
            "delete channel messages (channels.deleteMessages#84c1fd4e)",
            |s: &String| s.clone(),
            deleted.await,
        );

        let wiped = client.delete_history(&chan_str, 0, false, true).await;
        let _ = step(
            "delete_history (messages.deleteHistory)",
            |a: &mtprsto::client_wrappers::AffectedMessages| format!("pts {}", a.pts),
            wiped,
        );

        let left = client.leave_channel(channel).await;
        let _ = step(
            "leave_channel (channels.leaveChannel)",
            |(): &()| String::new(),
            left,
        );
        println!("        scratch channel \"{title}\" wiped and left");
    }

    // ------------------------------------------------------------------
    // Optional bot leg — separate client, separate session file.
    // ------------------------------------------------------------------
    match env_var("MTPRSTO_BOT_TOKEN") {
        Some(token) => {
            let bot = Client::builder()
                .api_id(api_id)
                .api_hash(&api_hash)
                .session("target/live-bot-session.json")
                .build()
                .expect("bot client builds");
            let authed = bot.authorize_bot(&token).await;
            let _ = step(
                "authorize_bot (bot session)",
                |(): &()| String::new(),
                authed,
            );
        }
        None => skip("authorize_bot", "MTPRSTO_BOT_TOKEN not set"),
    }

    let plain_delete = client.delete_messages(&[]).await;
    let _ = step(
        "delete_messages plain, empty (messages.deleteMessages)",
        |(): &()| String::new(),
        plain_delete,
    );

    report_and_finish();
}

fn channel_channel_id(c: &InputChannel) -> i64 {
    match c {
        InputChannel::Channel { channel_id, .. } => channel_id.0,
        _ => 0,
    }
}

fn channel_access_hash(c: &InputChannel) -> i64 {
    match c {
        InputChannel::Channel { access_hash, .. } => access_hash.0,
        _ => 0,
    }
}

fn document_size(d: &mtprsto::types::Document) -> i64 {
    match d {
        mtprsto::types::Document::Document { size, .. } => *size,
        mtprsto::types::Document::Empty { .. } => 0,
    }
}

fn chrono_stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_secs().to_string()
}

fn report_and_finish() -> ! {
    let (p, f, s) = (
        PASS.load(Ordering::Relaxed),
        FAIL.load(Ordering::Relaxed),
        SKIP.load(Ordering::Relaxed),
    );
    println!("== sweep done: {p} passed, {f} failed, {s} skipped ==");
    if f > 0 {
        panic!("{f} live step(s) failed — see FAIL lines above");
    }
    std::process::exit(0);
}

// AccessHash/UserId re-exports are used through the types module; keep the
// import list honest for the pieces referenced only in doc context.
#[allow(unused)]
fn _type_touch(_: AccessHash, _: UserId, _: FileLocation) {}
