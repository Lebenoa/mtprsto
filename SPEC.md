# Telegram MTProto 2.0 — working reference

A condensed, exact spec for the parts mtprsto implements or needs to
implement. Layered bottom-up: transport → auth key → authorisation →
framing → RPC → updates. Each one must work correctly before the next
is built.

---

## 1. Transport

Telegram speaks MTProto over TCP. Three wire formats:

| Name        | Marker                              | Use                                       |
|-------------|-------------------------------------|-------------------------------------------|
| Abridged    | first byte `0xef`                   | Default. Small frames.                    |
| Intermediate| first byte `0xee` then 4 random    | 4-byte LE length.                         |
| Obfuscated2 | first 64 bytes random (no marker)   | AES-256-CTR stream; defeats DPI. Default in clients. |

**Abridged framing:**

```
┌──────────┬─────────────────────────────────────────────────┐
│ len (1)  │ payload (len bytes)                             │
└──────────┴─────────────────────────────────────────────────┘
```

If `len > 0x7f`, write `0x7f`, then 3 bytes LE.

**Obfuscated2 framing:**

First 64 bytes encode (LE):
- `[0..4]` random
- `[56..60]` `(dc_id & 2) | (protocol_tag << 4)` where
  `protocol_tag = 0xefefefef` (intermediate), else non-magic
  (abridged). Telegram rejects specific values.
- `[60..64]` `dc_id` LE.

The preamble's bytes `[8..40]` are the AES key, `[40..56]` are the IV
for the rest of the stream. Decrypt on receipt (mirror on send).

**Production DC endpoints (IPv4, port 443):**

| dc_id | address             | region            |
|-------|---------------------|-------------------|
| 1     | 149.154.175.50      | USA, Miami        |
| 2     | 149.154.167.50      | Amsterdam         |
| 3     | 149.154.175.100     | Miami             |
| 4     | 149.154.167.92      | Amsterdam         |
| 5     | 91.108.56.100       | Singapore         |
| 201  | 91.108.56.4         | test DC           |

`help.getConfig` returns the canonical `dc_option` list; clients should
hard-code the list above for the initial bootstrap only.

---

## 2. Auth key creation (Diffie-Hellman)

Uses RFC 3526 2048-bit MODP group 14:

```
p = 0xFFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1
    29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD
    EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245
    E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED
    EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE65381
    FFFFFFFF FFFFFFFF
g = 3
```

The client has Telegram's RSA public key(s) embedded as ASN.1
DER-encoded RSAPublicKey. Each modulus has a 64-bit fingerprint = CRC32
over the ASN.1 bytes. The client picks the key whose fingerprint matches
the server's `resPQ.fingerprints` list.

**Flow:**

```
client →  req_pq_multi#be7e8ef1 {nonce:int128}              (multi = req_pq with retries built in)
server →  resPQ#05162463 {nonce server_nonce pq:[int256]
                          server_public_key_fingerprints:[long]}
client:  p,q = factor pq
client →  req_DH_params#d712e4be {nonce server_nonce pq p q
                                   public_key_fingerprint long
                                   encrypted_payload:bytes}
         encrypted_payload = RSA(p_q_inner_data#83c95aec
                                  {pq p q nonce server_nonce new_nonce})
server →  server_DH_params#79cb045d {nonce server_nonce new_nonce_n
                                     server_nonce2 encrypted_answer}
         encrypted_answer = AES-IGE(new_nonce, payload) where
            payload = server_DH_inner_data#b0a48244
              {nonce server_nonce g dh_prime g_a server_time}
client:  verify aes_key = SHA-1(new_nonce + sub(pq, 0..124))[0:32]
         decrypt, parse server_DH_inner_data
         verify g_b matches: g_b must equal pow(g, b, dh_prime)
client:  b random, g_b = pow(g, b, dh_prime)
         auth_key = pow(g_a, b, dh_prime)
         auth_key_hash = SHA-1(auth_key)[0..20]
         (lowest 64 bits of auth_key_hash) == server_DH_inner_data.dh_hints
         → OK
client →  set_client_DH_params#f5045f1f {nonce server_nonce
                                          encrypted_data}
         encrypted_data = AES-IGE(derived_key,
            client_DH_inner_data#6643a746
              {nonce server_nonce g_a retry:0})
server →  dh_gen_ok#3cbc6d52 {nonce server_nonce new_nonce_n1 new_nonce_n2}
                                  (encrypted)
                                  (g_b hash == server's; else dh_gen_fail)
client:  decrypt, parse dh_gen_ok
         persist auth_key
```

**RSA-encrypted `p_q_inner_data`** is encrypted with the modulus's
public key (no padding needed; key size ≥ payload). Decrypted payload
starts with SHA-1 hash of the rest as a 20-byte prefix.

**AES-IGE for `req_DH_params` reply** uses key/iv derived from
`new_nonce`:

```
x = 0
sha1_a = SHA-1(new_nonce + substr(auth_key, x, 32))
sha1_b = SHA-1(substr(auth_key, 32 + x, 16) + new_nonce + substr(auth_key, 48 + x, 16))
sha1_c = SHA-1(substr(auth_key, 64 + x, 32))
sha1_d = SHA-1(new_nonce + substr(auth_key, 96 + x, 32))
aes_key = substr(sha1_a, 0, 8) + substr(sha1_b, 8, 12)
        + substr(sha1_c, 4, 12)
aes_iv  = sha1_a[8..20] + sha1_b[0..8] + sha1_c[20..28]
```

This is **different** from the AES-IGE used for normal message framing.

---

## 3. Authorisation

Once auth_key is shared with a DC, all subsequent traffic uses it.

### 3.1 User auth

```
client →  auth.sendCode#a8503291 {api_id api_hash phone_number settings}
server →  auth.sentCode#38edab53 {phone_code_hash type:
              auth.sentCodeTypeApp|Sms|Call|FlashCall|SmsCall
              phone_code_length? next_code_type? timeout?}

client →  auth.signIn#8d52a951 {phone_number phone_code_hash code}
server →  auth.authorization#cd050916 {user ...}
       or auth.authorizationSignUpRequired#4474f402 {terms_of_service}

if 2FA required:
client →  auth.checkPassword#d18b4d16 {password}
server →  auth.authorization

sign out:
client →  auth.logOut#5717da40
```

`settings` carries `{flags: int, allow_flashcall: Bool, current_number: Bool,
  allow_app_hash: Bool}`. Each flag bit enables a delivery channel.

### 3.2 Bot auth

```
client →  auth.importBotAuthorization#67a3ff2c
            {api_id api_hash bot_auth_token flags:int=0}
server →  auth.authorization
```

`bot_auth_token = "<dc_id>:<bot_token>"` where `bot_token` is the
`<bot_id>:<secret>` string from BotFather.

After auth, every subsequent request uses `(auth_key, session_id)`.

---

## 4. Message framing (MTProto 2.0)

```
┌──────────────────┬──────────────┬────────────────────────────────────────┐
│ auth_key_id (8)  │ msg_key (16) │ AES-IGE-encrypted payload              │
└──────────────────┴──────────────┴────────────────────────────────────────┘
```

`auth_key_id` = little-endian `u64` of `auth_key[6..14]`.

`msg_key` = `SHA-1(substr(auth_key, x, 32) || plaintext)[0..16]`.

- Client → server: `x = 0`
- Server → client: `x = 8`

### 4.1 AES key + IV derivation (MTProto 2.0)

```
k_part       = substr(auth_key, x, 128)
sha256_a     = SHA-256(k_part || substr(auth_key, 40+x, 16))
sha256_b     = SHA-256(substr(auth_key, 40+x, 16) || k_part || substr(msg_key, 0, 16))
aes_key      = substr(sha256_a, 0, 8) || substr(sha256_b, 8, 12)
              || substr(sha256_a, 16+4, 12)
aes_iv       = substr(sha256_a, 8, 12) || substr(sha256_b, 0, 8)
              || substr(sha256_a, 32, 4)
```

`plaintext = plaintext_body || random_padding` where padding makes total
length a multiple of 16. **Critical**: padding length must satisfy
`padding_len ∈ [12, 1024]` (random 12-1024 bytes, but convention is
12..1024 with 12-1024 random bytes — Telegram rejects anything outside
that range).

### 4.2 Plain (decrypted) payload

```
┌─────────┬────────────┬─────────┬─────────┬──────────────────────┬──────────┐
│ salt (8)│session(8)  │msg_id(8)│seq_no(4)│ msg_len (4) │ msg     │ padding │
└─────────┴────────────┴─────────┴─────────┴─────────────┴──────────┴──────────┘
```

`salt`: 8 B random; identifies a server-side session of the user.
Refreshed every ~30 min via `new_server_salt`.

`session_id`: 8 B random; client picks once per `(auth_key, dc)` pair.
Together with `auth_key`, this identifies the session.

`msg_id`: 8 B monotonically increasing per session. Must satisfy
`(msg_id mod 2^32) < (server_time mod 2^32)` (the low 32 bits are the
"timestamp"). Client generates it on every send.

`seq_no`: 4 B. Increments per logical message (containers count as 1
× message count). Even = content; odd = ack-only.
- First client→server msg: `seq_no = 0`.
- Subsequent: increment by 1 for content, 1 for ack-only.

`msg_len`: 4 B LE; size of the TL-encoded `msg`.

`msg`: TL-encoded object — either a request (e.g. `messages.sendMessage`)
or a `msg_container#73f1f8dc`.

---

## 5. RPC envelopes

### 5.1 Request wrapping

Plain request → just the TL object.

To bind to API layer:
```
invokeWithLayer#da9b0d0d {layer:int query:bytes}
```

To execute after a previous msg_id is ack'd:
```
invokeAfterMsg#cb9f372d {msg_id:long query:bytes}
```

To suppress updates:
```
invokeWithoutUpdates#bf94591b {query:bytes}
```

### 5.2 Server reply

All RPC replies are wrapped in:
```
rpc_result#f35c6d01 {req_msg_id:long result:rpc_result_body}
```

`result` is one of:

| ID      | Name                            | Notes                                |
|---------|---------------------------------|--------------------------------------|
| 0x997275b5 | rpc_result ok         | contains bare TL of the reply        |
| 0x2144ca19 | rpc_error             | error_code:int, error_message:string |
| 0x5e2b3f5d | rpc_answer_unknown    | retry                                |
| 0x6d2c0b28 | rpc_answer_dropped_running | retry                |
| 0xa7ad2a5f | rpc_answer_dropped    | retry with backoff                    |
| 0x34770c5a | bad_msg_notification  | client-side bug or stale salt        |
| 0x7d6c7d7f | new_server_salt       | salt rotation; persist new salt      |
| 0xd3e4caf7 | pong                  | reply to `ping`                      |

`bad_msg_notification.bad_msg_code`:
- 16 → msg_id too low
- 17 → msg_id too high
- 18 → bad msg_key
- 20 → salt invalidated
- 32, 33, 34, 48 → sequence issues
- 64 → invalid container
- 65 → not authorised (no auth_key)
- 96 → user banned / flood wait

### 5.3 Containers

```
msg_container#73f1f8dc {
  messages: vector#1cb5c415 {
    msg_id:long seq_no:int bytes:bytes
  }
}
```

Forwards the contained `msg_id`s under the container's `msg_id`. Server
processes serially; client's per-message `msg_id` and `seq_no` follow
the container's.

### 5.4 Acks

```
msgs_ack#62d6b459 {msg_ids:vector<long>}
```

Ack each message processed. Sent on overflow (16 pending) or every
~10 s, whichever comes first. Lower latency by ack'ing more eagerly.

`msgs_state_req#a4e4e162 {msg_ids:vector<long>}` for explicit state.

---

## 6. Updates

Telegram delivers real-time state changes as `Updates` objects:

```
Updates#ed74c4a4 {date:int seq:int updates:vector<Update> users:vector<User> chats:vector<Chat>}
UpdateShort#78d4dec1 {date:int seq:int update:Update}
UpdateShortMessage#c0123071 {date:int seq:int user_id:int message_id:int flags:int
                            message? entities? via_bot_id? reply_to? pts:int pts_count:int}
UpdateShortChatMessage#3d5d0a23 {chat_id:int user_id:int message_id:int date:int
                                message? entities? via_bot_id? reply_to? pts:int pts_count:int}
UpdatesCombined#725b04c3 {date:int seq:int updates:vector<Update> users chats seq_start:int}
UpdateShortSentMessage#11f101d3 {flags date id:int pts:int pts_count:int message? media? entities?}
```

`Update` enum covers the full event vocabulary. ii-drive needs (subset):

```
UpdateNewMessage           — wraps a Message in Updates.update
UpdateEditMessage
UpdateMessageID
UpdateDeleteMessages
UpdateReadHistoryInbox
UpdateReadHistoryOutbox
UpdateChannelTooLong       — resync needed
UpdateNewEncryptedMessage  — not used by bots
```

### 6.1 Sync semantics

Client tracks:
- `pts` per-account (for private chats, mentions)
- `pts` per-channel (for subscribed channels)
- `seq` per-account (for ordering)
- `date` (server clock at last update)

`pts` gap ⇒ call `updates.getDifference {pts date qts}` and re-apply.
`seq` gap ⇒ ditto.
`UpdateChannelTooLong` ⇒ call `updates.getChannelDifference {channel pts limit=...}`.

`qts` is for "Quick replies" / mentions — separate from `pts`.

```
updates.getState#edd4882a → {pts date seq qts unread_count}
updates.getDifference#25939104 → {updates or difference_empty:state or difference_too_long:pt}
updates.getChannelDifference#3173d78 → Updates, ChannelDifferenceEmpty, ChannelDifferenceTooLong
```

---

## 7. TL surface mtprsto must cover

For the swap (cf. ii-drive usage), mtprsto currently implements ~30
constructor IDs. The full required surface:

### Auth
- `auth.sendCode`                  0xa8503291
- `auth.signIn`                    0x8d52a951
- `auth.signUp`                    0x80eead27
- `auth.checkPassword`             0xd18b4d16
- `auth.logOut`                    0x87971c3d
- `auth.importBotAuthorization`    0x67a3ff2c
- `auth.exportLoginToken`          0x6a38e58f
- `auth.acceptLoginToken`          0x089f695c
- `auth.importLoginToken`          0x95ac5ce4

### Messages
- `messages.sendMessage`           0x44942323
- `messages.sendMedia`             0xb8d0afdf
- `messages.sendMultiMedia`        0xb6f3e0c0
- `messages.getDialogs`            0x19109d5f
- `messages.getHistory`            0xdc3f8240
- `messages.getMessages`           0x63c66506
- `messages.getBotCallbackAnswer`  0x934a4ee1
- `messages.deleteMessages`        0xe58e95c6
- `messages.deleteHistory`         0xb7e36194
- `messages.editMessage`           0x48f71768
- `messages.readHistory`           0x0e306d3a
- `messages.search`                0xd07bbf76

### Users / contacts
- `users.getFullUser`              0xe0b917f2
- `users.getUsers`                 0x0d91a548
- `contacts.resolveUsername`       0xf93ccba3
- `contacts.resolvePhone`          0x8af2a521
- `contacts.search`                0x11f812d8

### Channels
- `channels.createChannel`         0x3d5d10fd
- `channels.inviteToChannel`       0x199f3a6c
- `channels.editAdmin`             0x70d896ff
- `channels.getChannels`           0xa7f6d76b
- `channels.getParticipants`       0x123ffe12
- `channels.editAbout`             0x13e27b46
- `channels.leaveChannel`          0xf836aa28

### Updates
- `updates.getState`               0xedd4882a
- `updates.getDifference`          0x25939104
- `updates.getChannelDifference`   0x3173d78

### Upload / files
- `upload.saveFilePart`            0xb304a621
- `upload.saveBigFilePart`         0xde7b673d
- `upload.getFile`                 0xb3e7e951
- `upload.getWebFile`              0x24e5e54e
- `upload.saveFile`                0x96f18c5e (used by sendMedia)
- `upload.getCdnFile`              0x572f9519

### Help
- `help.getConfig`                 0xc4f3926c
- `help.getNearestDc`              0x1fb33026

### Photos
- `photos.updateProfilePhoto`      0x1c3c2a85
- `photos.uploadProfilePhoto`      0x4f32c098
- `photos.deletePhotos`            0x87cf7f2f
- `photos.getUserPhotos`           0x91cd32a8

### Required reply types
- `auth.{SentCode, Authorization, AuthorizationSignUpRequired, SentCodeTypeApp|Sms|...}`
- `User{User, UserEmpty}`, `Chat{Chat, ChatEmpty, Channel, ChannelForbidden}`, `ChatFull`, `ChannelFull`
- `Message{Message, MessageEmpty, MessageService}`, `MessageMedia{Photo, Document, Unsupported, ...}`, `MessageEntity`
- `Photo`, `PhotoSize{Empty, Size, CachedSize, StrippedSize}`, `Document`, `DocumentAttribute*`
- `Updates`, `Update*` (per §6)
- `Dialog`, `DialogFolder`, `MessagesDialogs`, `ChatParticipants`
- `MessagesBotCallbackAnswer`
- `InputPeer*`, `InputUser*`, `InputChannel*`, `InputFile*`
- `ReplyMarkup`, `KeyboardButton*`, `KeyboardButtonRow`
- `ChatAdminRights`, `ChatBannedRights`

---

## 8. Bots

`api.telegram.org/bot<token>/...` is the **HTTP** Bot API — irrelevant
for MTProto client work. Bots via MTProto authenticate with
`auth.importBotAuthorization` (see §3.2) and use the full RPC surface.
Rate limits: 30 req/s globally, 1 req/s per method for some endpoints.

---

## 9. Constants cheat-sheet

| Item                   | Value                                         |
|------------------------|-----------------------------------------------|
| DH prime `p`           | RFC 3526 2048-bit MODP group 14               |
| `g`                    | 3                                             |
| `auth_key` size        | 2048 B                                        |
| `msg_key` size         | 16 B                                          |
| `salt` size            | 8 B                                           |
| `session_id` size      | 8 B                                           |
| `msg_id` size          | 8 B                                           |
| `seq_no` size          | 4 B                                           |
| AES block              | 16 B                                          |
| IGE: key / iv          | 32 / 32                                       |
| Padding length         | random ∈ [12, 1024]; total = 16-byte aligned  |
| `salt` validity        | ~30 min; server rotates via `new_server_salt` |
| `msg_id` validity      | reject future > 30 s, past > 300 s            |
| Acks per second        | ≥1 per 10 s, or queue ≥16 pending             |
| Container `msg_id`s    | must be ≥ parent msg's seq                    |
| API layer (2026)       | 218 (mtprsto ships with 175; bump on connect) |

---

## 10. References

- core.telegram.org/mtproto — official protocol docs
- core.telegram.org/mtproto/description — TL schema
- core.telegram.org/mtproto/mtproto-transports — transport details
- TL schema JSON: `https://core.telegram.org/schema/json`
- TL constructor ID = `CRC32` of the description string (RFC 1952,
  i.e. with the IEEE polynomial and final XOR — Telegram uses the
  standard CRC-32 IEEE 802.3 one).
---

## 11. Gap analysis — mtprsto today vs ii-drive needs

This section is maintained alongside the spec because it dictates the
order in which mtprsto grows. It maps every Telegram surface ii-drive
uses against what mtprsto currently ships. "Today" reflects
`S:/Data/CODE/Rust/mtprsto` as of the spec write.

### 11.1 mtprsto surface today (≈3,100 LoC)

| Module       | LoC | What it does                                                        |
|--------------|-----|---------------------------------------------------------------------|
| `crypto.rs`  | 319 | AES-IGE, SHA-1/256, MD5, RSA (modulus + fingerprint), CRC32        |
| `serialize.rs`| 415 | `TLWriter`/`TLReader` (i32/i64/u128/u256/bytes), `constructor_id`   |
| `transport.rs`| 319 | Abridged + Obfuscated2; DC IP table                                |
| `mtproto.rs` | 792 | `MtProtoSession`: DH handshake, msg framing, ping/pong, container, `parse_rpc_result` |
| `api.rs`     | 478 | ~30 constructor ID constants + `serialize_input_peer_user`/string |
| `error.rs`   | 60  | Io/Crypto/Serialization/Transport/Protocol/Api/DhVerification/UnexpectedResponse/NoAuthKey/Padding |
| `main.rs`    | 270 | CLI binary (user auth, bot auth, demo)                              |
| `lib.rs`     | 6   | Module exports                                                     |

**Public APIs actually wired up:**
`MtProtoSession::{auth_send_code, sign_in, sign_up, send_message, get_me, get_dialogs, get_state, get_difference, bot_sign_in}` — all via raw TL bytes through the transport. No high-level client, no file upload/download, no TL type library, no update stream, no session persistence.

### 11.2 ii-drive surface in use (≈3,500 LoC of MTProto logic)

| File                                  | LoC | What it does                                                                 |
|---------------------------------------|-----|------------------------------------------------------------------------------|
| `tg/mod.rs`                           | 276 | `TgManager`/`Conn` core, `SenderPoolHandle` lifecycle, `AUX_UPLOAD_POOLS=3` |
| `tg/session.rs`                       | 338 | `open_conn` (SqliteSession + SenderPool + aux copies), `ensure()`, `avatar()` |
| `tg/transfer.rs`                      | 167 | `upload_stream_parallel`, `send_message(InputMessage::text().document())`, thumb extract, `delete_messages`, `is_file_reference_error` |
| `tg/bots.rs`                          | 328 | `configure_bot` (bot_sign_in), `pool_target` round-robin, `add_bots_to_chat`, raw `channels.InviteToChannel`/`EditAdmin`/`GetChannels` |
| `tg/botfather.rs`                     | 287 | `botfather_send`, `markup_buttons`, `last_buttons`, `await_reply_buttons` (iter_messages poll), `press_botfather_callback` (getBotCallbackAnswer) |
| `tg/channels.rs`                      | 264 | `storage_peer`, `list_channels` (GetDialogs), `create_channel`, `harvest_chats`, `input_peer_for` |
| `tg/hub.rs`                           | 889 | login orchestration, session scan/move/remove, throttling, periodic health check |
| `tg/login.rs`                         | 173 | `send_code`, `sign_in` (`LoginToken`/`PasswordToken`), `SignInError` variants, `check_password` |
| `stream.rs`                           | 307 | `file_stream_from` (`get_messages_by_id` + `iter_download` chunked + skip_chunks + reference-expiry resume), `parts_stream_from` |
| `routes/files/upload.rs`              | 745 | `upload_file` / `spill_upload`, `PartPlan` (TG_DOC_CAP=4000·512 KiB, max 64 parts) |
| `routes/files/resume.rs`              | 569 | resumable upload `init()` spawning one uploader task per part → `tg.upload()`, transient retry |

Total: 4,343 LoC across these files (some lines are local-IO, not all
pure MTProto).

### 11.3 Gap table (priority-ordered)

| # | Surface                                           | mtprsto today | Required by                                                  | Priority |
|---|---------------------------------------------------|---------------|--------------------------------------------------------------|----------|
| 1 | TL type library                                   | none          | every call needs typed args/returns                          | **P0**   |
| 2 | `SenderPool` (multi-conn, aux copies, reconnect)  | none          | `tg/session.rs` open_conn, `tg/transfer.rs` parallel upload  | **P0**   |
| 3 | File upload (`upload.saveBigFilePart`)            | none          | `transfer.rs` + `upload.rs` + `resume.rs`                     | **P0**   |
| 4 | File download (`upload.getFile` + iter)           | none          | `stream.rs`                                                  | **P0**   |
| 5 | High-level `Client` (connect, send_message, ...)  | none          | every consumer                                              | **P0**   |
| 6 | SQLite session store                              | none          | `tg/session.rs` (must read existing files written by `grammers_session 0.10`) | **P0**   |
| 7 | Update stream + `iter_messages`                   | none          | `tg/botfather.rs` (poll for BotFather reply)                  | **P1**   |
| 8 | Callback query (`getBotCallbackAnswer`)           | none          | `tg/botfather.rs` (press inline button)                      | **P1**   |
| 9 | Channel admin (CreateChannel, Invite, EditAdmin)  | none          | `tg/bots.rs`, `tg/channels.rs`                               | **P1**   |
| 10| `messages.deleteMessages`                         | none          | `tg/transfer.rs` cleanup                                     | **P1**   |
| 11| Profile photos iter (`photos.getUserPhotos`)      | none          | `tg/session.rs::avatar()`                                    | **P2**   |
| 12| `iter_messages` pagination / `getHistory`         | none          | `tg/botfather.rs` history-scan fallback                      | **P2**   |
| 13| Error type extensions (`FloodWait`, `FileReferenceExpired`, `AuthKeyInvalid`) | partial | `is_auth_error`, `is_file_reference_error`, retry loops | **P1**   |
| 14| Reply-markup parsing (ButtonRow, Callback data)   | none          | `tg/botfather.rs::markup_buttons`                            | **P1**   |
| 15| WebSocket transport (fallback for blocked regions)| none          | resilience; not blocking if TCP/Obfs2 path stays open        | **P3**   |

**P0** = required for any migration. **P1** = required for full
feature parity. **P2** = nice-to-have, can ship without. **P3** =
backlog.

### 11.4 Sub-surfaces that must be designed together

A few gaps interlock — designing them in isolation will produce an
unusable API. They must be designed as a unit:

- **(2, 5, 13) SenderPool + Client + Error.** The Client's `invoke` is
  the only entry point; its error type decides what retry logic can
  express. `SenderPoolHandle.thin()` semantics (cloning, dropped handle
  semantics) gate the aux-pool pattern in `tg/mod.rs::Conn`.

- **(1, 8, 14) Types + Callback + ReplyMarkup.** `getBotCallbackAnswer`
  takes raw bytes (`data`); the markup-side types must serialise the
  same bytes back. Both must round-trip against the same `Message`
  enum.

- **(3, 4, 13) Upload/Download + Error.** `FileReferenceExpired` error
  on `iter_download` triggers a re-fetch of the source message in
  `stream.rs`. Without the error variant, the resume path in
  `stream.rs::file_stream_from` cannot work.

- **(1, 6, 9) Types + Session + ChannelAdmin.** `InputChannel` and
  `InputPeer` need `access_hash`. Without session persistence of
  `access_hash`, channel admin must call `channels.getChannels` every
  time, which adds 1 RTT per admin op.

### 11.5 Order of work (suggested)

1. **TL type library** (`src/types.rs`, ~3–5 kLoC) — gates everything.
2. **Session persistence** (`src/session.rs`, ~400 LoC) — needed
   before Client.
3. **SenderPool** (`src/pool.rs`, ~800 LoC) — the throughput-critical
   bit.
5. **High-level Client** (`src/client.rs`, ~1.5 kLoC) — composes all
   of the above.
7. Tests + benchmark vs grammers throughput.

Then migrate ii-drive per file (`tg/session.rs` first, then
`tg/transfer.rs`, `tg/bots.rs`, `tg/botfather.rs`, `tg/channels.rs`,
`tg/login.rs`, `tg/mod.rs`, `stream.rs`, `routes/files/{upload,
resume}.rs`). Drop `grammers-client`/`grammers-session` deps and
verify with a real-bot smoke test.

### 11.6 Open risks

| Risk                                                         | Mitigation |
|--------------------------------------------------------------|------------|
| Session sqlite schema mismatch → every bot re-auths          | Reverse-engineer `grammers_session 0.10` schema exactly; verify by reading an existing `data/sessions/*.sqlite`. |
| Throughput regression vs grammers' SenderPool                | Benchmark before migration; gate at end of step 4. |
| `file_reference` expiry handling diverges from grammers      | Match grammers' error-text matching (`FILE_REFERENCE_EXPIRED`) byte-for-byte in `error.rs`. |
| DC IP table stale → all initial connects fail                | Refresh from `help.getConfig` on every boot, fall back to hard-coded table. |
| API layer mismatch (mtprsto 175 vs current ~218)              | Bump on `Client::connect` via `invokeWithLayer`. |
| Updates channel only delivers when the main client pumps     | Replicate grammers' background "runner" task exactly; leak it past `Client::drop` until subscribers finish. |
---


## 12. Improvements over grammers

grammers is the baseline; mtprsto is **not** a re-implementation but a
successor that fixes the parts that hurt day-to-day use. Each axis
below names what grammers does today, why it's painful, and what
mtprsto replaces it with. Every behaviour is configurable through the
`ClientConfig` builder — the defaults match the recommendation, but
operators can opt out.

### 12.1 Developer experience

**DX-1: API ergonomics.**

grammers today:

- `client.send_message(peer, InputMessage::text())`
- `InputMessage::text().document().mime_type(...)`
- `client.iter_messages().limit(10).await?`
- `client.invoke::<Updates, _>(req_bytes).await?`

mtprsto replaces with:

- `client.send(peer, "text").await?`
- `client.send_file(peer, path).caption("hi")`
- `client.messages(peer).take(10).collect().await?`
- `client.invoke(req).await?` (typed returns)

Builder-free wherever the use is common; builder API exists for the
edges (e.g. `client.send(peer, "hi").reply_to(id).silent().await?`).
Every method that can fail returns `Result<T, Error>` with a typed
error variant (see DX-2).

**DX-2: Typed errors + good `Display`.**

| Error variant            | When                                            |
|--------------------------|-------------------------------------------------|
| `Rpc(i32, String)`       | any 4xx/5xx reply; code is Telegram's            |
| `FloodWait { seconds, retry_after: Instant }` | `FLOOD_WAIT_X` reply |
| `FileReferenceExpired`   | `FILE_REFERENCE_EXPIRED` (auto-refreshed, see BS-3) |
| `AuthKeyInvalid` / `AuthKeyUnregistered` | first-sign-error or key removed |
| `InvalidPassword` / `PasswordRequired` / `SignUpRequired` | `auth.signIn` reply |
| `InvalidCode`            | `PHONE_CODE_INVALID`                            |
| `Network`                | I/O, timeout, disconnect (with cause chain)     |
| `Migration { dc_id }`    | server says "move to DC N"                      |
| `Other(String)`          | fallback only; never for known cases            |

`Display` impls follow `{kind}: {detail} [dc=N key=0x{short}]` so
logs are searchable. `std::error::Error::source()` returns the inner
I/O / RPC cause for `Error::Network(_)`. `is_transient()` returns
`true` for `FloodWait`, `Network`, `FileReferenceExpired` so retry
loops are one-liners:

```rust
for attempt in 0..MAX {
    match op().await {
        Ok(v) => return Ok(v),
        Err(e) if e.is_transient() => tokio::time::sleep(backoff(attempt)).await,
        Err(e) => return Err(e),
    }
}
```

**DX-3: Tracing + observability.**

Every RPC wrapped in `#[tracing::instrument(skip_all, fields(dc_id,
msg_id, method, latency))]`. Each `Client` owns a `tracing::Span`
named `"mtprsto::session"` carrying `auth_key_id` (truncated, never
the full key), `dc_id`, `user_id`, `bot_or_user`. The Span is entered
on every request and update. Events:

- `info!` on `Client::connect` success / disconnect
- `warn!` on `Network` errors with retry counter
- `debug!` per RPC (`method=...`, `msg_id=...`, `latency=...`)
- `trace!` per message byte round-trip

Disabled when the `tracing` feature is off. No `println!` calls
anywhere.

**DX-4: Docstrings + examples.**

Every public item has:

- A one-line summary
- `# Arguments` / `# Returns` only when non-obvious
- `# Errors` listing the variants of `mtprsto::Error` it can produce
- `# Example` with a runnable doctest (≥1 per major API)
- `# Panics` if any

`cargo test --doc` runs all examples; CI fails on any panic or
unwrap. Module-level docs include a 20-line getting-started that
compiles.

**DX-5: Type safety.**

All IDs are newtypes, not raw integers:

```rust
pub struct UserId(pub i64);
pub struct ChannelId(pub i64);
pub struct AccessHash(pub i64);
pub struct MsgId(pub i32);
pub struct ChatId(pub i64);  // distinct from UserId even though both i64
```

`From` impls accept raw integers for ergonomics at call sites. Cross-
type assignment is a compile error: `let _: UserId = some_channel_id`
fails. `access_hash` cannot be passed where a `user_id` is expected.
No `unsafe` anywhere.

**DX-6: Builder pattern for messages.**

```rust
client.message(peer, "hi")
    .reply_to(msg_id)
    .silent()
    .no_preview()
    .send()
    .await?;
```

Lazy: nothing is sent until `.send().await`. `.into_input_message()`
exposes the lower-level builder for cases the high-level surface can't
express (rare). Builders are `Send` and own their `Client` via `Arc`,
so cloning the `Client` is cheap.

### 12.2 Behind the scenes

Each behind-the-scenes feature has a config knob in `ClientConfig`. The
default matches the recommendation; operators can disable or tune.

**BS-1: Adaptive connection pool.** (knob: `pool: PoolConfig`)

- Default: 1 main + 3 aux per DC; scales up to 8 aux if
  `inflight > 2 × aux_count` for more than 10 s; scales back down
  after 60 s of low load.
- Pin connections to TCP keepalive (every 30 s) so the kernel
  doesn't drop them.
- Exponential backoff on reconnect: 1 s, 2 s, 4 s, ... capped at 60 s,
  jitter ±20 %.
- Detect silent disconnects (no pong in 90 s) and reconnect with the
  same auth_key.
- Each connection owned by a single `tokio` task; requests are
  load-balanced across connections inside a connection's RPC inbox
  (grammers' bottleneck).
- WebSocket transport as fallback when TCP connect fails twice on a
  DC (`transport: TransportPolicy` knob).

**BS-2: Flood-wait handling.** (knob: `rate_limit: RateLimitConfig`)

- On `FLOOD_WAIT_X` reply, schedule the **method** (not the whole
  client) to be paused for `X` seconds.
- Subsequent calls to that method return `Error::FloodWait { seconds,
  retry_after }` without hitting the wire.
- Per-method, per-DC buckets (a flood-wait on DC 2 doesn't pause DC 5).
- Reset window on success.
- Knobs: `enabled: bool` (default on), `max_wait: Duration` (default
  30 min cap), `global_pause: bool` (default off; some operators
  prefer a global pause).

**BS-3: File reference auto-refresh.** (knob: `file_ref: FileRefConfig`)

- Track `file_reference` per Document in an in-memory LRU (default
  capacity 10k entries).
- On `iter_download` returning `FileReferenceExpired`, transparently
  re-fetch the source `Message` via `messages.getMessages`, retry
  with the new reference. Up to `max_retries: u8` (default 3) per
  chunk.
- Same for `upload.send_document` when the message being replied to
  has expired references. Off by default (knob) for users who want
  to handle it themselves.

**BS-4: Update dispatch model.** (knob: `updates: DispatchMode`)

- Default `DispatchMode::Channel`: every update is published into an
  `mpsc::UnboundedSender<Update>` that the `Client` owns; users call
  `let mut rx = client.updates()` and select on it.
- `DispatchMode::Handler`: register
  `client.on_update(|upd| async { ... })`. Internally a tokio task
  drives the dispatcher.
- No polling — updates arrive via the same connection that sent the
  RPC, deduped by `msg_id` / `pts`.
- `iter_messages(peer)` is a thin wrapper: drains updates between
  now and the requested history depth, then yields
  `messages.getHistory` results. No background thread per
  `iter_messages`.

**BS-5: Multi-connection download.** (knob: `download: DownloadConfig`)

- For files larger than `parallel_threshold: u64` (default 8 MiB),
  split the byte range across `parallel_count` chunks (default 4)
  and fetch them concurrently across aux connections.
- Reassemble in-order; surface the first error from any chunk.
- Below threshold: serial download (lower overhead).
- Off (`parallel_count = 1`) for users with low aux pool counts.

**BS-6: DC rotation + transport fallback.** (knob: `dc: DcConfig`)

- Background task polls `help.getConfig` every
  `config_refresh: Duration` (default 1 h) and updates the DC IP
  table.
- On `Migration { dc_id }` reply, transparently redirect to the new
  DC and persist the move.
- WebSocket transport (`wss://...`) used when
  `TransportPolicy::Auto` and TCP connect fails twice on the same DC
  within 5 min. Falls back to TCP/Obfs2 on subsequent success.
- All of the above are off by default (`TransportPolicy::TcpOnly`,
  `config_refresh: None`, etc.) so operators can opt in per their
  region.

### 12.3 What mtprsto explicitly does NOT improve

To keep scope honest:

- **TL type library size.** grammers' `grammers-tl-types` is generated
  from the schema; mtprsto's `types.rs` is hand-written for the ~20
  constructors ii-drive uses. We do **not** generate the full TL
  layer — generating the full schema is a separate project. The
  hand-written surface is enough for the migration; new constructors
  are added on demand.
- **Self-hosted schema definitions.** Telegram sometimes rolls layer
  bumps without notice; mtprsto hard-codes the layer at the version
  used at compile time. Operators running long-lived deployments must
  rebuild for new layers (matches grammers' behaviour).
- **Backward compatibility with grammers sessions.** The new session
  file format is incompatible. Migration is one-shot: read existing
  grammers sessions, write mtprsto sessions on first connect.
  Documented in `MIGRATION.md`.

### 12.4 Config summary

```rust
ClientConfig::builder()
    .api_id(12345)
    .api_hash("...")
    .session(path)
    .pool(PoolConfig::default())           // BS-1
    .rate_limit(RateLimitConfig::default()) // BS-2
    .file_ref(FileRefConfig::default())     // BS-3
    .updates(DispatchMode::Channel)         // BS-4
    .download(DownloadConfig::default())    // BS-5
    .dc(DcConfig::default())                // BS-6
    .tracing(true)
    .build()?;
```

Every default is the recommended setting; every setting can be
overridden by operators.