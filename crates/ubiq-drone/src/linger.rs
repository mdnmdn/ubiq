//! How long a drone with no client waits before it gives up.
//!
//! One knob, three answers, and it is the **drone's** timer and never Ubiq's: the whole point of
//! a detached drone is that the interface may be gone — closed, asleep, or on the other side of a
//! link that dropped — so a countdown held where the user is would never run when it is needed.
//!
//! It is passed at launch and re-asserted on every attach (see [`crate::socket`]), so changing it
//! costs no restart and no pane.

use std::fmt;
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
