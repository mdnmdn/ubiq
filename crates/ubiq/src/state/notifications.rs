//! What the bell is showing: the host's state as last broadcast, and the window's own view of it.
//!
//! **The host's half is never edited here.** `wire` is a copy of what the host filed, and every
//! change — reading one off, dropping one, standing a mute rule up — is a message out and a fresh
//! state back. Two windows that both drew the bell from their own edits would drift; two windows
//! that redraw from one broadcast cannot.
//!
//! Everything else in here is this window's: whether the list is on screen, whether the bell is
//! mid-flash, and the half-made mute rule the picker is collecting.

use std::time::{Duration, Instant};

use ubiq_proto::notifications::{Level, MuteScope, Notifications, UbiqLink};

/// How long the bell keeps flashing after something loud arrives.
///
/// Long enough to catch an eye that was elsewhere, short enough that a bell still flashing minutes
/// later would be furniture rather than news — the badge is what carries the fact afterwards.
pub const FLASH: Duration = Duration::from_secs(3);

/// How long one half of a blink lasts. A flash that only changed colour reads as a state, not as
/// an event; alternating at roughly three beats a second is what makes it read as arriving.
pub const BLINK: Duration = Duration::from_millis(350);

/// The mute rule being built, while the user is choosing what it covers and how loud it lets
/// through. The duration is chosen last, because picking one is what sends the rule.
pub struct MutePick {
    pub scope: MuteScope,
    pub max_level: Level,
}

impl MutePick {
    /// A rule for one scope, silencing warnings and below until the user says otherwise.
    ///
    /// Warning rather than `Error` because a mute that swallowed errors by default would be a
    /// setting nobody chose, and rather than `Info` because "quieten this" almost always means
    /// the chatter and not only the very quietest of it.
    pub fn new(scope: MuteScope) -> Self {
        Self {
            scope,
            max_level: Level::Warning,
        }
    }
}

/// What the bell knows between frames.
#[derive(Default)]
pub struct NotificationsState {
    /// The host's whole state, as last broadcast. The interface never edits it — every change is
    /// a message to the host and a new state coming back.
    pub wire: Notifications,
    /// Whether the list is on screen.
    pub open: bool,
    /// When the bell stops flashing. `None` is a bell at rest, and is also what says no flash loop
    /// is running: an arrival while it is `Some` pushes the moment out rather than starting a
    /// second timer.
    pub flash_until: Option<Instant>,
    /// Alternates while the bell flashes, so the icon actually blinks rather than merely changing
    /// colour.
    pub flash_on: bool,
    /// The newest unmuted notification's level and link. Clicking a flashing bell that carries a
    /// link goes straight there instead of opening the list.
    pub flash_level: Option<Level>,
    pub flash_link: Option<UbiqLink>,
    /// The mute picker, when the user is choosing a scope, a level and a duration.
    pub muting: Option<MutePick>,
}
