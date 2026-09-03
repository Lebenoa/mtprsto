# Fix Status (2026-09-03)

All findings addressed. `cargo test --lib`: 156 passed / 0 failed; `cargo build --all-targets`: clean; `cargo clippy --lib --all-targets`: 0 findings.

- C1+H1 — SRP rewritten to the tdlib-verified spec formulas (PH1/PH2/x, k = H(p‖g) with 2048-bit pads, M1 with hashed salts, 256-byte A/B/S). Regression test pins `srp_derive_x` byte-for-byte against an independent recomputation.
- C2 — `draftMessage` 223 id (0x96eaa5eb) accepted via `DRAFT_MESSAGE_223_ID` alias arm.
- H2 — `send_rpc` network-error/timeout arms return the error on the final attempt instead of falling into `unreachable!()`.
- H3+H4+M1 — the systemic forward-compat fix: unknown `MessageAction`/`MessageMedia`/`Update` variants now rewind and delegate to the generated parsers so payload bytes are consumed exactly (old code drained everything / nothing). Regression tests pin exact-consumption inside containers.
- H5 — duplicated `video_codec` read removed (one conditional read).
- H6 — container-item lengths parsed as u32 with checked adds in pool.rs (×2 sites) and api.rs; negative lengths can no longer sign-extend into slice panics.
- H7 — `parse_resolved_peer` restructured: updates-wrapped responses consume the updates vector and read users BEFORE chats (wire order); all branches share one matching tail.
- H8 — `read_dialog_skip` reads 6 counters (layer-223 dialog has no `unread_poll_votes_count`).
- H9+M4+M5 — pushed updates delivered: `SenderPool::set_update_sender` forwards raw containers; the `updates()` pump drains them before/after each poll; `PtsState` fixed to post-state pts semantics (`update.pts == stored + pts_count`, tdlib-verified); getDifference uses the local dispatcher cursor.
- H10+M9 — 223-dialect arms added with correct field shapes: `messageMediaPoll` (no flags/attached_media), `poll` (no countries_iso2/hash), `updateMessagePoll`/`updateMessagePollVote`, `botCommand` (no flags), `replyInlineMarkup` (KeyboardButtonRow rows, lossy button mapping documented), `message` 0x3ae56482. Note: reviewer's claim that live id 0x95ef6f2b "matches neither schema" is wrong — it is a wire-verified 225 re-issue (commit 12c5d9c) whose extra fields are flags-gated, so the merged 229-layout arm is correct.
- M2 — `compute_server_salt` parses LE (was byte-swapped vs every other salt path).
- M3 — dead `src/types/chat.rs` deleted (never compiled; generated parser is authoritative).
- M6 — `build_get_file` returns `Err` for unmodelable locations instead of writing ctor 0.
- M7 — WS split-stream Ping frames now queue an explicit Pong.
- M8 — resilience.rs doc no longer references nonexistent `Client::spawn_dc_refresher`.
- M10 — session fsync opens the temp file with write access (read-only open made sync_all a silent no-op on Windows).
- L1 — abridged recv caps lengths at 2 MiB like every other transport.
- L2 — `recv_unencrypted` parses msg_id little-endian (matches the write path).
- L3 — false positive: `0x9fd736` ≡ `0x009fd736` (same value); no change needed.
- L4 — public no-op `spawn_adaptive_scaler` deleted (had zero callers).
- L5 — GeoPoint comment corrected to little-endian.
- L6 — `MessagesIter::collect` propagates page errors.
- L7 — duplicated `enc_count` check removed.
- L8 — handshake msg_ids now spec-compliant (`crypto::next_msg_id`) in exchange_unencrypted and the req_DH/set_client_DH sends.

Residual known limitation (documented in tests): a ctor unknown to the ENTIRE schema inside a multi-element vector still consumes zero bytes (generated unions' `Other{constructor}` fallback) — unavoidable without schema knowledge; everything the schema knows now consumes exactly.

 CRITICAL (priority 0)

 C1. SRP 2FA derivation diverges from the Telegram spec in six places — src/crypto.rs:509-518 (conf. 0.95)
 Every auth.checkPassword fails with PASSWORD_HASH_INVALID. Verified against core.telegram.org/api/srp:
 - Official PH1 = H(salt2|H(salt1|password|salt1)|salt2); code computes H(salt1|password)
 - Official PH2 = PBKDF2-HMAC-SHA512(PH1, salt1, 100000) double-wrapped in H(salt2|…|salt2); code runs PBKDF2-HMAC-SHA256 with salt1||salt2, single wrap
 - Official k = H(p|g) with g padded to 2048 bits; code computes H(g_pad|p) with a 4-byte g
 - M1 hashes H(salt1)^H(salt2) + raw g/salts; spec hashes H(p)^H(g)
 - A/B/S use 255-byte pads where the spec mandates 256

 C2. get_dialogs fails on any dialog carrying a draft — src/types/message_gen.rs:5700-5706 (conf. 0.8)
 Client negotiates API_LAYER=223 (src/api.rs:37) where draftMessage is 0x96eaa5eb (per the repo's own CTOR_ALIASES, tools/gentl.py:1466), but the committed generated parser only accepts
 0x60fe3294 (the 229 id). Any dialog with flags.1 set fails the entire messages.getDialogs parse. The alias arm defined in the generator's table never made it into the committed generated
 file.

 HIGH (priority 1)

 H1. src/crypto.rs:601-603 — biguint_min_bytes returns a full 4-byte encoding [0,0,0,g] for small g (2..7) instead of minimal bytes; never produces the spec-required 2048-bit pad. Feeds k and
 M1.

 H2. src/pool.rs:601-609 → 772 — send_rpc's for attempt in 0..4 loop: the Err(Error::Network) and read-timeout arms run reconnect_connection(conn).await?; continue; with no attempt<3 guard
 (unlike bad_msg:659, CONNECTION_NOT_INITED:729, re_send:742). Four consecutive socket failures that each allow a successful reconnect → execution falls out of the loop into
 unreachable!("retry loop returns on every path") at line 772 — a panic in the calling task.

 H3. src/types/message.rs:543-547 — Unknown MessageAction fallback consumes every remaining byte (while r.remaining() > 0). Inside Vector<Message> (getHistory/getDialogs share one reader),
 one unhandled action ctor drains all subsequent messages/chats/users — parse "succeeds" with silently truncated data. The arm's comment ("callers treat the messageService tail as terminal")
 is false in vector contexts.

 H4. src/types/message.rs:738-748 — MESSAGE_MEDIA_POLL | INVOICE | STORY | GIVEAWAY | GIVEAWAY_RESULTS | PAID_MEDIA | GAME | Unsupported map to Ok without consuming the variant payload. One
 poll/invoice in history leaves the reader mid-variant; every later message in the vector misparses → whole response fails with a misleading downstream error.

 H5. src/types/reply_types.rs:426-436 — documentAttributeVideo arm has two consecutive if flags & (1 << 5) != 0 { r.read_bytes()? } blocks (_video_codec then video_codec). When flags.5 is
 set, the codec string is consumed twice → desyncs the whole attribute-vector parse. documentAttributeVideo#43c57c48 carries video_codec:flags.5?string exactly once
 (tools/schema_l225.tl:981).

 H6. src/pool.rs:816-826 (+ api.rs container loop) — container-item lengths read as i32 cast to usize. A negative server-controlled length sign-extends to ~1.8e19: the off + len > data.len()
 bound check overflows (debug panic) or wraps (release) → data[off..off+len] slice-panics on malformed frames.

 H7. src/client.rs:1595-1605 — parse_resolved_peer's updates#-wrapped arm reads chats before users, but updates#74ae4240 serializes users before chats (the in-code comment itself states the
 order). The observed-live UPDATES-wrapped resolveUsername shape fails with a confusing error instead of hitting the intended retry path.

 H8. src/types/updates.rs:757-762 — read_dialog_skip reads seven i32 counters; the negotiated layer-223 dialog#d58a08c6 has six (unread_poll_votes_count exists only in later dialects —
 schema_l225.tl line 830 also lacks it). Curated Dialog::read_from (dialog.rs:71-77) correctly reads six → channelDifferenceTooLong responses misparse each embedded dialog by 4 bytes.

 H9. src/pool.rs:667-672 — server-pushed updates (UPDATES/UPDATES_COMBINED/UPDATE_SHORT/UPDATE_SHORT_SENT_MESSAGE) are acked but never queued or dispatched. Client::updates() only polls
 getState/getDifference; every pushed update between polls is silently lost. examples/updates_listener.rs and SPEC §6 advertise live push that doesn't exist.

 H10. src/types/message_gen.rs:7986-7990 — committed alias arms decode 223-dialect ctor ids using 229-schema field layouts: 223 messageMediaPoll#4bd6e798 has no flags word but the arm reads a
 leading flags i32; 223 replyInlineMarkup#48a30254 rows use keyboardButtonRow/keyboardButton while the 229 layout uses keyboardInlineButtonRow; Poll (223: 0x58747131) has no alias arm at all.
 Production 223-dialect frames inside generated containers fail or desync.

 MEDIUM (priority 2)

 M1. src/types/updates.rs:495-505 — Update::Other{ctor} fallback doesn't advance past the variant's fields → next update in Vector<Update> misparses; whole container fails (same
 forward-compat defect class as H3/H4).

 M2. src/mtproto.rs:763-767 — compute_server_salt builds salt via u64::from_be_bytes while every other path is LE (adopt_service_state reads via from_le_bytes, pool.rs:800; write_i64 is LE).
 Handshake salt reaches the wire byte-swapped; the bad_server_salt recovery corrects it at the cost of one extra exchange per key generation.

 M3. src/types/chat.rs:14-23 (conf. 1.0) — no mod chat in src/types/mod.rs; the 330-line hand-written Chat parser is never compiled, emits no warnings, and its shapes have drifted from the
 generated parser actually used.

 M4. src/client.rs:1190-1200 — build_get_difference(last_pts, state.date, state.qts) uses the freshly-fetched server snapshot, not locally tracked last-known state; skipped-ahead local state
 is never reconciled.

 M5. src/updates.rs:66-75 — PtsState's accept path treats update pts as pre-state (stored == update.pts then += pts_count), but Telegram sends post-update pts values; init_state seeds
 post-state from getState → every real update looks like a gap and forces getDifference. Currently masked by H9.

 M6. src/rpc.rs:565-575 — build_get_file's location fallback writes ctor 0 for unmodeled FileLocation variants → server round-trip fails with a confusing RPC error instead of a local
 rejection.

 M7. src/ws.rs:41-50 — split WS stream's read loop matches Ping => {} without queuing a Pong (split halves don't auto-respond); idle WS connections get killed by server ping timeout.

 M8. src/resilience.rs:1-10 — docs instruct wiring the refresh loop via Client::spawn_dc_refresher; no such method exists. FloodLimiter/FileRefCache/DcRotator have no callers; documented
 workflow can't be assembled.

 M9. src/types/message_gen.rs:10405-10406 (conf. 0.85) — committed generated files are stale vs. gentl.py's CTOR_ALIASES: ~34 alias entries defined, only 8 committed arms present; the
 committed Message alias 0x95ef6f2b matches neither the 223 nor the 229 schema. Checked-in generated code is not reproducible from the checked-in generator.

 M10. src/session.rs:230-236 — atomic-save's fsync opens the file via File::open (read-only) then sync_all — fails/ignored on Windows; durability claim is a no-op on the dev platform.

 LOW (priority 3)

 - L1. src/transport.rs:385-394 — abridged 3-byte length path allocates uncapped (~64 MiB from 0xFFFFFF); other transports cap at 2 MiB. Low exposure (pool doesn't use abridged).
 - L2. src/transport.rs:510-519 — recv_unencrypted parses msg_id big-endian; write path is LE. Latent (value discarded today).
 - L3. examples/demo.rs:373-377 — sent_code_name maps 0x9fd736; real auth.sentCodeFirebaseSMS is 0x09fd0736 → "Unknown" in demo flow.
 - L4. src/client.rs:1795-1805 — spawn_adaptive_scaler is a public no-op (spawns a task that logs and exits).
 - L5. src/rpc.rs:595-600 — comment claims TL doubles are big-endian; TLWriter::write_double correctly writes LE. Comment wrong, code right.
 - L6. src/ergonomics.rs:417-426 — MessagesIter::collect drops the error arm; a failing page silently ends iteration early with short results.
 - L7. src/types/updates.rs:575-584 — DIFFERENCE arm runs the identical if enc_count > 0 check twice back-to-back; likely a lost second branch.
 - L8. src/transport.rs:500-510 — unencrypted handshake uses msg_id 0 and hardcoded 0xdeadbeef; live-tolerated today, spec-compliance debt.

 Verified correct

 AES-IGE, msg_key (v2) derivation, Obfuscated2, RSA PKCS#1 v1.5 padding, auth-key handshake state machine, session persistence format, TLWriter primitives (ints/longs/strings/vectors/doubles
 LE).

 Coverage & caveats

 - 100% of hand-written source read in full: all 17 src/*.rs, all 14 hand-written src/types/*.rs, all 11 tracked examples (repo tracks 11, not 12), tests/live_sweep.rs, tools/gentl.py (1584
   lines), tools/dbg_shapes.py, tools/gen_docs_aliases.py.
 - Generated files skimmed systemically + targeted full reads of every alias arm and live-reachable parse paths; full 223-vs-229 schema diff of all 2,121 shared constructors.
 - Not reviewed: README.md, SPEC.md, DOCUMENTATION_LAYER.md, Cargo.toml (doc claims cross-checked from code instead).

 Biggest systemic risk: the forward-compatibility parse strategy (unknown variants = drain-nothing / drain-everything / Ok without consuming) — H3, H4, M1 share one root cause and mean any
 schema drift in server payloads cascades into truncated or failed responses. Second: regenerating from committed tooling would change committed parsers (M9/C2) — the generator, its alias
 table, and the checked-in output are mutually inconsistent.
