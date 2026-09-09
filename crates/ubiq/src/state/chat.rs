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

/// What an attach list offers: the project's conversations, filtered by what is typed, and which
/// of the survivors are already spoken for.
pub struct AttachChoices {
    /// `(agent, name)`, in the host's own order, after the filter.
    pub items: Vec<(AgentId, String)>,
    /// Indices into `items` an open panel in the *same* surface already shows — drawn disabled,
    /// never dropped: a row that vanishes reads as a conversation that ended, not one taken.
    pub disabled: Vec<usize>,
    /// The asking panel's own attachment, if it survived the filter. Never in `disabled`, even if
    /// the filter kept it: a picker's current value is always its own to leave.
    pub selected: Option<usize>,
}

/// Build one panel's attach choices out of the project's agents.
///
/// **Exclusivity is per surface, not per conversation.** `shown` is what the *other* panels of the
/// asking surface are looking at — the other chat tabs for the IDE, the other columns for the
/// agents screen — and those rows are disabled. The two surfaces may show the same conversation at
/// the same time, and the host does not care, because a view was never the workspace.
///
/// One builder for every surface that offers to attach: the `+` menus, the chat header's chevron
/// and the agents screen alike, so what "already taken" means is answered once.
pub fn attach_choices(
    agents: &[WorkAgent],
    shown: &[AgentId],
    mine: Option<AgentId>,
    query: &str,
) -> AttachChoices {
    let query = query.trim().to_lowercase();
    let items: Vec<(AgentId, String)> = agents
        .iter()
        .filter(|agent| query.is_empty() || agent.name.to_lowercase().contains(&query))
        .map(|agent| (agent.id, agent.name.clone()))
        .collect();

    let disabled = items
        .iter()
        .enumerate()
        .filter_map(|(ix, (agent, _))| {
            (shown.contains(agent) && mine != Some(*agent)).then_some(ix)
        })
        .collect();
    let selected = mine.and_then(|agent| items.iter().position(|(id, _)| *id == agent));

    AttachChoices {
        items,
        disabled,
        selected,
    }
}

/// What one row of the chat header's control does when it is clicked.
///
/// **Rows and the actions behind them are matched by position**, the rule every menu in this
/// window follows, so the list drawn and the list a pick resolves against are one list built
/// once — never two that could drift apart under a reorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChatPick {
    /// Raise the New agent modal, and attach this tab to whatever it starts.
    New,
    /// Show a conversation that is already running.
    Attach(AgentId),
    /// A heading or a hairline: drawn, never picked.
    Inert,
}

/// The control's rows: everything this tab can do about what it is showing.
pub struct ChatPicks {
    pub rows: Vec<ChatPick>,
    pub labels: Vec<String>,
    /// Hairlines, and conversations another tab holds.
    pub disabled: Vec<usize>,
    pub separators: Vec<usize>,
    pub selected: Option<usize>,
}

/// Build them: *New agent* first, then every conversation there is to move to.
///
/// **An attached tab is offered the start too.** It used to be denied one, because starting from a
/// tab that already showed a conversation would leave the first with no view; that stopped being
/// true once the conversation it left is still on this very list, one click from coming back.
///
/// `attach` has already been filtered by what was typed, so there is nothing left to narrow here —
/// and the hairline is dropped with the list it separated when a query empties it, because a
/// divider over nothing reads as a group that failed to load.
pub fn chat_picks(attach: &AttachChoices) -> ChatPicks {
    let mut picks = ChatPicks {
        rows: vec![ChatPick::New],
        labels: vec!["New agent".to_string()],
        disabled: Vec::new(),
        separators: Vec::new(),
        selected: None,
    };
    if attach.items.is_empty() {
        return picks;
    }
    picks.separators.push(picks.rows.len());
    picks.rows.push(ChatPick::Inert);
    picks.labels.push(String::new());

    for (ix, (agent, name)) in attach.items.iter().enumerate() {
        if attach.selected == Some(ix) {
            picks.selected = Some(picks.rows.len());
        }
        if attach.disabled.contains(&ix) {
            picks.disabled.push(picks.rows.len());
        }
        picks.rows.push(ChatPick::Attach(*agent));
        picks.labels.push(name.clone());
    }

    picks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every list starts with the one thing a tab can do without an agent to move to.
    #[test]
    fn the_first_row_is_always_a_new_agent() {
        let picks = chat_picks(&attach_choices(&[], &[], None, ""));

        assert_eq!(picks.rows, vec![ChatPick::New]);
        assert_eq!(picks.labels, vec!["New agent"]);
        assert!(picks.separators.is_empty());
    }

    /// A conversation an open panel in the same surface already shows is drawn, not dropped — and
    /// the asking panel's own attachment is the selected row rather than a disabled one.
    #[test]
    fn a_conversation_shown_elsewhere_is_disabled_rather_than_dropped() {
        let mine = AgentId::generate();
        let theirs = AgentId::generate();
        let agents = vec![agent(mine, "mine"), agent(theirs, "theirs")];
        let attach = attach_choices(&agents, &[mine, theirs], Some(mine), "");
        let picks = chat_picks(&attach);

        assert_eq!(
            picks.rows,
            vec![
                ChatPick::New,
                ChatPick::Inert, // hairline
                ChatPick::Attach(mine),
                ChatPick::Attach(theirs),
            ]
        );
        assert_eq!(picks.separators, vec![1]);
        assert_eq!(picks.selected, Some(2));
        assert_eq!(picks.disabled, vec![3]);
    }

    /// Typing narrows the conversations; what it leaves empty takes its hairline with it.
    #[test]
    fn typing_narrows_the_list_and_the_hairline_goes_with_it() {
        let one = AgentId::generate();
        let agents = vec![agent(one, "codex-run"), agent(AgentId::generate(), "other")];

        let picks = chat_picks(&attach_choices(&agents, &[], None, "codex"));
        assert_eq!(
            picks.rows,
            vec![ChatPick::New, ChatPick::Inert, ChatPick::Attach(one)]
        );

        let picks = chat_picks(&attach_choices(&agents, &[], None, "nothing"));
        assert_eq!(picks.rows, vec![ChatPick::New]);
    }

    fn agent(id: AgentId, name: &str) -> WorkAgent {
        WorkAgent {
            id,
            session: ubiq_proto::ids::SessionId::generate(),
            task: None,
            parent: None,
            name: name.to_string(),
            summary: None,
            role: String::new(),
            activity: ubiq_proto::work::Activity::Thinking,
            note: String::new(),
            branch: String::new(),
            tokens: 0.0,
            harness: String::new(),
            account: String::new(),
            model: String::new(),
            context_pct: 0,
            thread: Vec::new(),
        }
    }
}
