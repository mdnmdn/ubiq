//! The conflict table, the local side's hashes, and the patch a push is made of.
//!
//! `sync` runs the pass; this module decides what the pass should do. It is split out because
//! **what a pass decides is a pure function of two hashes and a switch**, and a rule table that
//! can be read and tested without a board, a thread or a network is the only form in which the
//! rule is checkable at all.
//!
//! ## The conflict table (`D188`)
//!
//! Each field of each linked task is in one of four states, derived by comparing the task and the
//! fresh remote item against the *two* hash maps the link row carries — [`TaskLink::hash`], what
//! the remote said last pass, and [`TaskLink::local`], what the task said last pass.
//!
//! | Local changed | Remote changed | What happens |
//! |---|---|---|
//! | no | no | nothing |
//! | no | yes | **pull** — the remote value is written to the task |
//! | yes | no | **push** — the local value is written to the remote |
//! | yes | yes | the binding's [`Authority`] decides, the loser's value is kept in a comment, and the task is filed [`LinkState::Drifted`] |
//!
//! Four rules qualify it, and each is a rule of the *layer* rather than of any provider:
//!
//! - **A `Pull`-only binding never pushes.** `R13`: outbound is off until the user turns it on, so
//!   under [`Direction::Pull`] every `push` cell becomes *leave the local value alone and tell the
//!   remote nothing*, and files **no drift row** — a local edit there is the user's to keep,
//!   nothing will ever be done about it, and flagging it would leave every edited card on every
//!   pull-only board permanently drifted. It is still a local *change*, so if the remote later
//!   moves the same field the table reaches its fourth row and the switch decides.
//! - **A field the provider cannot write is never attempted, and never dropped either.** The
//!   capability is checked while the patch is built, the field is left out of it, and a
//!   [`FieldDrift`] with `settles: None` records that the two sides differ and no switch setting
//!   will close it. Greying a control and silently discarding a value are not the same thing, and
//!   only the first is honest.
//! - **Four fields are pull-only by construction**: `key` and `url` are minted by the remote
//!   (`R11`), and `kind` and `priority` are *derived* through the binding's maps from a label or a
//!   type hint — pushing one would mean running the map backwards, and a many-to-one map has no
//!   inverse. They take part in the table's first three rows and in the fourth, but their `push`
//!   outcome is always "leave the task alone", exactly as a pull-only binding's is.
//! - **A refused write is never retried by overwriting.** A provider that rejects the write —
//!   because the revision moved, whether as a precondition it honours or as the re-read it
//!   emulates one with (`R8`) — files the row [`LinkState::Conflict`] and nothing is written on
//!   either side. A human decides.
//!
//! **Nothing here deletes anything**, and a push that fails destroys no local content: the patch
//! is built from the task, the provider is asked, and a refusal leaves the task exactly as it was.
//!
//! ## Why two hash maps rather than one
//!
//! The obvious shape — hash the task in the *remote's* vocabulary and compare it against the one
//! map `D185` already files — does not work, and the lane is why: the lane map is many-to-one, so
//! two lists that both mean `Done` are the same task state and different remote values, and a task
//! sitting still would read as changed on every pass. Normalising either side onto the other would
//! also make every mapping decision a hashing decision. So each side is hashed in its own words
//! and only ever compared against its own previous value.

use std::collections::BTreeMap;

use ubiq_proto::tasksrc::{
    Authority, Binding, Direction, DriftSide, FieldDrift, ProviderCaps, RemoteItem, RemotePatch,
};
use ubiq_proto::work::TaskRecord;

use super::sync::{
    FIELD_ASSIGNEES, FIELD_BODY, FIELD_KEY, FIELD_KIND, FIELD_LABELS, FIELD_LANE, FIELD_PRIORITY,
    FIELD_TITLE, FIELD_URL, digest,
};

/// The fields the conflict table covers, in the order a drift overview reads best.
///
/// `checklist` and `comments` are **not** here. They are slice 7's, and they are also the two the
/// per-call capability problem (`T-237`) actually bites: a provider may fill them on one endpoint
/// and not on another, so an empty list is not a claim that there are none — which makes "did the
/// remote change it" unanswerable from a read alone. Every field below is filled by every call
/// that returns a [`RemoteItem`] at all, so its absence is a value and not a silence.
pub const FIELDS: &[&str] = &[
    FIELD_TITLE,
    FIELD_BODY,
    FIELD_LANE,
    FIELD_LABELS,
    FIELD_ASSIGNEES,
    FIELD_KIND,
    FIELD_PRIORITY,
    FIELD_KEY,
    FIELD_URL,
];

/// The five fields a [`RemotePatch`] can carry, each with the capability that says whether the
/// bound provider will take it.
///
/// A table rather than five branches, on `ABILITIES`' own reasoning in the interface half: a
/// capability check written once per field is five places to forget one. The draw path there and
/// the write path here ask the same question of the same flag set, which is what keeps a greyed
/// control and a skipped write from ever disagreeing.
pub type Writable = (&'static str, fn(&ProviderCaps) -> bool);

/// See [`WRITABLE`].
pub const WRITABLE: &[Writable] = &[
    (FIELD_LANE, |caps| caps.write_lane),
    (FIELD_TITLE, |caps| caps.write_title),
    (FIELD_BODY, |caps| caps.write_body),
    (FIELD_LABELS, |caps| caps.write_labels),
    (FIELD_ASSIGNEES, |caps| caps.write_assignee),
];

/// Whether this field can ever travel outward at all, capability aside.
pub fn is_writable(field: &str) -> bool {
    WRITABLE.iter().any(|(name, _)| *name == field)
}

/// Whether the bound provider will take a write to this field.
pub fn accepts(caps: &ProviderCaps, field: &str) -> bool {
    WRITABLE
        .iter()
        .find(|(name, _)| *name == field)
        .is_some_and(|(_, needs)| needs(caps))
}

/// What one field's read hashes to on **the task's** side, in the task's own vocabulary.
///
/// The same keys as `sync::hashes` and deliberately not the same values: see the module doc. A
/// field with no local counterpart is simply absent, and an absent key compares equal to an absent
/// key, which is the right answer for a row written before this existed.
pub fn local_hashes(record: &TaskRecord) -> BTreeMap<String, String> {
    let labels: Vec<&str> = record.labels.iter().map(|l| l.name.as_str()).collect();
    BTreeMap::from([
        (FIELD_TITLE.to_string(), digest(&record.title)),
        (FIELD_BODY.to_string(), digest(&record.description)),
        (
            FIELD_LANE.to_string(),
            digest(&format!("{:?}", record.status)),
        ),
        (FIELD_LABELS.to_string(), digest(&labels.join("\u{1f}"))),
        (
            FIELD_ASSIGNEES.to_string(),
            digest(record.assigned_to.as_deref().unwrap_or_default()),
        ),
        (
            FIELD_KEY.to_string(),
            digest(record.key.as_deref().unwrap_or_default()),
        ),
        (
            FIELD_URL.to_string(),
            digest(record.link.as_deref().unwrap_or_default()),
        ),
        (
            FIELD_KIND.to_string(),
            digest(&format!("{:?}", record.kind)),
        ),
        (
            FIELD_PRIORITY.to_string(),
            digest(&format!("{:?}", record.priority)),
        ),
    ])
}

/// What the pass does with one field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Neither side moved. The commonest cell by far, and the cheapest.
    Nothing,
    /// The remote moved and the task did not.
    Pull,
    /// The task moved and the remote did not.
    Push,
    /// Both moved. The binding's switch names the winner, and the loser's value is kept in a
    /// comment on the task before anything is overwritten.
    Both(DriftSide),
}

impl Decision {
    /// Which way this settles, for a [`FieldDrift`] row and for the writer.
    pub fn side(self) -> Option<DriftSide> {
        match self {
            Decision::Nothing => None,
            Decision::Pull => Some(DriftSide::Pull),
            Decision::Push => Some(DriftSide::Push),
            Decision::Both(side) => Some(side),
        }
    }

    /// Whether this is the cell the switch exists for — which is also the cell that files the task
    /// [`LinkState::Drifted`](ubiq_proto::tasksrc::LinkState::Drifted).
    pub fn is_conflict(self) -> bool {
        matches!(self, Decision::Both(_))
    }
}

/// The table itself, and the only place it is written down in code.
///
/// The switch is consulted **only** when both sides changed. In every other cell there is nothing
/// to decide: one side changed, so that side's value is the answer.
pub fn decide(local_changed: bool, remote_changed: bool, authority: Authority) -> Decision {
    match (local_changed, remote_changed) {
        (false, false) => Decision::Nothing,
        (false, true) => Decision::Pull,
        (true, false) => Decision::Push,
        (true, true) => Decision::Both(match authority {
            Authority::UbiqWins => DriftSide::Push,
            Authority::RemoteWins => DriftSide::Pull,
        }),
    }
}

/// What a push would actually do: the patch, and every field that wanted to go and could not.
#[derive(Debug, Default)]
pub struct Push {
    pub patch: RemotePatch,
    /// Fields the bound provider will not take, by name. Never dropped in silence — each becomes a
    /// [`FieldDrift`] with no settling side.
    pub refused: Vec<String>,
}

/// Build the patch for one task, from the fields that settle as [`DriftSide::Push`].
///
/// A field the provider cannot write is left out of the patch and named in [`Push::refused`]. A
/// field that is pull-only by construction never reaches here — [`is_writable`] is asked first by
/// the caller, which is also what makes the pull-only set one list rather than a rule repeated at
/// every call site.
pub fn push_patch(
    binding: &Binding,
    caps: &ProviderCaps,
    record: &TaskRecord,
    fields: &[&str],
) -> Push {
    let mut out = Push::default();
    for field in fields {
        if !is_writable(field) {
            continue;
        }
        if !accepts(caps, field) {
            out.refused.push((*field).to_string());
            continue;
        }
        match *field {
            FIELD_TITLE => out.patch.title = Some(record.title.clone()),
            FIELD_BODY => out.patch.body = Some(record.description.clone()),
            FIELD_LABELS => {
                out.patch.labels = Some(
                    record
                        .labels
                        .iter()
                        .map(|l| l.name.clone())
                        .collect::<Vec<_>>(),
                )
            }
            FIELD_ASSIGNEES => {
                out.patch.assignees =
                    Some(record.assigned_to.clone().into_iter().collect::<Vec<_>>())
            }
            FIELD_LANE => match binding.lane_of(record.status) {
                Some(lane) => out.patch.lane = Some(lane),
                // The column this task sits in maps to no remote lane. Moving the card anywhere
                // else would put it in a lane nobody chose, which is `R9`'s parking rule read in
                // the outbound direction — so nothing is sent and the divergence is stated.
                None => out.refused.push(FIELD_LANE.to_string()),
            },
            _ => {}
        }
    }
    out
}

/// What a field's value reads as, for the drift overview's two columns.
///
/// Rendered for a human and never parsed back: this is evidence somebody chooses between, not a
/// value anything round-trips.
pub fn local_text(record: &TaskRecord, field: &str) -> String {
    match field {
        FIELD_TITLE => record.title.clone(),
        FIELD_BODY => record.description.clone(),
        FIELD_LANE => record.status.label().to_string(),
        FIELD_LABELS => record
            .labels
            .iter()
            .map(|l| l.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        FIELD_ASSIGNEES => record.assigned_to.clone().unwrap_or_default(),
        FIELD_KEY => record.key.clone().unwrap_or_default(),
        FIELD_URL => record.link.clone().unwrap_or_default(),
        FIELD_KIND => record
            .kind
            .map(|k| k.label().to_string())
            .unwrap_or_default(),
        FIELD_PRIORITY => record.priority.label().unwrap_or("normal").to_string(),
        _ => String::new(),
    }
}

/// The same, for the remote item. The lane reads as the column it maps to where the map names one,
/// because that is the word the task's own column is drawn with and two vocabularies side by side
/// would be unreadable; an unmapped lane reads as the raw id, which is the honest answer.
pub fn remote_text(binding: &Binding, item: &RemoteItem, field: &str) -> String {
    match field {
        FIELD_TITLE => item.title.clone(),
        FIELD_BODY => item.body.clone(),
        FIELD_LANE => binding
            .status_of(&item.lane)
            .map(|status| status.label().to_string())
            .unwrap_or_else(|| item.lane.to_string()),
        FIELD_LABELS => item.labels.join(", "),
        FIELD_ASSIGNEES => item.assignees.first().cloned().unwrap_or_default(),
        FIELD_KEY => item.key.clone().unwrap_or_default(),
        FIELD_URL => item.url.clone(),
        FIELD_KIND => binding
            .kind_of(item)
            .map(|k| k.label().to_string())
            .unwrap_or_default(),
        FIELD_PRIORITY => binding
            .priority_of(item)
            .and_then(|p| p.label())
            .unwrap_or("normal")
            .to_string(),
        _ => String::new(),
    }
}

/// Why a field settles the way it does, in the one sentence the overview draws beside it.
pub fn note(binding: &Binding, caps: &ProviderCaps, field: &str, decision: Decision) -> String {
    if decision.is_conflict() {
        let who = match binding.authority {
            Authority::UbiqWins => "Ubiq wins",
            Authority::RemoteWins => "the remote wins",
        };
        if binding.direction == Direction::Pull && binding.authority == Authority::UbiqWins {
            return format!(
                "Both sides changed {field}. The switch says {who}, and this binding is pull \
                 only — so the task keeps its value and the remote is never told."
            );
        }
        return format!("Both sides changed {field}. The switch says {who}.");
    }
    if decision == Decision::Push {
        if !is_writable(field) {
            return format!(
                "{field} is pull only: the remote mints it, or the binding's map derives it, and \
                 a many-to-one map has no inverse."
            );
        }
        if binding.direction == Direction::Pull {
            return format!(
                "{field} changed here. This binding is pull only, so nothing was sent (`R13`)."
            );
        }
        if !accepts(caps, field) {
            return format!("This provider does not accept a change to {field}.");
        }
    }
    format!("{field} differs.")
}

/// One drift row, assembled.
pub fn row(
    binding: &Binding,
    caps: &ProviderCaps,
    record: &TaskRecord,
    item: &RemoteItem,
    field: &str,
    decision: Decision,
) -> FieldDrift {
    // A push that cannot be made settles nowhere: it is a standing divergence, not a choice.
    let settles = match decision {
        Decision::Push | Decision::Both(DriftSide::Push)
            if binding.direction == Direction::Pull
                || !is_writable(field)
                || !accepts(caps, field) =>
        {
            None
        }
        other => other.side(),
    };
    FieldDrift {
        field: field.to_string(),
        local: local_text(record, field),
        remote: remote_text(binding, item, field),
        settles,
        note: note(binding, caps, field, decision),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_consulted_only_where_both_sides_moved() {
        for authority in [Authority::UbiqWins, Authority::RemoteWins] {
            assert_eq!(decide(false, false, authority), Decision::Nothing);
            assert_eq!(decide(false, true, authority), Decision::Pull);
            assert_eq!(decide(true, false, authority), Decision::Push);
        }
        assert_eq!(
            decide(true, true, Authority::UbiqWins),
            Decision::Both(DriftSide::Push)
        );
        assert_eq!(
            decide(true, true, Authority::RemoteWins),
            Decision::Both(DriftSide::Pull)
        );
    }

    #[test]
    fn the_writable_set_is_exactly_what_a_patch_can_carry() {
        // A field on the table that a `RemotePatch` has no slot for would be a write that is
        // planned and never made.
        assert_eq!(WRITABLE.len(), 5);
        for (field, _) in WRITABLE {
            assert!(
                FIELDS.contains(field),
                "{field} is writable but not compared"
            );
        }
        for pull_only in [FIELD_KEY, FIELD_URL, FIELD_KIND, FIELD_PRIORITY] {
            assert!(!is_writable(pull_only));
        }
    }

    #[test]
    fn a_capability_a_provider_lacks_is_refused_by_name_rather_than_dropped() {
        let binding = Binding::new(
            "test",
            ubiq_proto::ids::ConnectionId::generate(),
            ubiq_proto::tasksrc::RemoteContainerId::new("board"),
        );
        let caps = ProviderCaps {
            write_title: true,
            ..ProviderCaps::default()
        };
        let mut record = TaskRecord::new("a title".to_string(), None, chrono::Utc::now());
        record.description = "a body".to_string();
        let push = push_patch(&binding, &caps, &record, &[FIELD_TITLE, FIELD_BODY]);
        assert_eq!(push.patch.title.as_deref(), Some("a title"));
        assert_eq!(push.patch.body, None, "the body is not attempted");
        assert_eq!(push.refused, vec![FIELD_BODY.to_string()]);
    }

    #[test]
    fn a_column_the_lane_map_does_not_reach_refuses_rather_than_guessing_a_lane() {
        // `R9`'s parking rule, read outward: a card is never moved to a lane nobody chose.
        let binding = Binding::new(
            "test",
            ubiq_proto::ids::ConnectionId::generate(),
            ubiq_proto::tasksrc::RemoteContainerId::new("board"),
        );
        let caps = ProviderCaps {
            write_lane: true,
            ..ProviderCaps::default()
        };
        let record = TaskRecord::new("a title".to_string(), None, chrono::Utc::now());
        let push = push_patch(&binding, &caps, &record, &[FIELD_LANE]);
        assert!(push.patch.lane.is_none());
        assert_eq!(push.refused, vec![FIELD_LANE.to_string()]);
    }

    #[test]
    fn a_local_edit_moves_only_its_own_field_s_hash() {
        let mut record = TaskRecord::new("first".to_string(), None, chrono::Utc::now());
        let before = local_hashes(&record);
        record.title = "second".to_string();
        let after = local_hashes(&record);
        assert_ne!(before.get(FIELD_TITLE), after.get(FIELD_TITLE));
        assert_eq!(before.get(FIELD_BODY), after.get(FIELD_BODY));
        assert_eq!(before.get(FIELD_LANE), after.get(FIELD_LANE));
    }
}
