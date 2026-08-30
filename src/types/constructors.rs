//! TL constructor ID constants (Layer 223).

#[allow(unused_imports)]
use std::fmt;

// §7 Constructor IDs
// ===========================================================================

// --- Input peer (Layer 223) ---
pub const INPUT_PEER_EMPTY: u32 = 0x7f3b18ea;
pub const INPUT_PEER_SELF: u32 = 0x7da07ec9;
pub const INPUT_PEER_USER: u32 = 0xdde8a54c;
pub const INPUT_PEER_USER_FROM_ID: u32 = 0xa87b0a1c;
pub const INPUT_PEER_CHAT: u32 = 0x35a95cb9;
pub const INPUT_PEER_CHANNEL: u32 = 0x27bcbbfc;
pub const INPUT_PEER_CHANNEL_FROM_ID: u32 = 0xbd2a0840;

// --- Input user (Layer 223) ---
pub const INPUT_USER_EMPTY: u32 = 0xb98886cf;
pub const INPUT_USER_SELF: u32 = 0xf7c1b13f;
pub const INPUT_USER: u32 = 0xf21158c6;
pub const INPUT_USER_FROM_ID: u32 = 0x1da448e2;

pub const INPUT_REPLY_TO_MESSAGE: u32 = 0x869fbe10;
// --- Input channel (Layer 223) ---
pub const INPUT_CHANNEL: u32 = 0xf35aec28;
pub const INPUT_CHANNEL_FROM_MESSAGE: u32 = 0x5b934f9d; // inputChannelFromMessage

// --- Input file (Layer 223) ---
pub const INPUT_FILE: u32 = 0xf52ff27f;
pub const INPUT_FILE_BIG: u32 = 0xfa4f0bb5;
pub const INPUT_FILE_STORY_DOCUMENT: u32 = 0x62dc8b48;

// --- Input document (Layer 223) ---
pub const INPUT_DOCUMENT: u32 = 0x1abfb575;
pub const INPUT_DOCUMENT_EMPTY: u32 = 0x72f0eaae; // inputDocumentEmpty
/// `document#8fd4c4d8 flags:# id:long access_hash:long file_reference:bytes
///  date:int mime_type:string size:long thumbs:flags.0?Vector<PhotoSize>
///  video_thumbs:flags.1?Vector<VideoSize> dc_id:int
///  attributes:Vector<DocumentAttribute>`
pub const DOCUMENT: u32 = 0x8fd4c4d8; // document#8fd4c4d8 (Layer 223)
/// documentEmpty#3631cf4c id:long
pub const DOCUMENT_EMPTY: u32 = 0x36f8c871;

// --- User (Layer 223) ---
pub const USER: u32 = 0x31774388;
pub const USER_EMPTY: u32 = 0xd3bc4b7a;

// --- User status (Layer 223) ---
// emojiStatus#e7ff068a / emojiStatusEmpty#2de11aae
pub const EMOJI_STATUS: u32 = 0xe7ff068a;
pub const EMOJI_STATUS_EMPTY: u32 = 0x2de11aae;
/// `recentStory#711d692d flags:# live:flags.0?true max_id:flags.1?int`
pub const RECENT_STORY: u32 = 0x711d692d;
/// `peerNotifySettings#99622c0c`
pub const PEER_NOTIFY_SETTINGS: u32 = 0x99622c0c;
/// `notificationSoundDefault#97e8bebe` / `None#6f0c344c` /
/// `Local#830b9ae9` / `Ringtone#50640d3d`
pub const NOTIFICATION_SOUND_DEFAULT: u32 = 0x97e8bebe;
pub const NOTIFICATION_SOUND_NONE: u32 = 0x6f0c34df;
pub const NOTIFICATION_SOUND_LOCAL: u32 = 0x830b9ae9;
pub const NOTIFICATION_SOUND_RINGTONE: u32 = 0x50640d3d;
pub const USER_STATUS_EMPTY: u32 = 0x9d05049;
pub const USER_STATUS_ONLINE: u32 = 0xedb93949;
pub const USER_STATUS_OFFLINE: u32 = 0x8c703f;
pub const USER_STATUS_RECENTLY: u32 = 0x7b197dc8;
pub const USER_STATUS_LAST_WEEK: u32 = 0x541a1d1a;
pub const USER_STATUS_LAST_MONTH: u32 = 0x65899777;

// --- Chat (Layer 223) ---
pub const CHAT: u32 = 0x41cbf256;
pub const CHAT_EMPTY: u32 = 0x29562865;
pub const CHAT_FORBIDDEN: u32 = 0x6592a1a7;
pub const CHAT_FULL: u32 = 0x2633421b;

// --- Channel (Layer 223) ---
/// `channel#d49f34c6` — live layer 225 (old 0x1c32b11c stale).
pub const CHANNEL: u32 = 0xd49f34c6;
/// `updateChannel#635b4c09 channel_id:long`
pub const UPDATE_CHANNEL: u32 = 0x635b4c09;
/// `updateReadChannelInbox#922e6e10 flags:# folder_id:flags.0?int
///   channel_id:long max_id:int still_unread_count:int pts:int`
pub const UPDATE_READ_CHANNEL_INBOX: u32 = 0x922e6e10;
/// `updateNewChannelMessage#62ba04d9 message:Message pts:int pts_count:int`
pub const UPDATE_NEW_CHANNEL_MESSAGE: u32 = 0x62ba04d9;
/// `updateEditChannelMessage#1b3f4df7 message:Message pts:int pts_count:int`
pub const UPDATE_EDIT_CHANNEL_MESSAGE: u32 = 0x1b3f4df7;
/// `updateDeleteChannelMessages#c32d5b12 channel_id:long messages:Vector<int>
///   pts:int pts_count:int`
pub const UPDATE_DELETE_CHANNEL_MESSAGES: u32 = 0xc32d5b12;
/// `updateReadChannelOutbox#b75f99a9 channel_id:long max_id:int`
pub const UPDATE_READ_CHANNEL_OUTBOX: u32 = 0xb75f99a9;
pub const CHANNEL_FORBIDDEN: u32 = 0x17d493d5;

// --- Photo/UserProfilePhoto/ChatPhoto (Layer 223) ---
pub const PHOTO_EMPTY: u32 = 0x2331b22d;
pub const PHOTO: u32 = 0xfb197a65;
pub const CHAT_PHOTO: u32 = 0x1c6e1c11;
pub const CHAT_PHOTO_EMPTY: u32 = 0x37c1011c;
pub const USER_PROFILE_PHOTO: u32 = 0x82d1f706;
pub const USER_PROFILE_PHOTO_EMPTY: u32 = 0x4f11bae1;

// --- Message (Layer 223) ---
pub const MESSAGE: u32 = 0x3ae56482;
// Live layer 225 re-issued the message ctor (wire-verified against
// production DCs 2026-08; core.telegram.org's published JSON still lists
// the old ID). Both are accepted when parsing.
pub const MESSAGE_V225: u32 = 0x95ef6f2b;
pub const MESSAGE_EMPTY: u32 = 0x90a6ca84;
pub const MESSAGE_SERVICE: u32 = 0x7a800e0a;
// messageReplyHeader#6917560b
pub const MESSAGE_REPLY_HEADER: u32 = 0x6917560b;
// Live layer 225 re-issued the reply-header ctor (wire-verified; same
// field layout). Both IDs are accepted when parsing.
pub const MESSAGE_REPLY_HEADER_V225: u32 = 0x1b97dd66;
/// `messageReplyStoryHeader#0e5af939 peer:Peer story_id:int`
pub const MESSAGE_REPLY_STORY_HEADER: u32 = 0x0e5af939;

// --- Message media (Layer 223) ---
pub const MESSAGE_MEDIA_EMPTY: u32 = 0x3ded6320;
/// `messageMediaPhoto#e216eb63` (live layer 225; old 0x695150d7 stale)
pub const MESSAGE_MEDIA_PHOTO: u32 = 0xe216eb63;
/// `geoPoint#b2a2f663 flags:# long:double lat:double access_hash:long
///   accuracy_radius:flags.0?int` / `geoPointEmpty#1117dd5f`
pub const GEO_POINT: u32 = 0xb2a2f663;
pub const GEO_POINT_EMPTY: u32 = 0x1117dd5f;
pub const MESSAGE_MEDIA_DOCUMENT: u32 = 0x52d8ccd9;
pub const MESSAGE_MEDIA_WEB_PAGE: u32 = 0xddf10c3b;
pub const MESSAGE_MEDIA_GEO: u32 = 0x56e0d474;
pub const MESSAGE_MEDIA_CONTACT: u32 = 0x70322949;
pub const MESSAGE_MEDIA_DICE: u32 = 0x08cbec07;
pub const MESSAGE_MEDIA_UNSUPPORTED: u32 = 0x9f84f49e;
pub const MESSAGE_MEDIA_GAME: u32 = 0xfdb19008;
pub const MESSAGE_MEDIA_POLL: u32 = 0x4bd6e798;
pub const MESSAGE_MEDIA_INVOICE: u32 = 0xf6a548d3;
pub const MESSAGE_MEDIA_STORY: u32 = 0x68cb6283;
pub const MESSAGE_MEDIA_GIVEAWAY: u32 = 0xaa073beb;
pub const MESSAGE_MEDIA_GIVEAWAY_RESULTS: u32 = 0xceaa3ea1;
pub const MESSAGE_MEDIA_PAID_MEDIA: u32 = 0xa8852491;
// --- Message entities (Layer 223, verified 2026-08) ---
pub const MESSAGE_ENTITY_UNKNOWN: u32 = 0xbb92ba95;
pub const MESSAGE_ENTITY_MENTION: u32 = 0xfa04579d;
pub const MESSAGE_ENTITY_HASHTAG: u32 = 0x6f635b0d;
pub const MESSAGE_ENTITY_BOT_COMMAND: u32 = 0x6cef8ac7;
pub const MESSAGE_ENTITY_URL: u32 = 0x6ed02538;
pub const MESSAGE_ENTITY_EMAIL: u32 = 0x64e475c2;
pub const MESSAGE_ENTITY_BOLD: u32 = 0xbd610bc9;
pub const MESSAGE_ENTITY_ITALIC: u32 = 0x826f8b60;
pub const MESSAGE_ENTITY_UNDERLINE: u32 = 0x9c4e7e8b;
pub const MESSAGE_ENTITY_STRIKE: u32 = 0xbf0693d4;
pub const MESSAGE_ENTITY_CODE: u32 = 0x28a20571;
pub const MESSAGE_ENTITY_PRE: u32 = 0x73924be0;
pub const MESSAGE_ENTITY_TEXT_URL: u32 = 0x76a6d327;
pub const MESSAGE_ENTITY_MENTION_NAME: u32 = 0xdc7b1140;
pub const MESSAGE_ENTITY_PHONE: u32 = 0x9b69e34b;
pub const MESSAGE_ENTITY_CASHTAG: u32 = 0x4c4e743f;
pub const MESSAGE_ENTITY_SPOILER: u32 = 0x32ca960f;
pub const MESSAGE_ENTITY_CUSTOM_EMOJI: u32 = 0xc8cf05f8;
pub const MESSAGE_ENTITY_BLOCKQUOTE: u32 = 0xf1ccaaac;
pub const MESSAGE_ENTITY_BANK_CARD: u32 = 0x761e6af4;

// --- Reply markup (Layer 223, verified 2026-08) ---
pub const REPLY_KEYBOARD_MARKUP_223: u32 = REPLY_KEYBOARD_MARKUP;
pub const REPLY_KEYBOARD_HIDE: u32 = 0xa03e5b85;
pub const REPLY_KEYBOARD_FORCE_REPLY: u32 = 0x86b40b08;
pub const REPLY_INLINE_MARKUP: u32 = 0x48a30254;
pub const KEYBOARD_BUTTON_ROW: u32 = 0x77608b83;

// --- Photo sizes (Layer 223) ---
pub const PHOTO_SIZE: u32 = 0x75c78e60;
pub const PHOTO_STRIPPED_SIZE: u32 = 0xe0b0bc2e;
pub const PHOTO_SIZE_PROGRESSIVE: u32 = 0xfa3efb95;
pub const PHOTO_PATH_SIZE: u32 = 0xd8214d41;
/// `photoSizeEmpty#e17e23c type:string`
pub const PHOTO_SIZE_EMPTY: u32 = 0xe17e23c;
/// `videoSize#de33b094 flags:# type:string w:int h:int size:int
///   video_start_ts:flags.0?double`
pub const VIDEO_SIZE: u32 = 0xde33b094;
/// `forumTopicDeleted#23f109b id:int`
pub const FORUM_TOPIC_DELETED: u32 = 0x23f109b;
/// `peerSettings#f47741f7 flags:# ...` (all-bool + optional scalars)
pub const PEER_SETTINGS: u32 = 0xf47741f7;
/// `userFull#6cbe645 flags:# ...` (live layer 225)
pub const USER_FULL: u32 = 0x6cbe645;
/// `webPage#e89c45b2 flags:# ...`
pub const WEB_PAGE: u32 = 0xe89c45b2;

// --- Document attributes (Layer 223) ---
pub const DOCUMENT_ATTRIBUTE_IMAGE_SIZE: u32 = 0x6c37c15c;
pub const DOCUMENT_ATTRIBUTE_ANIMATED: u32 = 0x11b58939;
pub const DOCUMENT_ATTRIBUTE_STICKER: u32 = 0x6319d612;
pub const DOCUMENT_ATTRIBUTE_VIDEO: u32 = 0x43c57c48;
pub const DOCUMENT_ATTRIBUTE_AUDIO: u32 = 0x9852f9c6;
pub const DOCUMENT_ATTRIBUTE_FILENAME: u32 = 0x15590068;
pub const DOCUMENT_ATTRIBUTE_HAS_STICKERS: u32 = 0x9801d2f7;
pub const DOCUMENT_ATTRIBUTE_CUSTOM_EMOJI: u32 = 0xfd149899;

// --- Chat full / rights / folder (Layer 223) ---
pub const CHANNEL_FULL: u32 = 0xe4e0b29d;
pub const CHAT_ADMIN_RIGHTS: u32 = 0x5fb224d5;
pub const CHAT_BANNED_RIGHTS: u32 = 0x9f120418;
pub const FOLDER: u32 = 0xff544e65;

// --- Message media (Layer 223) ---
pub const MESSAGE_MEDIA_VENUE: u32 = 0x2ec0533f;
pub const MESSAGE_MEDIA_GEO_LIVE: u32 = 0xb940c666;

// --- Message action (Layer 223) ---
pub const MESSAGE_ACTION_EMPTY: u32 = 0xb6aef7b0;
pub const MESSAGE_ACTION_HISTORY_CLEAR: u32 = 0x9fbab604;
pub const MESSAGE_ACTION_CHAT_CREATE: u32 = 0xbd47cbad;
pub const MESSAGE_ACTION_CHAT_EDIT_TITLE: u32 = 0xb5a1ce5a;
pub const MESSAGE_ACTION_CHAT_ADD_USER: u32 = 0x15cefd00;
pub const MESSAGE_ACTION_CHAT_DELETE_USER: u32 = 0xa43f30cc;
pub const MESSAGE_ACTION_CHAT_JOINED_BY_LINK: u32 = 0x031224c3;
/// `messageActionContactSignUp#f3f25f76`
pub const MESSAGE_ACTION_CONTACT_SIGN_UP: u32 = 0xf3f25f76;
/// `messageActionChatJoinedByRequest#ebbca3cb`
pub const MESSAGE_ACTION_CHAT_JOINED_BY_REQUEST: u32 = 0xebbca3cb;
pub const MESSAGE_ACTION_CHANNEL_CREATE: u32 = 0x95d2ac92;
pub const MESSAGE_ACTION_PIN_MESSAGE: u32 = 0x94bd38ed;
pub const MESSAGE_ACTION_GAME_SCORE: u32 = 0x92a72876;

// --- Peer (Layer 223) ---
pub const PEER_USER: u32 = 0x59511722;
pub const PEER_CHAT: u32 = 0x36c6019a;
pub const PEER_CHANNEL: u32 = 0xa2a5371e;

// --- Updates (Layer 223) ---
pub const UPDATES: u32 = 0x74ae4240;
pub const UPDATE_SHORT: u32 = 0x78d4dec1; // updateShort#78d4dec1 — verified against api.tl
pub const UPDATES_COMBINED: u32 = 0x725b04c3; // updatesCombined#725b04c3 — verified against api.tl
pub const UPDATE_SHORT_SENT_MESSAGE: u32 = 0x9015e101;

// --- Update events (Layer 223) ---
pub const UPDATE_NEW_MESSAGE: u32 = 0x1f2b0afd;
// updateMessageID#4e90bfd6 id:int random_id:long
pub const UPDATE_MESSAGE_ID: u32 = 0x4e90bfd6;
pub const UPDATE_DELETE_MESSAGES: u32 = 0xa20db0e5;
pub const UPDATE_READ_HISTORY_INBOX: u32 = 0x9e84bc99;
pub const UPDATE_READ_HISTORY_OUTBOX: u32 = 0x2f2f21bf;
pub const UPDATE_CHANNEL_TOO_LONG: u32 = 0x108d941f;
pub const UPDATE_EDIT_MESSAGE: u32 = 0xe40370a3;
/// updateReadMessages#c66f9217 messages:Vector<int>
pub const UPDATE_READ_MESSAGES: u32 = 0xf8227181;
pub const UPDATE_WEB_PAGE: u32 = 0x7f891213;
/// replyKeyboardMarkup#350284c2
pub const REPLY_KEYBOARD_MARKUP: u32 = 0x85dd99d1;
pub mod inline_keyboard_markup { pub const CONSTRUCTOR_ID: u32 = 0x48a30254; }

// --- Keyboard buttons ---
pub const KEYBOARD_BUTTON: u32 = 0x7d170cff;
// keyboardButtonStyle#4fdd3430
pub const KEYBOARD_BUTTON_STYLE: u32 = 0x4fdd3430;
pub const KEYBOARD_BUTTON_URL: u32 = 0xd80c25ec;
pub const KEYBOARD_BUTTON_CALLBACK: u32 = 0xe62bc960;
pub const KEYBOARD_BUTTON_SWITCH_INLINE: u32 = 0x991399fc;
pub const KEYBOARD_BUTTON_GAME: u32 = 0x89c590f9;
pub const KEYBOARD_BUTTON_URL_AUTH: u32 = 0xf51006f9;
pub const KEYBOARD_BUTTON_REQUEST_PEER: u32 = 0x5b0f15f5;

// --- Messages (Layer 223) ---
pub const MESSAGES_DIALOGS: u32 = 0x15ba6c40;
pub const MESSAGES_DIALOGS_SLICE: u32 = 0x71e094f3;
pub const MESSAGES_DIALOGS_NOT_MODIFIED: u32 = 0xf0e3e596;
pub const MESSAGES_MESSAGES: u32 = 0x1d73e7ea;
pub const MESSAGES_MESSAGES_SLICE: u32 = 0x5f206716;
pub const MESSAGES_CHANNEL_MESSAGES: u32 = 0xc776ba4e;
pub const MESSAGES_MESSAGES_NOT_MODIFIED: u32 = 0x74535f21;

// --- Dialog (layer 225: dialog#fc89f7f3; old 0xd58a08c6 stale) ---
pub const DIALOG: u32 = 0xfc89f7f3;
pub const DIALOG_FOLDER: u32 = 0x71bd134c;

// --- Sent code (Layer 223) ---
pub const AUTH_SENT_CODE: u32 = 0x5e002502;
pub const AUTH_SENT_CODE_SUCCESS: u32 = 0x2390fe44;
pub const AUTH_SENT_CODE_PAYMENT_REQUIRED: u32 = 0xe0955a3c;
pub const AUTH_SENT_CODE_TYPE_APP: u32 = 0x3dbb5986;
pub const AUTH_SENT_CODE_TYPE_SMS: u32 = 0xc000bba2;
/// `codeSettings#ad253d78 flags:# ...` — full TL object in modern layers
/// (was a bare flags int before).
pub const CODE_SETTINGS: u32 = 0xad253d78;

// --- Auth (Layer 223) ---
// IDs verified against https://core.telegram.org/schema/json (2026-08).
pub const AUTH_AUTHORIZATION: u32 = 0x2ea2c0d4;
pub const AUTH_AUTHORIZATION_SIGN_UP_REQUIRED: u32 = 0x44747e9a;
/// `auth.loggedOut#c3a2835f flags:# future_auth_token:flags.0?bytes`
pub const AUTH_LOGGED_OUT: u32 = 0xc3a2835f;
/// `auth.logOut#3e72ba19 = auth.LoggedOut;`
pub const AUTH_LOG_OUT: u32 = 0x3e72ba19;

// --- Auth functions (Layer 223) ---
pub const AUTH_SEND_CODE: u32 = 0xa677244f;
/// `auth.signIn#8d52a951 flags:# ...` — has a `flags:#` field in Layer 223.
pub const AUTH_SIGN_IN: u32 = 0x8d52a951;
/// `auth.signUp#aac7b717 flags:# ...` — has a `flags:#` field in Layer 223.
pub const AUTH_SIGN_UP: u32 = 0xaac7b717;
/// `auth.checkPassword#d18b4d16 password:InputCheckPasswordSRP = auth.Authorization;`
pub const AUTH_CHECK_PASSWORD: u32 = 0xd18b4d16;
pub const IMPORT_BOT_AUTH: u32 = 0x67a3ff2c;
/// `auth.exportLoginToken#b7e085fe api_id:int api_hash:string except_ids:Vector<long> = auth.LoginToken;`
pub const AUTH_EXPORT_LOGIN_TOKEN: u32 = 0xb7e085fe;
/// `auth.importLoginToken#95ac5ce4 token:bytes = auth.LoginToken;`
pub const AUTH_IMPORT_LOGIN_TOKEN: u32 = 0x95ac5ce4;
/// `auth.acceptLoginToken#e894ad4d token:bytes = Authorization;`
pub const AUTH_ACCEPT_LOGIN_TOKEN: u32 = 0xe894ad4d;

// --- Login token results (Layer 223) ---
/// `auth.loginToken#629f1980 expires:int token:bytes`
pub const AUTH_LOGIN_TOKEN: u32 = 0x629f1980;
/// `auth.loginTokenMigrateTo#68e9916 dc_id:int token:bytes`
pub const AUTH_LOGIN_TOKEN_MIGRATE_TO: u32 = 0x068e9916;
/// `auth.loginTokenSuccess#390d5c5e authorization:auth.Authorization`
pub const AUTH_LOGIN_TOKEN_SUCCESS: u32 = 0x390d5c5e;

// --- SRP password check (Layer 223) ---
/// `inputCheckPasswordEmpty#9880f658`
pub const INPUT_CHECK_PASSWORD_EMPTY: u32 = 0x9880f658;
/// `inputCheckPasswordSRP#d27ff082 srp_id:long A:bytes M1:bytes`
pub const INPUT_CHECK_PASSWORD_SRP: u32 = 0xd27ff082;
/// `account.getPassword#548a30f5 = account.Password;` (function)
pub const ACCOUNT_GET_PASSWORD: u32 = 0x548a30f5;
/// `account.password#5188ee1b ...` (response predicate)
pub const ACCOUNT_GET_PASSWORD_RESPONSE: u32 = 0x957b50fb;
// securePasswordKdfAlgoPBKDF2HMACSHA512iter100000#bbf2dda0 {salt:bytes}
pub const SECURE_PASSWORD_KDF_ALGO_PBKDF2: u32 = 0xbbf2dda0;
// securePasswordKdfAlgoSHA512#86471d92 {salt:bytes}
pub const SECURE_PASSWORD_KDF_ALGO_SHA512: u32 = 0x86471d92;
/// `passwordKdfAlgoSHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow#3a912d4a
///  salt1:bytes salt2:bytes g:int p:bytes`
pub const PASSWORD_KDF_ALGO_SHA256_SHA256_PBKDF2_HMACSHA512_100K_MODPOW: u32 = 0x3a912d4a;

// --- Messages methods ---
pub const MESSAGES_SEND_MESSAGE: u32 = 0x545cd15a;
/// `messages.sendMedia#330e77f` — Layer 223 (verified against schema 2026-08).
pub const MESSAGES_SEND_MEDIA: u32 = 0x0330e77f;
/// `messages.sendMultiMedia#1bf89d74`
pub const MESSAGES_SEND_MULTI_MEDIA: u32 = 0x1bf89d74;
pub const MESSAGES_GET_DIALOGS: u32 = 0xa0f4cb4f;
pub const MESSAGES_GET_HISTORY: u32 = 0x4423e6c5;
pub const MESSAGES_GET_MESSAGES: u32 = 0x63c66506;
/// `messages.getBotCallbackAnswer#9342ca07 flags:# game:flags.1?true
///  peer:InputPeer msg_id:int data:flags.0?bytes
///  password:flags.2?InputCheckPasswordSRP` (Layer 223)
pub const MESSAGES_GET_BOT_CALLBACK_ANSWER: u32 = 0x9342ca07;
/// `messages.botCallbackAnswer#36585ea4 flags:# alert:flags.1?true
///  has_url:flags.3?true native_ui:flags.4?true message:flags.0?string
///  url:flags.2?string cache_time:int`
pub const MESSAGES_BOT_CALLBACK_ANSWER: u32 = 0x36585ea4;
/// `messages.affectedMessages#84d19185 pts:int pts_count:int`
pub const MESSAGES_AFFECTED_MESSAGES: u32 = 0x84d19185;
pub const MESSAGES_DELETE_MESSAGES: u32 = 0xe58e95d2;
/// `messages.deleteHistory#b08f922a flags:# just_clear:flags.0?true
///  revoke:flags.1?true peer:InputPeer max_id:int min_date:flags.2?int
///  max_date:flags.3?int = messages.AffectedHistory;`
pub const MESSAGES_DELETE_HISTORY: u32 = 0xb08f922a;
pub const MESSAGES_EDIT_MESSAGE: u32 = 0x51e842e1;
pub const MESSAGES_READ_HISTORY: u32 = 0x0e306d3a;
pub const MESSAGES_SEARCH: u32 = 0x29ee847a;
/// `messages.affectedHistory#b45c69d1 pts:int pts_count:int offset:int`
pub const MESSAGES_AFFECTED_HISTORY: u32 = 0xb45c69d1;

// --- Users ---
pub const USERS_GET_FULL_USER: u32 = 0xb60f5918;
pub const USERS_GET_USERS: u32 = 0x0d91a548;
/// users.userFull#d69e83e0 full_user:UserFull chats:Vector<Chat> users:Vector<User>
pub const USERS_USER_FULL: u32 = 0x3b6d152e;
/// contacts.found#b3134d19 my_results:Vector<Peer> results:Vector<Peer> chats:Vector<Chat> users:Vector<User>
pub const CONTACTS_FOUND: u32 = 0xb3134d9d;
/// `contacts.resolvePhone#8af94344 phone:string = contacts.ResolvedPeer;`
pub const CONTACTS_RESOLVE_PHONE: u32 = 0x8af94344;
pub const CONTACTS_SEARCH: u32 = 0x11f812d8;
/// `contacts.resolvedPeer#7f077ad9 peer:Peer chats:Vector<Chat> users:Vector<User>`
pub const CONTACTS_RESOLVED_PEER: u32 = 0x7f077ad9;
pub const CONTACTS_RESOLVE_USERNAME: u32 = 0x725afbbc;

// --- Channels ---
pub const CHANNELS_CREATE_CHANNEL: u32 = 0x91006707;
pub const CHANNELS_INVITE_TO_CHANNEL: u32 = 0xc9e33d54;
pub const CHANNELS_EDIT_ADMIN: u32 = 0x9a98ad68;
pub const CHANNELS_GET_CHANNELS: u32 = 0xa7f6bbb;
/// `channels.getParticipants#77ced9d0 channel:InputChannel
///  filter:ChannelParticipantsFilter offset:int limit:int hash:long`
pub const CHANNELS_GET_PARTICIPANTS: u32 = 0x77ced9d0;
/// `channels.editAbout#13e27b46` was removed from the schema; about is now
/// set via `messages.editChatAbout#def60797 peer:InputPeer about:string`.
pub const MESSAGES_EDIT_CHAT_ABOUT: u32 = 0xdef60797;
pub const CHANNELS_LEAVE_CHANNEL: u32 = 0xf836aa95;
/// `channels.channelParticipants#9ab0feaf count:int participants:...`
pub const CHANNELS_CHANNEL_PARTICIPANTS: u32 = 0x9ab0feaf;
/// `channelParticipantsRecent#de3f3c79`
pub const CHANNEL_PARTICIPANTS_RECENT: u32 = 0xde3f3c79;
/// `channelParticipantsSearch#656ac4b q:string`
pub const CHANNEL_PARTICIPANTS_SEARCH: u32 = 0x0656ac4b;

// --- Updates ---
/// `updates.state#a56c2a3e pts:int qts:int date:int seq:int unread_count:int`
pub const UPDATES_STATE: u32 = 0xa56c2a3e;
pub const UPDATES_GET_STATE: u32 = 0xedd4882a;
pub const UPDATES_GET_DIFFERENCE: u32 = 0x19c2f763;
pub const UPDATES_GET_CHANNEL_DIFFERENCE: u32 = 0x3173d78;
/// updates.differenceEmpty#a9eca690 date:int seq:int pts:int pts_count:int
pub const DIFFERENCE_EMPTY: u32 = 0x5d75a138;
/// updates.difference#f46ca0 seq:int new_messages:Vector<Message>
///   other_updates:Vector<Update> chats:Vector<Chat> users:Vector<User>
pub const DIFFERENCE: u32 = 0xf49ca0;
/// updates.differenceSlice#a004db6 new_messages:... other_updates:...
///   chats:... users:... intermediate_state:State
pub const DIFFERENCE_SLICE: u32 = 0xa8fb1981;
/// updates.differenceTooLong#4afe8f6d pts:int
pub const DIFFERENCE_TOO_LONG: u32 = 0x4afe8f6d;
/// updates.channelDifferenceEmpty#3e11affb flags:# pts:int final:flags.0?true
pub const CHANNEL_DIFFERENCE_EMPTY: u32 = 0x3e11affb;
/// updates.channelDifference#2064674e flags:# pts:int final:flags.0?true
///   timeout:flags.1?int messages:Vector<Message> chats:... users:...
pub const CHANNEL_DIFFERENCE: u32 = 0x2064674e;
/// updates.channelDifferenceTooLong#4103bd2d flags:# pts:int final:flags.0?true
///   timeout:flags.1?int messages:Vector<Message> chats:... users:...
pub const CHANNEL_DIFFERENCE_TOO_LONG: u32 = 0xa4bcc6fe;

// --- Upload ---
pub const UPLOAD_SAVE_FILE_PART: u32 = 0xb304a621;
pub const UPLOAD_SAVE_BIG_FILE_PART: u32 = 0xde7b673d;
/// `upload.getFile#be5335be flags:# precise:flags.0?true
///  cdn_supported:flags.1?true location:InputFileLocation offset:long limit:int`
pub const UPLOAD_GET_FILE: u32 = 0xbe5335be;
/// `upload.getWebFile#24e6818d location:InputWebFileLocation offset:int limit:int`
pub const UPLOAD_GET_WEB_FILE: u32 = 0x24e6818d;
/// Historical ID. `upload.saveFile` does not exist in Layer 223; the
/// upload path is `upload.saveFilePart` / `upload.saveBigFilePart`.
pub const UPLOAD_SAVE_FILE: u32 = 0x96f18c5e;
/// `upload.getCdnFile#395f69da file_token:bytes offset:long limit:int`
pub const UPLOAD_GET_CDN_FILE: u32 = 0x395f69da;
/// `upload.webFile#21e753bc size:int mime_type:string file_type:storage.FileType
///  mtime:int bytes:bytes`
pub const UPLOAD_WEB_FILE: u32 = 0x21e753bc;
/// `upload.cdnFile#a99fca4f bytes:bytes`
pub const UPLOAD_CDN_FILE: u32 = 0xa99fca4f;
/// `upload.cdnFileReuploadNeeded#eea8e46e request_token:bytes`
pub const UPLOAD_CDN_FILE_REUPLOAD_NEEDED: u32 = 0xeea8e46e;
/// `inputWebFileLocation#c239d686 url:string access_hash:long`
pub const INPUT_WEB_FILE_LOCATION: u32 = 0xc239d686;

// --- Photos ---
/// `photos.updateProfilePhoto#9e82039 flags:# fallback:flags.0?true
///  bot:flags.1?InputUser id:InputPhoto = photos.Photo;`
pub const PHOTOS_UPDATE_PROFILE_PHOTO: u32 = 0x09e82039;
/// `photos.uploadProfilePhoto#388a3b5 flags:# fallback:flags.3?true
///  bot:flags.5?InputUser file:flags.0?InputFile video:flags.1?InputFile
///  video_start_ts:flags.2?double video_emoji_markup:flags.4?VideoSize`
pub const PHOTOS_UPLOAD_PROFILE_PHOTO: u32 = 0x388a3b5;
pub const PHOTOS_DELETE_PHOTOS: u32 = 0x87cf7f2f;
pub const PHOTOS_GET_USER_PHOTOS: u32 = 0x91cd32a8;
/// `photos.photo#20212ca8 photo:Photo users:Vector<User>`
pub const PHOTOS_PHOTO: u32 = 0x20212ca8;
/// `photos.photos#8dca6aa5 photos:Vector<Photo> users:Vector<User>`
pub const PHOTOS_PHOTOS: u32 = 0x8dca6aa5;
/// `photos.photosSlice#15051f54 count:int photos:Vector<Photo> users:Vector<User>`
pub const PHOTOS_PHOTOS_SLICE: u32 = 0x15051f54;
/// `inputPhoto#3bb3b94a id:long access_hash:long file_reference:bytes`
pub const INPUT_PHOTO: u32 = 0x3bb3b94a;
pub const INPUT_PHOTO_EMPTY: u32 = 0x1cd7bf0d;
/// `inputMediaEmpty#9664f57f`
pub const INPUT_MEDIA_EMPTY: u32 = 0x9664f57f;
/// `inputMediaContact#f8ab7dfb phone_number:string first_name:string
///  last_name:string vcard:string`
pub const INPUT_MEDIA_CONTACT: u32 = 0xf8ab7dfb;
/// `inputMediaGeoPoint#f9c44144 flags:# lat:double long:double
///  accuracy_radius:flags.0?int` — wraps an InputGeoPoint.
pub const INPUT_MEDIA_GEO_POINT: u32 = 0xf9c44144;
pub const INPUT_GEO_POINT: u32 = 0x48222faf;
pub const INPUT_GEO_POINT_EMPTY: u32 = 0xe4c123d6;
/// `inputSingleMedia#1cc6e91f flags:# media:InputMedia random_id:long
///  message:string entities:flags.0?Vector<MessageEntity>`
pub const INPUT_SINGLE_MEDIA: u32 = 0x1cc6e91f;

// --- Invoke wrappers ---
pub const INVOKE_WITH_LAYER: u32 = 0xda9b0d0d;
// initConnection#c1cd5ea9 — identifies the client for RPC (CONNECTION_NOT_INITED)
pub const INIT_CONNECTION: u32 = 0xc1cd5ea9;
pub const INVOKE_AFTER_MSG: u32 = 0xcb9f372d;
pub const INVOKE_WITHOUT_UPDATES: u32 = 0xbf9459b7;

// --- Help ---
pub const HELP_GET_CONFIG: u32 = 0xc4f9186b;
pub const HELP_GET_NEAREST_DC: u32 = 0x1fb33026;
// nearestDc#8e1a1775 country:string this_dc:int nearest_dc:int
pub const NEAREST_DC: u32 = 0x8e1a1775;
// config#cc1a241e — help.getConfig response
pub const CONFIG: u32 = 0xcc1a241e;
// dcOption#18b7a10d — one entry of config.dc_options
pub const DC_OPTION: u32 = 0x18b7a10d;


// --- Bool ---
pub const BOOL_TRUE: u32 = 0x997275b5;
pub const BOOL_FALSE: u32 = 0xbc799737;
pub const VECTOR: u32 = 0x1cb5c415;
