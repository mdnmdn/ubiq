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
