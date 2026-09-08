//! The bell's one source of truth: what has been raised, what is silenced, and who is told.
//!
//! Every notification in Ubiq is filed here, whichever half raised it, because two things have to
//! be decided in exactly one place. **Whether a mute rule silences it** — a rule the user set in
//! one window governs every window. And **whether the desktop hears about it** — one operating
//! system notification per event, however many windows are open.
//!
//! The centre holds no bus and draws nothing: it answers in [`Reply`]s and the coordinator
//! addresses them, which is what keeps it testable without a host. See
//! `_docs/features/notifications.md`.

pub mod os;

use std::time::{SystemTime, UNIX_EPOCH};

use ubiq_proto::ids::NotificationId;
use ubiq_proto::messages::Message;
use ubiq_proto::notifications::{
    HISTORY_CAP, Level, MuteFor, MuteRule, MuteScope, Notification, NotificationRequest,
    Notifications,
};

use crate::reply::Reply;

/// Milliseconds since the Unix epoch. A clock before 1970 reads as 0 rather than panicking — a
/// notification with a silly timestamp is better than a host that stops.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The history and the rules over it.
///
/// Held by the coordinator, on its thread. Nothing here blocks: the one call that could — telling
/// the desktop — is handed to [`os::post`], which runs it on a thread of its own.
#[derive(Default)]
pub struct Centre {
    /// Newest first, capped at [`HISTORY_CAP`].
    items: Vec<Notification>,
    /// At most one rule per scope.
    mutes: Vec<MuteRule>,
}

impl Centre {
    pub fn new() -> Self {
        Self::default()
    }

    /// File a request: decide whether it is silenced, mint its id, tell the desktop if it asked
    /// and is not silenced, and tell every window.
    ///
    /// A silenced notification is still filed and still counts against the badge. Muting is about
    /// *interruption* — the flash and the desktop — never about hiding what happened.
    pub fn raise(&mut self, request: NotificationRequest) -> Vec<Reply> {
        let now = now_ms();
        self.drop_lapsed(now);

        let muted = request.muted
            || self.mutes.iter().any(|rule| {
                rule.silences(
                    request.level,
                    request.family,
                    request.actor.as_deref(),
                    request.category.as_deref(),
                    now,
                )
            });

        if request.os && !muted {
            os::post(request.level, request.family, &request.text);
        }

        let notification = Notification {
            id: NotificationId::generate(),
            level: request.level,
            family: request.family,
            actor: request.actor,
            category: request.category,
            text: request.text,
            link: request.link,
            at: now,
            muted,
            read: false,
        };

        self.items.insert(0, notification.clone());
        self.items.truncate(HISTORY_CAP);

        vec![Reply::Everyone(Message::NotificationRaised {
            notification: Box::new(notification),
        })]
    }

    /// The whole state, for the window that asked.
    pub fn list(&mut self) -> Vec<Reply> {
        self.drop_lapsed(now_ms());
        vec![Reply::Asker(self.state_message())]
    }

    /// Tick one off, or every one of them.
    pub fn read(&mut self, id: Option<NotificationId>) -> Vec<Reply> {
        for item in &mut self.items {
            if id.is_none_or(|wanted| wanted == item.id) {
                item.read = true;
            }
        }
        self.broadcast_state()
    }

    /// Drop one from the history, or every one of them.
    pub fn dismiss(&mut self, id: Option<NotificationId>) -> Vec<Reply> {
        match id {
            Some(wanted) => self.items.retain(|item| item.id != wanted),
            None => self.items.clear(),
        }
        self.broadcast_state()
    }

    /// Stand a rule up on a scope, replacing whatever rule that scope already had.
    pub fn mute(&mut self, scope: MuteScope, max_level: Level, duration: MuteFor) -> Vec<Reply> {
        let now = now_ms();
        self.drop_lapsed(now);
        let until = duration.minutes().map(|m| now + m * 60_000);
        self.mutes.retain(|rule| rule.scope != scope);
        self.mutes.push(MuteRule {
            scope,
            max_level,
            until,
        });
        self.broadcast_state()
    }

    /// Lift the rule on a scope. Lifting a scope with no rule is not an error.
    pub fn unmute(&mut self, scope: MuteScope) -> Vec<Reply> {
        self.drop_lapsed(now_ms());
        self.mutes.retain(|rule| rule.scope != scope);
        self.broadcast_state()
    }

    /// What the bell draws right now.
    pub fn state(&self) -> Notifications {
        Notifications {
            items: self.items.clone(),
            mutes: self.mutes.clone(),
        }
    }

    fn state_message(&self) -> Message {
        Message::NotificationsState {
            state: Box::new(self.state()),
        }
    }

    fn broadcast_state(&mut self) -> Vec<Reply> {
        self.drop_lapsed(now_ms());
        vec![Reply::Everyone(self.state_message())]
    }

    /// A rule whose moment has passed is gone, not merely inert — so the list the user reads is
    /// the list that is in force.
    fn drop_lapsed(&mut self, now: u64) {
        self.mutes
            .retain(|rule| rule.until.is_none_or(|until| now < until));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::notifications::Family;

    fn raised(replies: &[Reply]) -> &Notification {
        match replies[0].message() {
            Message::NotificationRaised { notification } => notification,
            other => panic!("expected a raise, got {other:?}"),
        }
    }

    fn state_of(replies: &[Reply]) -> &Notifications {
        match replies[0].message() {
            Message::NotificationsState { state } => state,
            other => panic!("expected a state, got {other:?}"),
        }
    }

    #[test]
    fn a_raise_is_filed_newest_first_and_told_to_everyone() {
        let mut centre = Centre::new();
        centre.raise(NotificationRequest::info(Family::Agents, "first"));
        let replies = centre.raise(NotificationRequest::info(Family::Agents, "second"));

        assert!(replies[0].is_broadcast());
        assert_eq!(raised(&replies).text, "second");
        assert_eq!(centre.state().items[0].text, "second");
        assert_eq!(centre.state().unread(), 2);
    }

    #[test]
    fn a_rule_silences_without_hiding() {
        let mut centre = Centre::new();
        centre.mute(
            MuteScope::Family(Family::Agents),
            Level::Warning,
            MuteFor::Hour1,
        );

        let quiet = centre.raise(NotificationRequest::info(Family::Agents, "tool call"));
        assert!(raised(&quiet).muted);
        let loud = centre.raise(NotificationRequest::error(Family::Agents, "crashed"));
        assert!(!raised(&loud).muted);
        let elsewhere = centre.raise(NotificationRequest::info(Family::Git, "fetched"));
        assert!(!raised(&elsewhere).muted);

        // Silenced, not hidden: both still count.
        assert_eq!(centre.state().unread(), 3);
    }

    #[test]
    fn a_scope_holds_one_rule() {
        let mut centre = Centre::new();
        let scope = MuteScope::Family(Family::Git);
        centre.mute(scope.clone(), Level::Info, MuteFor::Minutes5);
        let replies = centre.mute(scope.clone(), Level::Error, MuteFor::Hour1);

        let mutes = &state_of(&replies).mutes;
        assert_eq!(mutes.len(), 1);
        assert_eq!(mutes[0].max_level, Level::Error);

        let replies = centre.unmute(scope);
        assert!(state_of(&replies).mutes.is_empty());
    }

    #[test]
    fn reading_and_dismissing_take_one_or_all() {
        let mut centre = Centre::new();
        centre.raise(NotificationRequest::info(Family::Host, "a"));
        centre.raise(NotificationRequest::info(Family::Host, "b"));
        let first = centre.state().items[0].id;

        let replies = centre.read(Some(first));
        assert_eq!(state_of(&replies).unread(), 1);
        let replies = centre.read(None);
        assert_eq!(state_of(&replies).unread(), 0);

        let keep = centre.state().items[1].id;
        let replies = centre.dismiss(Some(keep));
        assert_eq!(state_of(&replies).items.len(), 1);
        let replies = centre.dismiss(None);
        assert!(state_of(&replies).items.is_empty());
    }

    #[test]
    fn the_history_is_capped() {
        let mut centre = Centre::new();
        for n in 0..HISTORY_CAP + 10 {
            centre.raise(NotificationRequest::info(Family::Ubiq, format!("{n}")));
        }
        let state = centre.state();
        assert_eq!(state.items.len(), HISTORY_CAP);
        assert_eq!(state.items[0].text, format!("{}", HISTORY_CAP + 9));
    }
}
