//! What the Control screen is looking at: which of its two tabs is up, and the last reading the
//! host answered with.
//!
//! **Nothing here belongs to a project.** The figures are the host's own — its memory, how long it
//! has been running, how many projects it holds open across every window — so this sits on the
//! window beside the sink's state rather than inside an open project, for the same reason: a
//! window with an empty catalogue can open Control and it still has something true to say.
//!
//! The reading is an [`Option`] and the distinction matters. `None` is "nothing has been asked
//! yet, or nothing has come back", which is not a host using no memory and running no agents. The
//! screen draws the two differently, because a zero that means "we have not asked" is a lie.

use ubiq_proto::stats::HostStats;

/// One page of the Control screen. Two, and the split is what they are made of: the first is the
/// host reporting on itself, the second is what its agents have spent.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StatsTab {
    #[default]
    General,
    Ai,
}

impl StatsTab {
    /// The two, in the order the strip draws them.
    pub fn all() -> &'static [StatsTab] {
        &[StatsTab::General, StatsTab::Ai]
    }

    /// What the strip's tab says.
    pub fn label(self) -> &'static str {
        TAB_COPY[self as usize].0
    }

    /// The one line beside the strip: what this page is reporting.
    pub fn note(self) -> &'static str {
        TAB_COPY[self as usize].1
    }
}

/// The tab and the line beside it, one row per [`StatsTab`], in variant order.
const TAB_COPY: [(&str, &str); 2] = [
    ("General", "What the host is doing right now."),
    ("AI", "What the agents have spent, by the hour."),
];

/// What the Control screen holds between frames.
#[derive(Default)]
pub struct StatsState {
    pub tab: StatsTab,
    /// The last reading the host answered with. `None` until the first answer arrives — see the
    /// module note: it is not a reading of zero.
    pub host: Option<HostStats>,
    /// Whether a refresh loop is already running for this window. One flag, because opening the
    /// screen twice must not leave two timers asking.
    pub polling: bool,
}
