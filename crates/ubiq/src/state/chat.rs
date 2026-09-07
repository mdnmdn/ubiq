//! The chat surface's own state: one entry per open tab.
//!
//! **What a tab shows is not here.** A conversation is the host's, projected into
//! [`super::conversation`] and drawn by the one view every surface shares — a column on the
//! agents screen and a chat tab alike. What is here is only which conversation a tab has picked,
//! and nothing about the conversation itself.

use ubiq_proto::work::{AgentId, WorkAgent};

use super::agents::{CHATS_MAX, COLUMNS_MAX};
use super::dock::ChatId;

/// One chat tab: a view, and nothing else. It owns a composer slot and, at most, an attachment.
/// Closing it ends nothing — the conversation is the host's.
#[derive(Clone, Copy, Debug)]
pub struct ChatTab {
    pub id: ChatId,
    pub slot: usize,
    /// The conversation this tab looks at, or none: a tab may exist attached to nothing, which is
    /// what a fresh `+` produces.
    pub attached: Option<AgentId>,
    /// Whether this tab's attach picker is down. Per tab, the way a conversation's own
    /// pre-launch config picker is per conversation: several may be open at once.
    pub picker_open: bool,
}

/// The lowest chat composer slot nothing is using. `None` is every chat slot taken.
///
/// Chat tabs draw from the range above the columns' — `COLUMNS_MAX..COLUMNS_MAX + CHATS_MAX` —
/// so the two halves of the pool never hand out the same slot.
pub fn free_chat_slot(tabs: &[ChatTab]) -> Option<usize> {
    (COLUMNS_MAX..COLUMNS_MAX + CHATS_MAX).find(|slot| tabs.iter().all(|tab| tab.slot != *slot))
}

/// What one chat tab's attach picker offers: the project's conversations, filtered by what is
/// typed, and which of the survivors are already spoken for.
pub struct AttachChoices {
    /// `(agent, name)`, in the host's own order, after the filter.
    pub items: Vec<(AgentId, String)>,
    /// Indices into `items` attached to a *different* chat tab — drawn disabled, never dropped:
    /// a row that vanishes reads as a conversation that ended, not one taken.
    pub disabled: Vec<usize>,
    /// This tab's own attachment, if it survived the filter. Never in `disabled`, even if the
    /// filter kept it: a picker's current value is always its own to leave.
    pub selected: Option<usize>,
}

/// Build one tab's attach choices out of the project's chats and agents.
///
/// **Exclusivity is per surface, not per conversation.** A conversation already open in a
/// *different* chat tab is disabled here; the agents workbench may show the same conversation at
/// the same time, and the host does not care, because a view was never the workspace.
pub fn attach_choices(
    chats: &[ChatTab],
    this: ChatId,
    agents: &[WorkAgent],
    query: &str,
) -> AttachChoices {
    let query = query.to_lowercase();
    let items: Vec<(AgentId, String)> = agents
        .iter()
        .filter(|agent| query.is_empty() || agent.name.to_lowercase().contains(&query))
        .map(|agent| (agent.id, agent.name.clone()))
        .collect();

    let elsewhere: Vec<AgentId> = chats
        .iter()
        .filter(|tab| tab.id != this)
        .filter_map(|tab| tab.attached)
        .collect();
    let disabled = items
        .iter()
        .enumerate()
        .filter_map(|(ix, (agent, _))| elsewhere.contains(agent).then_some(ix))
        .collect();

    let attached = chats
        .iter()
        .find(|tab| tab.id == this)
        .and_then(|tab| tab.attached);
    let selected = attached.and_then(|agent| items.iter().position(|(id, _)| *id == agent));

    AttachChoices {
        items,
        disabled,
        selected,
    }
}

/// One thing the unified control can *start*, already labelled by the surface that knows the
/// harness list. `choice` is the row's index in [`crate::state::WorkbenchState::harness_choices`],
/// which is what a pick resolves through; a heading or a hairline carries none.
pub struct StartOffer {
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub choice: Option<usize>,
}

/// What one row of the unified control does when it is clicked.
///
/// **Rows and the actions behind them are matched by position**, the rule every menu in this
/// window follows, so the list drawn and the list a pick resolves against are one list built
/// once — never two that could drift apart under a reorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChatPick {
    /// Start a conversation from that row of `harness_choices`, and attach this tab to it.
    Start(usize),
    /// Show a conversation that is already running.
    Attach(AgentId),
    /// A heading or a hairline: drawn, never picked.
    Inert,
}

/// The unified control's rows: everything this tab can do about what it is showing.
pub struct ChatPicks {
    pub rows: Vec<ChatPick>,
    pub labels: Vec<String>,
    /// Headings, hairlines, harnesses this machine lacks, and conversations another tab holds.
    pub disabled: Vec<usize>,
    pub separators: Vec<usize>,
    pub selected: Option<usize>,
}

/// Build them.
///
/// **An attached tab is offered switches only.** Starting a second conversation from a tab that
/// already shows one would leave the first with no view and no way back to it, so the offer to
/// start is the empty tab's alone — which is what makes one control answer both questions without
/// either becoming a trap.
/// `query` is what was typed. [`attach_choices`] has already applied it to the conversations, so
/// this applies it to the other half — a harness list can run to hundreds of rows once every
/// account and saved setup is on it, and half a searchable list is not a searchable list.
///
/// **A filtered list loses its groups.** While something is typed the headings and hairlines are
/// dropped rather than left standing over rows that may all have gone: what a search shows is the
/// matches, and a heading with nothing under it reads as a group that failed to load.
pub fn chat_picks(
    offers: &[StartOffer],
    attach: &AttachChoices,
    attached: bool,
    query: &str,
) -> ChatPicks {
    let query = query.trim().to_lowercase();
    let filtering = !query.is_empty();
    let mut picks = ChatPicks {
        rows: Vec::new(),
        labels: Vec::new(),
        disabled: Vec::new(),
        separators: Vec::new(),
        selected: None,
    };
    fn push(picks: &mut ChatPicks, row: ChatPick, label: String, enabled: bool) {
        if !enabled {
            picks.disabled.push(picks.rows.len());
        }
        picks.rows.push(row);
        picks.labels.push(label);
    }

    if !attached {
        if !filtering {
            push(&mut picks, ChatPick::Inert, "Start new".to_string(), false);
        }
        for offer in offers {
            if filtering && (offer.choice.is_none() || !offer.label.to_lowercase().contains(&query))
            {
                continue;
            }
            if offer.separator {
                picks.separators.push(picks.rows.len());
            }
            let row = match offer.choice {
                Some(choice) if !offer.separator => ChatPick::Start(choice),
                _ => ChatPick::Inert,
            };
            let enabled = offer.enabled && offer.choice.is_some() && !offer.separator;
            push(&mut picks, row, offer.label.clone(), enabled);
        }
        // The second half is only worth a heading when there is something under it: a lone
        // heading over nothing reads as a list that failed to load.
        if !attach.items.is_empty() && !filtering {
            picks.separators.push(picks.rows.len());
            push(&mut picks, ChatPick::Inert, String::new(), false);
            push(
                &mut picks,
                ChatPick::Inert,
                "Attach running".to_string(),
                false,
            );
        }
    }

    for (ix, (agent, name)) in attach.items.iter().enumerate() {
        if attach.selected == Some(ix) {
            picks.selected = Some(picks.rows.len());
        }
        let enabled = !attach.disabled.contains(&ix);
        push(&mut picks, ChatPick::Attach(*agent), name.clone(), enabled);
    }

    picks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(label: &str, choice: Option<usize>) -> StartOffer {
        StartOffer {
            label: label.to_string(),
            enabled: true,
            separator: false,
            choice,
        }
    }

    fn attach(
        items: Vec<(AgentId, String)>,
        disabled: Vec<usize>,
        selected: Option<usize>,
    ) -> AttachChoices {
        AttachChoices {
            items,
            disabled,
            selected,
        }
    }

    /// An empty tab is offered both halves, and every row resolves to the action drawn beside it.
    #[test]
    fn an_empty_tab_is_offered_starting_and_attaching() {
        let running = AgentId::generate();
        let picks = chat_picks(
            &[offer("Claude Code", Some(0)), offer("Codex", Some(1))],
            &attach(vec![(running, "wandering-ibis".to_string())], vec![], None),
            false,
            "",
        );

        assert_eq!(
            picks.rows,
            vec![
                ChatPick::Inert, // "Start new"
                ChatPick::Start(0),
                ChatPick::Start(1),
                ChatPick::Inert, // hairline
                ChatPick::Inert, // "Attach running"
                ChatPick::Attach(running),
            ]
        );
        assert_eq!(picks.labels.len(), picks.rows.len());
        // Only the two harnesses and the one conversation can be clicked.
        assert_eq!(picks.disabled, vec![0, 3, 4]);
    }

    /// An attached tab is offered switches only — never a start, which would orphan what it shows.
    #[test]
    fn an_attached_tab_is_offered_switches_only() {
        let mine = AgentId::generate();
        let theirs = AgentId::generate();
        let picks = chat_picks(
            &[offer("Claude Code", Some(0))],
            &attach(
                vec![(mine, "mine".to_string()), (theirs, "theirs".to_string())],
                vec![1],
                Some(0),
            ),
            true,
            "",
        );

        assert_eq!(
            picks.rows,
            vec![ChatPick::Attach(mine), ChatPick::Attach(theirs)]
        );
        assert!(
            !picks
                .rows
                .iter()
                .any(|row| matches!(row, ChatPick::Start(_)))
        );
        // The one another tab holds is drawn, not dropped; this tab's own is the selected row.
        assert_eq!(picks.disabled, vec![1]);
        assert_eq!(picks.selected, Some(0));
    }

    /// With nothing running the "Attach running" heading is not drawn over an empty list.
    #[test]
    fn no_conversations_means_no_attach_heading() {
        let picks = chat_picks(
            &[offer("Codex", Some(0))],
            &attach(vec![], vec![], None),
            false,
            "",
        );

        assert_eq!(picks.rows, vec![ChatPick::Inert, ChatPick::Start(0)]);
        assert_eq!(picks.labels[0], "Start new");
    }

    /// Typing filters **both** halves — a harness list runs to hundreds of rows once accounts and
    /// saved setups are on it, and a search that reached only the conversations would leave the
    /// long half untouched. The groups go with it: what a search shows is the matches.
    #[test]
    fn typing_filters_the_harness_half_too() {
        let running = AgentId::generate();
        let offers = vec![
            StartOffer {
                label: "Default".into(),
                enabled: false,
                separator: false,
                choice: None,
            },
            offer("Claude Code", Some(1)),
            offer("Codex", Some(2)),
        ];
        // `attach_choices` has already applied the query to its half, so the conversation that
        // survives is handed in filtered — this only has the other half left to do.
        let picks = chat_picks(
            &offers,
            &attach(vec![(running, "codex-run".to_string())], vec![], None),
            false,
            "codex",
        );

        assert_eq!(
            picks.rows,
            vec![ChatPick::Start(2), ChatPick::Attach(running)]
        );
        assert_eq!(picks.labels, vec!["Codex", "codex-run"]);
        // No headings and no hairlines while a query is typed.
        assert!(picks.separators.is_empty());
        assert!(picks.disabled.is_empty());
    }

    /// A heading inside the harness list stays inert, and the index of every real row still
    /// resolves to the `harness_choices` row it was built from.
    #[test]
    fn headings_inside_the_harness_list_stay_inert() {
        let offers = vec![
            StartOffer {
                label: "Default".into(),
                enabled: false,
                separator: false,
                choice: None,
            },
            offer("Claude Code", Some(1)),
            StartOffer {
                label: String::new(),
                enabled: false,
                separator: true,
                choice: None,
            },
            offer("Claude Code \u{2014} mdn", Some(3)),
        ];
        let picks = chat_picks(&offers, &attach(vec![], vec![], None), false, "");

        assert_eq!(
            picks.rows,
            vec![
                ChatPick::Inert, // "Start new"
                ChatPick::Inert, // "Default"
                ChatPick::Start(1),
                ChatPick::Inert, // hairline
                ChatPick::Start(3),
            ]
        );
        assert_eq!(picks.separators, vec![3]);
    }
}
