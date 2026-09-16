//! How long a drone with no client waits before it gives up.
//!
//! One knob, three answers, and it is the **drone's** timer and never Ubiq's: the whole point of
//! a detached drone is that the interface may be gone — closed, asleep, or on the other side of a
//! link that dropped — so a countdown held where the user is would never run when it is needed.
//!
//! It is passed at launch and re-asserted on every attach (see [`crate::socket`]), so changing it
//! costs no restart and no pane.

use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// What a drone does once the last client has detached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linger {
    /// Exit as soon as the last client leaves. An attached drone's lifetime, with a socket.
    Immediate,
    /// Survive a dropped link for this long, then kill the panes and exit.
    For(Duration),
    /// Wait forever: a managed drone, stopped deliberately and never by silence.
    Never,
}

/// Ten minutes: long enough to survive a laptop closing, a network changing or a machine paging
/// back in, short enough that a forgotten drone is not a resident process on somebody else's
/// machine.
pub const DEFAULT: Duration = Duration::from_secs(600);

impl Default for Linger {
    fn default() -> Self {
        Self::For(DEFAULT)
    }
}

impl Linger {
    /// Read the knob. A bare number is **seconds** — the unit `DroneOrigin.linger_secs` will
    /// carry — with `s`, `m` and `h` accepted for the same reason a human writes `10m`.
    pub fn parse(given: &str) -> Result<Self, String> {
        let given = given.trim();
        if given.eq_ignore_ascii_case("never") {
            return Ok(Self::Never);
        }
        let (digits, scale) = match given.as_bytes().last() {
            Some(b's' | b'S') => (&given[..given.len() - 1], 1),
            Some(b'm' | b'M') => (&given[..given.len() - 1], 60),
            Some(b'h' | b'H') => (&given[..given.len() - 1], 3600),
            _ => (given, 1),
        };
        let count: u64 = digits
            .trim()
            .parse()
            .map_err(|_| format!("{given} is not a number of seconds, nor `never`"))?;
        Ok(match count.saturating_mul(scale) {
            0 => Self::Immediate,
            seconds => Self::For(Duration::from_secs(seconds)),
        })
    }

    /// Whether a drone that has been alone for `alone` should stop now.
    pub fn expired(self, alone: Duration) -> bool {
        match self {
            Self::Immediate => true,
            Self::For(window) => alone >= window,
            Self::Never => false,
        }
    }
}

impl fmt::Display for Linger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Immediate => write!(f, "0"),
            Self::For(window) => write!(f, "{}s", window.as_secs()),
            Self::Never => write!(f, "never"),
        }
    }
}

/// What a listening drone's accept loop and its relay share, in place of the `Arc<Mutex<Linger>>`
/// phase 6 got away with.
///
/// Phase 6 had one fact to share across threads — the linger, re-asserted on every attach — and a
/// mutex around it was the whole of the machinery. Phase 7 adds two more: the pane count a state
/// file reports without asking the relay for it, and a stop flag `--stop` sets from a session
/// thread that never touches the relay's own loop otherwise. Bundling the three keeps one thing
/// alive across `socket::listen`'s threads instead of three, and keeps the panes-and-stopping
/// counters lock-free since nothing about them needs the mutex's ordering.
pub struct Live {
    linger: Mutex<Linger>,
    panes: AtomicUsize,
    stopping: AtomicBool,
}

impl Live {
    /// A drone about to listen, with the linger it was launched with and no panes yet.
    pub fn new(linger: Linger) -> Self {
        Self {
            linger: Mutex::new(linger),
            panes: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
        }
    }

    /// The linger as it stands right now — re-asserted on every attach, so this can change between
    /// one read and the next.
    pub fn linger(&self) -> Linger {
        *self.linger.lock().expect("the linger")
    }

    /// An attach re-asserting the knob.
    pub fn set_linger(&self, linger: Linger) {
        *self.linger.lock().expect("the linger") = linger;
    }

    /// How many panes the relay is holding, as of its last tick.
    pub fn panes(&self) -> usize {
        self.panes.load(Ordering::Relaxed)
    }

    /// The relay's own count, kept here so a state file or a `--status` answer can be written
    /// without asking the relay's thread for it.
    pub fn set_panes(&self, panes: usize) {
        self.panes.store(panes, Ordering::Relaxed);
    }

    /// `--stop` asked, over the socket, for this process to end.
    ///
    /// It sets a flag rather than killing anything itself: the relay's own tick is the one place
    /// panes are killed and the socket unlinked, and a second path to that would be the second
    /// shutdown path the doc says never to add.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }

    /// Whether `--stop` has been asked for. The relay treats this as an immediate expiry, whatever
    /// the linger says.
    pub fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_knob_reads_seconds_suffixes_and_never() {
        assert_eq!(Linger::parse("0"), Ok(Linger::Immediate));
        assert_eq!(
            Linger::parse("30"),
            Ok(Linger::For(Duration::from_secs(30)))
        );
        assert_eq!(Linger::parse("10m"), Ok(Linger::For(DEFAULT)));
        assert_eq!(
            Linger::parse("2h"),
            Ok(Linger::For(Duration::from_secs(7200)))
        );
        assert_eq!(Linger::parse("NEVER"), Ok(Linger::Never));
        assert!(Linger::parse("soon").is_err());
        // `0m` is still zero: the unit cannot make a wait out of no wait.
        assert_eq!(Linger::parse("0m"), Ok(Linger::Immediate));
    }

    #[test]
    fn expiry_is_the_whole_of_the_lifecycle() {
        assert!(Linger::Immediate.expired(Duration::ZERO));
        assert!(!Linger::Never.expired(Duration::from_secs(86_400)));
        let ten = Linger::For(Duration::from_secs(10));
        assert!(!ten.expired(Duration::from_secs(9)));
        assert!(ten.expired(Duration::from_secs(10)));
    }
}
