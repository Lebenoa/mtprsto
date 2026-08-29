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
pub const INPUT_REPLY_TO_MONOFORUM: u32 = 0x76ab27de;
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
/// document#8fd32c0b (Layer 223)
pub const DOCUMENT: u32 = 0x8fd32c0b;
/// documentEmpty#3631cf4c id:long
pub const DOCUMENT_EMPTY: u32 = 0x3631cf4c;

// --- User (Layer 223) ---
pub const USER: u32 = 0x31774388;
pub const USER_EMPTY: u32 = 0xd3bc4b7a;

// --- User status (Layer 223) ---
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
pub const CHANNEL: u32 = 0x1c32b11c;
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
pub const MESSAGE_EMPTY: u32 = 0x90a6ca84;
pub const MESSAGE_SERVICE: u32 = 0x7a800e0a;

// --- Message media (Layer 223) ---
pub const MESSAGE_MEDIA_EMPTY: u32 = 0x3ded6320;
pub const MESSAGE_MEDIA_PHOTO: u32 = 0x695150d7;
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

// --- Message action (Layer 223) ---
pub const MESSAGE_ACTION_EMPTY: u32 = 0xb6aef7b0;
pub const MESSAGE_ACTION_HISTORY_CLEAR: u32 = 0x9fbab604;
pub const MESSAGE_ACTION_CHAT_CREATE: u32 = 0xbd47cbad;
pub const MESSAGE_ACTION_CHAT_EDIT_TITLE: u32 = 0xb5a1ce5a;
pub const MESSAGE_ACTION_CHAT_ADD_USER: u32 = 0x15cefd00;
pub const MESSAGE_ACTION_CHAT_DELETE_USER: u32 = 0xa43f30cc;
pub const MESSAGE_ACTION_CHAT_JOINED_BY_LINK: u32 = 0x031224c3;
pub const MESSAGE_ACTION_CHANNEL_CREATE: u32 = 0x95d2ac92;
pub const MESSAGE_ACTION_PIN_MESSAGE: u32 = 0x94bd38ed;
pub const MESSAGE_ACTION_GAME_SCORE: u32 = 0x92a72876;

// --- Peer (Layer 223) ---
pub const PEER_USER: u32 = 0x59511722;
pub const PEER_CHAT: u32 = 0x36c6019a;
pub const PEER_CHANNEL: u32 = 0xa2a5371e;

// --- Updates (Layer 223) ---
pub const UPDATES: u32 = 0x74ae4240;
pub const UPDATE_SHORT: u32 = 0x78d4dec1; // TODO: verify from schema
pub const UPDATES_COMBINED: u32 = 0x725b04c3; // TODO: verify from schema
pub const UPDATE_SHORT_SENT_MESSAGE: u32 = 0x9015e101;

// --- Update events (Layer 223) ---
pub const UPDATE_NEW_MESSAGE: u32 = 0x1f2b0afd;
pub const UPDATE_DELETE_MESSAGES: u32 = 0xa20db0e5;
pub const UPDATE_READ_HISTORY_INBOX: u32 = 0x9e84bc99;
pub const UPDATE_READ_HISTORY_OUTBOX: u32 = 0x2f2f21bf;
pub const UPDATE_CHANNEL_TOO_LONG: u32 = 0x108d941f;
pub const UPDATE_EDIT_MESSAGE: u32 = 0xe40370a3;
/// updateReadMessages#c66f9217 messages:Vector<int>
pub const UPDATE_READ_MESSAGES: u32 = 0xc66f9217;
pub const UPDATE_WEB_PAGE: u32 = 0x7f891213;
/// replyKeyboardMarkup#350284c2
pub const REPLY_KEYBOARD_MARKUP: u32 = 0x350284c2;
pub const FORCE_REPLY: u32 = 0x86872538;
pub mod inline_keyboard_markup { pub const CONSTRUCTOR_ID: u32 = 0x158b2380; }

// --- Keyboard buttons ---
pub const KEYBOARD_BUTTON: u32 = 0x683a5c46;
pub const KEYBOARD_BUTTON_URL: u32 = 0x258aff06;
pub const KEYBOARD_BUTTON_CALLBACK: u32 = 0x3250872a;
pub const KEYBOARD_BUTTON_SWITCH_INLINE: u32 = 0x063760c8;
pub const KEYBOARD_BUTTON_GAME: u32 = 0x568be74c;
pub const KEYBOARD_BUTTON_URL_AUTH: u32 = 0x10b78d29;
pub const KEYBOARD_BUTTON_REQUEST_PEER: u32 = 0xb1764226;

// --- Messages (Layer 223) ---
pub const MESSAGES_DIALOGS: u32 = 0x15ba6c40;
pub const MESSAGES_DIALOGS_SLICE: u32 = 0x71e094f3;
pub const MESSAGES_DIALOGS_NOT_MODIFIED: u32 = 0xf0e3e596;
pub const MESSAGES_MESSAGES: u32 = 0x1d73e7ea;
pub const MESSAGES_MESSAGES_SLICE: u32 = 0x5f206716;
pub const MESSAGES_CHANNEL_MESSAGES: u32 = 0xc776ba4e;
pub const MESSAGES_MESSAGES_NOT_MODIFIED: u32 = 0x74535f21;

// --- Dialog (Layer 223) ---
pub const DIALOG: u32 = 0xd58a08c6;
pub const DIALOG_FOLDER: u32 = 0x71bd134c;

// --- Sent code (Layer 223) ---
pub const AUTH_SENT_CODE: u32 = 0x5e002502;
pub const AUTH_SENT_CODE_SUCCESS: u32 = 0x2390fe44;
pub const AUTH_SENT_CODE_PAYMENT_REQUIRED: u32 = 0xe0955a3c;
pub const AUTH_SENT_CODE_TYPE_APP: u32 = 0x3dbb5986;
pub const AUTH_SENT_CODE_TYPE_SMS: u32 = 0xc004bac7;

// --- Auth (Layer 223) ---
pub const AUTH_AUTHORIZATION: u32 = 0x2ea2c0d4;
pub const AUTH_AUTHORIZATION_SIGN_UP_REQUIRED: u32 = 0x44747e9a;
pub const AUTH_LOG_OUT: u32 = 0x87971c3d; // TODO: verify

// --- Auth functions (Layer 223) ---
pub const AUTH_SEND_CODE: u32 = 0xa677244f;
pub const AUTH_SIGN_IN: u32 = 0x8d52a951; // TODO: verify
pub const AUTH_SIGN_UP: u32 = 0x80eead27; // TODO: verify
pub const AUTH_CHECK_PASSWORD: u32 = 0xd18b4d16; // TODO: verify
pub const IMPORT_BOT_AUTH: u32 = 0x67a3ff2c;

// --- Messages methods ---
pub const MESSAGES_SEND_MESSAGE: u32 = 0x545cd15a;
pub const MESSAGES_SEND_MEDIA: u32 = 0xb8d0afdf;
pub const MESSAGES_SEND_MULTI_MEDIA: u32 = 0xb6f3e0c0;
pub const MESSAGES_GET_DIALOGS: u32 = 0xa0f4cb4f;
pub const MESSAGES_GET_HISTORY: u32 = 0xdc3f8240;
pub const MESSAGES_GET_MESSAGES: u32 = 0x63c66506;
pub const MESSAGES_GET_BOT_CALLBACK_ANSWER: u32 = 0x934a4ee1;
pub const MESSAGES_DELETE_MESSAGES: u32 = 0xe58e95c6;
pub const MESSAGES_DELETE_HISTORY: u32 = 0xb7e36194;
pub const MESSAGES_EDIT_MESSAGE: u32 = 0x48f71768;
pub const MESSAGES_READ_HISTORY: u32 = 0x0e306d3a;
pub const MESSAGES_SEARCH: u32 = 0xd07bbf76;
pub const MESSAGES_SEND_CALLBACK_DATA: u32 = 0x934a4ee1;

// --- Users ---
pub const USERS_GET_FULL_USER: u32 = 0xe0b917f2;
pub const USERS_GET_USERS: u32 = 0x0d91a548;
/// users.userFull#d69e83e0 full_user:UserFull chats:Vector<Chat> users:Vector<User>
pub const USERS_USER_FULL: u32 = 0xd69e83e0;
/// contacts.found#b3134d19 my_results:Vector<Peer> results:Vector<Peer> chats:Vector<Chat> users:Vector<User>
pub const CONTACTS_FOUND: u32 = 0xb3134d19;
/// updates.state#a56c2a3e pts:int qts:int date:int seq:int unread_count:int
pub const UPDATES_STATE: u32 = 0xa56c2a3e;

// --- Contacts ---
pub const CONTACTS_RESOLVE_USERNAME: u32 = 0xf93ccba3;
pub const CONTACTS_RESOLVE_PHONE: u32 = 0x8af2a521;
pub const CONTACTS_SEARCH: u32 = 0x11f812d8;

// --- Channels ---
pub const CHANNELS_CREATE_CHANNEL: u32 = 0x3d5d10fd;
pub const CHANNELS_INVITE_TO_CHANNEL: u32 = 0x199f3a6c;
pub const CHANNELS_EDIT_ADMIN: u32 = 0x70d896ff;
pub const CHANNELS_GET_CHANNELS: u32 = 0xa7f6d76b;
pub const CHANNELS_GET_PARTICIPANTS: u32 = 0x123ffe12;
pub const CHANNELS_EDIT_ABOUT: u32 = 0x13e27b46;
pub const CHANNELS_LEAVE_CHANNEL: u32 = 0xf836aa28;

// --- Updates ---
pub const UPDATES_GET_STATE: u32 = 0xedd4882a;
pub const UPDATES_GET_DIFFERENCE: u32 = 0x25939104;
pub const UPDATES_GET_CHANNEL_DIFFERENCE: u32 = 0x3173d78;

// --- Upload ---
pub const UPLOAD_SAVE_FILE_PART: u32 = 0xb304a621;
pub const UPLOAD_SAVE_BIG_FILE_PART: u32 = 0xde7b673d;
pub const UPLOAD_GET_FILE: u32 = 0xb3e7e951;
pub const UPLOAD_GET_WEB_FILE: u32 = 0x24e5e54e;
pub const UPLOAD_SAVE_FILE: u32 = 0x96f18c5e;
pub const UPLOAD_GET_CDN_FILE: u32 = 0x572f9519;

// --- Help ---
pub const HELP_GET_CONFIG: u32 = 0xc4f3926c;
pub const HELP_GET_NEAREST_DC: u32 = 0x1fb33026;

// --- Photos ---
pub const PHOTOS_UPDATE_PROFILE_PHOTO: u32 = 0x1c3c2a85;
pub const PHOTOS_UPLOAD_PROFILE_PHOTO: u32 = 0x4f32c098;
pub const PHOTOS_DELETE_PHOTOS: u32 = 0x87cf7f2f;
pub const PHOTOS_GET_USER_PHOTOS: u32 = 0x91cd32a8;

// --- Invoke wrappers ---
pub const INVOKE_WITH_LAYER: u32 = 0xda9b0d0d;
pub const INVOKE_AFTER_MSG: u32 = 0xcb9f372d;
pub const INVOKE_WITHOUT_UPDATES: u32 = 0xbf94591b;

// --- Bool ---
pub const BOOL_TRUE: u32 = 0x997275b5;
pub const BOOL_FALSE: u32 = 0xbc799737;
pub const VECTOR: u32 = 0x1cb5c415;
