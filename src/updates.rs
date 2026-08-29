//! Update dispatch and pts/seq tracking per SPEC §6.
//!
//! Telegram delivers real-time state changes as `Updates` objects.
//! This module tracks `pts` (per-account and per-channel), `seq`,
//! and `date` to detect gaps and request `updates.getDifference`.
//!
//! # Architecture
//!
//! ```text
//! TelegramServer
//!     │
//!     ├─ Updates (via main connection)
//!     │    │
//!     │    ▼
//!     │  UpdateDispatcher
//!     │    ├─ pts tracking (gap detection)
//!     │    ├─ seq tracking (ordering)
//!     │    └─ mpsc::UnboundedSender<Update>
//!     │         │
//!     │         ▼
//!     │    Client.updates() → mpsc::UnboundedReceiver<Update>
//!     │
//!     └─ RPC replies (via invoke)
//! ```

use crate::types::{Update, Updates};
use std::collections::HashMap;
use tokio::sync::mpsc;

/// Dispatch mode for updates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DispatchMode {
    /// Publish all updates to an mpsc channel.
    #[default]
    Channel,
    /// Register a handler function.
    Handler,
}

/// State tracking for a single pts domain (per-account or per-channel).
#[derive(Debug, Clone)]
struct PtsState {
    /// Last known pts value.
    pts: i32,
    /// Last known seq value (for per-account only).
    seq: i32,
    /// Last known server date.
    date: i32,
    /// Whether we have a pending gap detection.
    gap_pending: bool,
}

impl PtsState {
    fn new() -> Self {
        Self {
            pts: 0,
            seq: 0,
            date: 0,
            gap_pending: false,
        }
    }

    /// Check if the update's pts is the next one to process.
    ///
    /// In Telegram, `pts` is the state counter BEFORE the update,
    /// and `pts_count` is how much it advances. So the update
    /// matches our expected next state when `pts == self.pts`.
    fn is_ahead(&self, pts: i32, _pts_count: i32) -> bool {
        pts == self.pts
    }

    /// Check if we have a gap (update's pts is behind ours).
    fn is_behind(&self, pts: i32, _pts_count: i32) -> bool {
        pts < self.pts
    }

    /// Update the tracked pts after processing an update.
    fn advance(&mut self, pts: i32, pts_count: i32) {
        self.pts = pts + pts_count;
    }
}

/// Update dispatcher with pts/seq tracking.
/// Shared handler callback type (Handler dispatch mode).
pub type UpdateHandler = std::sync::Arc<dyn Fn(&Update) + Send + Sync>;

pub struct UpdateDispatcher {
    /// Per-account pts state.
    account_pts: PtsState,
    /// Per-account qts (mentions / "quick" update counter, SPEC §6.1).
    qts: i32,
    /// Per-channel pts states.
    channel_pts: HashMap<i64, PtsState>,
    /// Channel ids from `UpdateChannelTooLong` awaiting
    /// `updates.getChannelDifference` resync.
    channels_too_long: Vec<i64>,
    /// Sender for updates (used in Channel mode).
    sender: Option<mpsc::UnboundedSender<Update>>,
    /// Buffered updates waiting for gap resolution.
    buffered: Vec<Update>,
    /// Maximum buffer size before dropping.
    max_buffer: usize,
    /// Registered handlers (Handler mode); fired for every filtered update.
    handlers: Vec<UpdateHandler>,
}

impl UpdateDispatcher {
    /// Create a new update dispatcher.
    pub fn new() -> Self {
        let (sender, _receiver) = mpsc::unbounded_channel();
        Self {
            account_pts: PtsState::new(),
            qts: 0,
            channel_pts: HashMap::new(),
            channels_too_long: Vec::new(),
            sender: Some(sender),
            buffered: Vec::new(),
            max_buffer: 10_000,
            handlers: Vec::new(),
        }
    }

    /// Create a dispatcher with an mpsc channel receiver.
    pub fn with_channel() -> (Self, mpsc::UnboundedReceiver<Update>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                account_pts: PtsState::new(),
                qts: 0,
                channel_pts: HashMap::new(),
                channels_too_long: Vec::new(),
                sender: Some(sender),
                buffered: Vec::new(),
                max_buffer: 10_000,
                handlers: Vec::new(),
            },
            receiver,
        )
    }


    /// Create a dispatcher that drops updates (for testing).
    pub fn noop() -> Self {
        Self::new()
    }

    /// Initialize pts from updates.getState response.
    pub fn init_state(&mut self, pts: i32, seq: i32, date: i32) {
        self.account_pts.pts = pts;
        self.account_pts.seq = seq;
        self.account_pts.date = date;
        tracing::info!(pts, seq, date, "update state initialized");
    }

    /// Process an incoming Updates object and dispatch individual updates.
    pub fn process_updates(&mut self, updates: Updates) -> Vec<Update> {
        match updates {
            Updates::Updates { updates: upd_list, date, .. } => {
                self.account_pts.date = date;
                self.dispatch_list(upd_list)
            }
            Updates::UpdateShort { update, date, seq: _ } => {
                self.account_pts.date = date;
                self.dispatch_one(update)
            }
            Updates::UpdatesCombined { updates: upd_list, date, seq, .. } => {
                self.account_pts.date = date;
                self.account_pts.seq = seq;
                self.dispatch_list(upd_list)
            }
            Updates::UpdateShortSentMessage { id: _, pts, pts_count, date } => {
                self.account_pts.date = date;
                self.account_pts.advance(pts, pts_count);
                // This isn't a real Update — the caller uses the message_id
                Vec::new()
            }
        }
    }

    /// Dispatch a list of updates, tracking pts gaps.
    fn dispatch_list(&mut self, updates: Vec<Update>) -> Vec<Update> {
        let mut dispatched = Vec::with_capacity(updates.len());
        for update in updates {
            dispatched.extend(self.dispatch_one(update));
        }
        dispatched
    }

    /// Dispatch a single update, checking for pts gaps.
    fn dispatch_one(&mut self, update: Update) -> Vec<Update> {
        match &update {
            Update::NewMessage { pts, pts_count, .. }
            | Update::EditMessage { pts, pts_count, .. }
            | Update::DeleteMessages { pts, pts_count, .. }
            | Update::ReadHistoryInbox { pts, pts_count, .. }
            | Update::ReadHistoryOutbox { pts, pts_count, .. } => {
                let pts = *pts;
                let pts_count = *pts_count;

                if self.account_pts.is_behind(pts, pts_count) {
                    // Stale update, already processed
                    tracing::debug!(pts, expected = self.account_pts.pts, "stale update, skipping");
                    return Vec::new();
                }

                if self.account_pts.is_ahead(pts, pts_count) {
                    // Normal: this is the next update to process
                    self.account_pts.advance(pts, pts_count);
                } else {
                    // pts > self.account_pts.pts → gap detected!
                    tracing::warn!(
                        pts,
                        expected = self.account_pts.pts,
                        "pts gap detected — need getDifference"
                    );
                    self.account_pts.gap_pending = true;
                    // Buffer this update until gap is resolved
                    if self.buffered.len() < self.max_buffer {
                        self.buffered.push(update.clone());
                    }
                    return Vec::new();
                }
            }
            Update::ChannelTooLong { channel_id, pts } => {
                tracing::warn!(
                    channel_id = channel_id.0,
                    ?pts,
                    "ChannelTooLong — queued for getChannelDifference"
                );
                if !self.channels_too_long.contains(&channel_id.0) {
                    self.channels_too_long.push(channel_id.0);
                }
                if let Some(pts) = *pts {
                    self.init_channel_pts(channel_id.0, pts);
                }
            }
            _ => {}
        }

        // Dispatch to channel
        if let Some(sender) = &self.sender {
            let _ = sender.send(update.clone());
        }
        // Dispatch to registered handlers (Handler mode)
        for handler in &self.handlers {
            handler(&update);
        }
        vec![update]
    }

    /// Register a handler fired for every filtered update (Handler mode).
    ///
    /// Handlers run inline on the dispatcher task — keep them fast or spawn.
    pub fn on_update(&mut self, handler: UpdateHandler) {
        self.handlers.push(handler);
    }

    /// Check if we need to call updates.getDifference.
    pub fn needs_difference(&self) -> bool {
        self.account_pts.gap_pending
    }

    /// Mark that we've resolved the gap.
    pub fn gap_resolved(&mut self, new_pts: i32, new_seq: i32, new_date: i32) {
        self.account_pts.pts = new_pts;
        self.account_pts.seq = new_seq;
        self.account_pts.date = new_date;
        self.account_pts.gap_pending = false;

        // Flush buffered updates
        let buffered = std::mem::take(&mut self.buffered);
        tracing::info!(
            buffered_count = buffered.len(),
            "gap resolved, flushing buffered updates"
        );
        // Re-dispatch buffered updates (they'll be accepted now)
        for update in buffered {
            self.dispatch_one(update);
        }
    }

    /// Get the current account pts.
    pub fn account_pts(&self) -> i32 {
        self.account_pts.pts
    }

    /// Get the current account seq.
    pub fn account_seq(&self) -> i32 {
        self.account_pts.seq
    }

    /// Get the current date.
    pub fn date(&self) -> i32 {
        self.account_pts.date
    }

    /// Get the number of buffered updates.
    pub fn buffered_count(&self) -> usize {
        self.buffered.len()
    }

    /// Initialize channel pts (call when joining a channel).
    pub fn init_channel_pts(&mut self, channel_id: i64, pts: i32) {
        self.channel_pts.insert(channel_id, PtsState {
            pts,
            seq: 0,
            date: 0,
            gap_pending: false,
        });
    }

    /// Check if a specific channel has a pending gap.
    pub fn channel_needs_difference(&self, channel_id: i64) -> bool {
        self.channel_pts
            .get(&channel_id)
            .map(|s| s.gap_pending)
            .unwrap_or(false)
    }

    /// Drain the queue of channels flagged by `UpdateChannelTooLong`
    /// (SPEC §6.1: each needs `updates.getChannelDifference`).
    pub fn take_channels_too_long(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.channels_too_long)
    }

    /// Store a channel's absolute pts (after a getChannelDifference round).
    pub fn advance_channel_pts(&mut self, channel_id: i64, pts: i32) {
        self.channel_pts
            .entry(channel_id)
            .and_modify(|s| {
                s.pts = pts;
                s.gap_pending = false;
            })
            .or_insert_with(|| PtsState {
                pts,
                seq: 0,
                date: 0,
                gap_pending: false,
            });
    }

    /// Get a channel's tracked pts, if any.
    pub fn channel_pts_of(&self, channel_id: i64) -> Option<i32> {
        self.channel_pts.get(&channel_id).map(|s| s.pts)
    }

    /// Record the server-side qts from `updates.getState`
    /// (SPEC §6.1: qts tracks mentions separately from pts).
    pub fn set_qts(&mut self, qts: i32) {
        self.qts = qts;
    }

    /// Get the tracked qts value.
    pub fn account_qts(&self) -> i32 {
        self.qts
    }
}

impl Default for UpdateDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, Peer, MsgId};

    fn make_text_update(msg_id: i32, pts: i32, pts_count: i32) -> Update {
        Update::NewMessage {
            message: Message::Message(Box::new(crate::types::MessageFull {
                id: MsgId(msg_id as i64),
                from_id: None,
                peer_id: Peer::User { user_id: crate::types::UserId(1) },
                date: 1000,
                message: "hello".into(),
                media: None,
                reply_markup: None,
                entities: Vec::new(),
                views: None,
                edit_date: None,
                post: false,
                grouped_id: None,
                via_bot_id: None,
                reply_to: None,
                edit_hide: false,
            })),
            pts,
            pts_count,
        }
    }

    #[test]
    fn test_pts_advance() {
        let mut state = PtsState::new();
        state.pts = 10;
        assert!(state.is_ahead(10, 1)); // next expected = 10+1
        state.advance(10, 1);
        assert_eq!(state.pts, 11);
    }

    #[test]
    fn test_pts_gap_detection() {
        let mut state = PtsState::new();
        state.pts = 10;
        // Update with pts=15 means a gap (expected 11)
        assert!(!state.is_ahead(15, 1));
        assert!(!state.is_behind(15, 1));
    }

    #[test]
    fn test_pts_behind() {
        let mut state = PtsState::new();
        state.pts = 10;
        assert!(state.is_behind(5, 1));
    }

    #[test]
    fn test_dispatch_normal() {
        let mut dispatcher = UpdateDispatcher::noop();
        // pts=0 means "state was 0 before this update" — first update
        let updates = Updates::UpdateShort {
            update: make_text_update(1, 0, 1),
            date: 1000,
            seq: 0,
        };
        let dispatched = dispatcher.process_updates(updates);
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatcher.account_pts(), 1);
    }

    #[test]
    fn test_dispatch_stale() {
        let mut dispatcher = UpdateDispatcher::noop();
        dispatcher.init_state(10, 0, 1000);

        // pts=5 is behind pts=10 → stale
        let updates = Updates::UpdateShort {
            update: make_text_update(1, 5, 1),
            date: 1000,
            seq: 0,
        };
        let dispatched = dispatcher.process_updates(updates);
        assert!(dispatched.is_empty());
        assert_eq!(dispatcher.account_pts(), 10); // unchanged
    }

    #[test]
    fn test_dispatch_with_channel() {
        let (mut dispatcher, mut rx) = UpdateDispatcher::with_channel();
        // pts=0 matches the initial dispatcher state
        let updates = Updates::UpdateShort {
            update: make_text_update(1, 0, 1),
            date: 1000,
            seq: 0,
        };
        dispatcher.process_updates(updates);

        let received = rx.try_recv().unwrap();
        match received {
            Update::NewMessage { pts, .. } => assert_eq!(pts, 0),
            _ => panic!("expected NewMessage"),
        }
    }

    #[test]
    fn test_init_state() {
        let mut dispatcher = UpdateDispatcher::noop();
        dispatcher.init_state(100, 50, 500000);
        assert_eq!(dispatcher.account_pts(), 100);
        assert_eq!(dispatcher.account_seq(), 50);
        assert_eq!(dispatcher.date(), 500000);
    }

    #[test]
    fn test_gap_resolved() {
        let mut dispatcher = UpdateDispatcher::noop();
        dispatcher.init_state(10, 0, 1000);

        // Simulate a gap
        let updates = Updates::UpdateShort {
            update: make_text_update(1, 20, 1), // pts=20, gap from 10
            date: 1000,
            seq: 0,
        };
        dispatcher.process_updates(updates);
        assert!(dispatcher.needs_difference());
        assert_eq!(dispatcher.buffered_count(), 1);

        // Resolve the gap
        dispatcher.gap_resolved(21, 0, 1000);
        assert!(!dispatcher.needs_difference());
        assert_eq!(dispatcher.buffered_count(), 0);
    }

    #[test]
    fn test_multiple_dispatch() {
        let mut dispatcher = UpdateDispatcher::noop();
        // Sequential updates: pts 0→1→2→3
        let updates = Updates::Updates {
            updates: vec![
                make_text_update(1, 0, 1), // state was 0, becomes 1
                make_text_update(2, 1, 1), // state was 1, becomes 2
                make_text_update(3, 2, 1), // state was 2, becomes 3
            ],
            users: Vec::new(),
            chats: Vec::new(),
            date: 1000,
            seq: 0,
        };
        let dispatched = dispatcher.process_updates(updates);
        assert_eq!(dispatched.len(), 3);
        assert_eq!(dispatcher.account_pts(), 3);
    }

    #[test]
    fn test_channel_pts() {
        let mut dispatcher = UpdateDispatcher::noop();
        dispatcher.init_channel_pts(12345, 100);
        assert_eq!(dispatcher.channel_pts.get(&12345).unwrap().pts, 100);
        assert!(!dispatcher.channel_needs_difference(12345));
    }

    #[test]
    fn test_channel_too_long_queued() {
        let mut dispatcher = UpdateDispatcher::noop();
        let updates = Updates::UpdateShort {
            update: Update::ChannelTooLong {
                channel_id: crate::types::ChannelId(42),
                pts: Some(7),
            },
            date: 1000,
            seq: 0,
        };
        dispatcher.process_updates(updates);
        assert_eq!(dispatcher.take_channels_too_long(), vec![42]);
        // Drained; also seeded the channel pts from the update.
        assert!(dispatcher.take_channels_too_long().is_empty());
        assert_eq!(dispatcher.channel_pts_of(42), Some(7));

        // advance_channel_pts stores absolute pts for unknown channels too
        dispatcher.advance_channel_pts(99, 500);
        assert_eq!(dispatcher.channel_pts_of(99), Some(500));
    }

    #[test]
    fn test_qts_tracking() {
        let mut dispatcher = UpdateDispatcher::noop();
        assert_eq!(dispatcher.account_qts(), 0);
        dispatcher.set_qts(33);
        assert_eq!(dispatcher.account_qts(), 33);
    }
}
