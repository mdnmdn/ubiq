//! The one status vocabulary an agent and a delegate both speak: a lifecycle, an activity, and
//! the pair of them.
//!
//! **Two dictionaries, because they answer two questions.** A [`Lifecycle`] answers whether an
//! execution *can continue*; a [`Doing`] answers what it is *doing*, or — once it cannot continue —
//! what happened to the work. They are independent, and every surface that reports an agent or a
//! delegate reads both: `Working \u{b7} Tools` is an agent handling a tool call, `Ended \u{b7} Done` is a
//! delegate whose work came back, and neither is expressible as one enum without inventing a
//! state that means two things.
//!
//! **`Done` is not a lifecycle and `Running` is not an activity.** A delegate reporting `Done` has
//! lifecycle [`Lifecycle::Ended`]: its work finished, so it is no longer running, and nothing that
//! counts active executions may count it. The word `running` survives only as a compact chip
//! reading, derived from an active pair — never as the state itself.
//!
//! **Nothing here draws and nothing here names a colour.** A pair says what it *is*; `ui::work`
//! decides which token that reads in, and `theme` keeps the values. That is the same split
//! `super::teams` and `ubiq_proto::work` already hold to, and it is what lets the chat panel, the
//! agents column and the Teams hexagon show one reading of one fact.

use ubiq_proto::conversation::{StopReason, ToolStatus};
use ubiq_proto::work::{Activity, Bucket, WorkAgent};

use super::conversation::{Conversation, Run, SubagentTab};

/// Whether an execution can continue, and on what.
///
/// **`Unloaded` and `Starting` are both "not launched".** What tells them apart is the transcript,
/// not a flag: a harness that is gone still leaves what it said, a harness never started leaves
/// nothing. [`conversation_status`] tests that rather than carrying a second field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifecycle {
    /// Being prepared, with no usable runtime state yet.
    Starting,
    /// Configured, and able to accept its first turn.
    Ready,
    /// Alive, able to accept another turn, with none running.
    Idle,
    /// A turn or a delegated execution is in flight.
    Working,
    /// Blocked on a human response or a permission answer before it can proceed. Outranks
    /// [`Self::Working`]: it is the one state that needs the reader to do something.
    Waiting,
    /// The conversation is preserved; its harness is not running.
    Unloaded,
    /// Takes no more work.
    Ended,
}

impl Lifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Lifecycle::Starting => "Starting",
            Lifecycle::Ready => "Ready",
            Lifecycle::Idle => "Idle",
            Lifecycle::Working => "Working",
            Lifecycle::Waiting => "Waiting",
            Lifecycle::Unloaded => "Unloaded",
            Lifecycle::Ended => "Ended",
        }
    }

    /// Whether the execution is going somewhere — which is what a pulse means, and the one
    /// question an active count may ask. `Ended` is never active, whatever its result says.
    pub fn active(self) -> bool {
        matches!(self, Lifecycle::Working | Lifecycle::Waiting)
    }
}

/// What an execution is doing, or — once it has ended — what came of the work.
///
/// One dictionary for both because a surface reads one slot: while an execution runs that slot
/// says what it is busy with, and when it stops it says how it stopped. `Unknown` is the honest
/// answer whenever the events in hand identify neither, and it is drawn as no glyph rather than as
/// a guess.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Doing {
    /// Accepted, not started.
    Queued,
    /// Reasoning, with no tool call attributed to it.
    Thinking,
    /// Producing prose.
    Writing,
    /// Handling a tool call.
    Tools,
    /// Blocked on a human response.
    NeedsYou,
    /// A delegated execution completed successfully.
    Done,
    /// A delegated execution completed with an error.
    Failed,
    /// The events in hand do not identify the activity or the result.
    Unknown,
}

impl Doing {
    pub fn label(self) -> &'static str {
        match self {
            Doing::Queued => "Queued",
            Doing::Thinking => "Thinking",
            Doing::Writing => "Writing",
            Doing::Tools => "Tools",
            Doing::NeedsYou => "Needs you",
            Doing::Done => "Done",
            Doing::Failed => "Error",
            Doing::Unknown => "Unknown",
        }
    }

    fn from_activity(activity: Activity) -> Self {
        match activity {
            Activity::Thinking => Doing::Thinking,
            Activity::Writing => Doing::Writing,
            Activity::Tools => Doing::Tools,
            Activity::NeedsYou => Doing::NeedsYou,
            // The record's `Ended` collapses "stopped" into the activity slot, which is a
            // lifecycle fact; read as an activity it says nothing.
            Activity::Ended => Doing::Unknown,
            Activity::Failed => Doing::Failed,
        }
    }
}

/// The pair, which is what every surface actually reports.
///
/// A card's edge, a dot and a hexagon's border take the lifecycle; a chip, a glyph and a
/// hexagon's inner fill take the activity. The two move independently, which is the point: a
/// result arriving does not erase what the execution was doing, and a lifecycle transition does
/// not falsely claim the work is still going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub lifecycle: Lifecycle,
    pub doing: Doing,
}

impl Status {
    pub fn new(lifecycle: Lifecycle, doing: Doing) -> Self {
        Self { lifecycle, doing }
    }

    /// The precise reading, for a tooltip and an accessible label: `Working \u{b7} Tools`,
    /// `Ended \u{b7} Done`. The lifecycle alone where the activity says nothing — `Unknown \u{b7} Idle`
    /// would be two words for one fact.
    pub fn label(self) -> String {
        match self.doing {
            Doing::Unknown => self.lifecycle.label().to_string(),
            doing => format!("{} \u{b7} {}", self.lifecycle.label(), doing.label()),
        }
    }

    /// The one word a chip has room for. The activity where there is one, because *what* an
    /// execution is doing is the more useful of the two at a glance; the lifecycle where there is
    /// not.
    pub fn chip(self) -> &'static str {
        match self.doing {
            Doing::Unknown => self.lifecycle.label(),
            doing => doing.label(),
        }
    }

    /// Whether this execution is going somewhere — [`Lifecycle::active`], and nothing else. A
    /// delegate reading `Done` or `Failed` is `Ended`, so it is never counted and never pulses.
    pub fn active(self) -> bool {
        self.lifecycle.active()
    }

    /// Which filter bucket the pair reads in — the coarse projection the status bar counts and the
    /// Teams rail filters on. Derived from the pair rather than stored beside it, so a filter can
    /// never disagree with the mark.
    pub fn bucket(self) -> Bucket {
        match (self.lifecycle, self.doing) {
            (_, Doing::Failed) => Bucket::Error,
            (Lifecycle::Waiting, _) => Bucket::Waiting,
            (Lifecycle::Working, _) => Bucket::Running,
            (Lifecycle::Idle | Lifecycle::Ready, _) => Bucket::Waiting,
            (Lifecycle::Starting | Lifecycle::Unloaded | Lifecycle::Ended, _) => Bucket::Ended,
        }
    }
}

/// A live conversation's pair, read off the fields it already carries.
///
/// Order matters, and it is the order a reader needs: ended outranks everything (a harness taking
/// no more turns is not "working" because a race left `run` behind), a question outranks the turn
/// it blocks, a turn in flight outranks idle, and only then does whether it ever launched — and,
/// if not, whether it left a transcript — decide the rest.
pub fn conversation_status(conversation: &Conversation) -> Status {
    if conversation.run == Run::Ended || !conversation.accepts_input {
        let doing = match conversation.stop_reason {
            Some(StopReason::Failed) => Doing::Failed,
            _ => Doing::Unknown,
        };
        return Status::new(Lifecycle::Ended, doing);
    }
    if !conversation.pending.is_empty() {
        return Status::new(Lifecycle::Waiting, Doing::NeedsYou);
    }
    if conversation.run == Run::Working {
        return Status::new(
            Lifecycle::Working,
            Doing::from_activity(conversation.activity()),
        );
    }
    if conversation.launched {
        return Status::new(Lifecycle::Idle, Doing::Unknown);
    }
    if !conversation.blocks.is_empty() {
        return Status::new(Lifecycle::Unloaded, Doing::Unknown);
    }
    if conversation.config.is_empty() {
        Status::new(Lifecycle::Starting, Doing::Unknown)
    } else {
        Status::new(Lifecycle::Ready, Doing::Unknown)
    }
}

/// One delegate's pair: the question it is blocked on if it has one, its spawning call's terminal
/// word if that call has stated one, and otherwise what its own last line was doing.
///
/// **A completion is terminal, and it ends the delegate.** `ToolStatus::Completed` is
/// `Ended \u{b7} Done` and `Failed` is `Ended \u{b7} Error`: the delegated work is back, so the delegate is
/// not running and no active count may include it. A late permission request does not undo that —
/// the `waiting` test is below the terminal one for exactly that reason.
///
/// **Silence never proves `Done`.** A spawning call the transcript does not hold gives
/// `Starting \u{b7} Unknown` — the delegate exists, nothing says it finished, and nothing here invents
/// either half.
pub fn delegate_status(tab: &SubagentTab) -> Status {
    match tab.status {
        Some(ToolStatus::Completed) => return Status::new(Lifecycle::Ended, Doing::Done),
        Some(ToolStatus::Failed) => return Status::new(Lifecycle::Ended, Doing::Failed),
        _ => {}
    }
    if tab.waiting > 0 {
        return Status::new(Lifecycle::Waiting, Doing::NeedsYou);
    }
    match tab.status {
        Some(ToolStatus::Pending) => Status::new(Lifecycle::Starting, Doing::Queued),
        Some(ToolStatus::InProgress) => Status::new(Lifecycle::Working, tab.doing),
        // Handled above; repeated rather than unreachable, because a terminal status is a fact
        // this function states once.
        Some(ToolStatus::Completed) => Status::new(Lifecycle::Ended, Doing::Done),
        Some(ToolStatus::Failed) => Status::new(Lifecycle::Ended, Doing::Failed),
        None => Status::new(Lifecycle::Starting, Doing::Unknown),
    }
}

/// An agent's pair, read off the live conversation where the window holds one and off the host's
/// record where it does not.
///
/// The conversation is the better witness — it is the stream itself, and the record is the host's
/// periodic reading of it — so it wins wherever it exists. The record's `Activity` carries no
/// lifecycle, which is the whole reason a card drawn from one cannot tell a harness that stopped
/// from a turn that did.
pub fn agent_status(agent: &WorkAgent, conversation: Option<&Conversation>) -> Status {
    let Some(conversation) = conversation else {
        return match agent.activity {
            Activity::NeedsYou => Status::new(Lifecycle::Waiting, Doing::NeedsYou),
            Activity::Ended => Status::new(Lifecycle::Ended, Doing::Unknown),
            Activity::Failed => Status::new(Lifecycle::Ended, Doing::Failed),
            activity => Status::new(Lifecycle::Working, Doing::from_activity(activity)),
        };
    };
    conversation_status(conversation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(status: Option<ToolStatus>, waiting: usize, doing: Doing) -> SubagentTab {
        SubagentTab {
            id: "t1".to_string(),
            name: "worker".to_string(),
            status,
            kind: None,
            model: None,
            thinking: None,
            waiting,
            activity: None,
            doing,
        }
    }

    /// The card this model exists for: a delegate that came back is *ended*, whatever else the
    /// transcript still says, and nothing may count it as running.
    #[test]
    fn a_completed_delegate_is_ended_and_never_active() {
        let status = delegate_status(&tab(Some(ToolStatus::Completed), 0, Doing::Tools));
        assert_eq!(status, Status::new(Lifecycle::Ended, Doing::Done));
        assert!(!status.active());
        assert_eq!(status.chip(), "Done");
        assert_eq!(status.label(), "Ended \u{b7} Done");
    }

    #[test]
    fn a_failed_delegate_is_ended_with_an_error_result() {
        let status = delegate_status(&tab(Some(ToolStatus::Failed), 0, Doing::Unknown));
        assert_eq!(status, Status::new(Lifecycle::Ended, Doing::Failed));
        assert!(!status.active());
        assert_eq!(status.bucket(), Bucket::Error);
    }

    /// A completion is terminal: a permission request that arrives after it is a protocol
    /// inconsistency, not a reason to claim the delegate is alive again.
    #[test]
    fn a_late_permission_does_not_revive_a_completed_delegate() {
        assert_eq!(
            delegate_status(&tab(Some(ToolStatus::Completed), 2, Doing::Unknown)),
            Status::new(Lifecycle::Ended, Doing::Done)
        );
    }

    #[test]
    fn a_blocked_delegate_waits_on_the_human_over_whatever_it_was_doing() {
        let status = delegate_status(&tab(Some(ToolStatus::InProgress), 1, Doing::Tools));
        assert_eq!(status, Status::new(Lifecycle::Waiting, Doing::NeedsYou));
        assert!(status.active());
    }

    /// Silence is not success: a spawning call the transcript never held says so rather than
    /// claiming either half of a pair it cannot read.
    #[test]
    fn an_unreported_delegate_claims_nothing() {
        let status = delegate_status(&tab(None, 0, Doing::Unknown));
        assert_eq!(status, Status::new(Lifecycle::Starting, Doing::Unknown));
        assert!(!status.active());
        assert_eq!(status.chip(), "Starting");
        assert_eq!(status.label(), "Starting");
    }

    #[test]
    fn a_working_delegate_reports_its_own_last_line() {
        assert_eq!(
            delegate_status(&tab(Some(ToolStatus::InProgress), 0, Doing::Writing)),
            Status::new(Lifecycle::Working, Doing::Writing)
        );
    }
}
