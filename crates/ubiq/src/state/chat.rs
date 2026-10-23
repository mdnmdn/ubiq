//! The chat surface's own state: one entry per open tab.
//!
//! **What a tab shows is not here.** A conversation is the host's, projected into
//! [`super::conversation`] and drawn by the one view every surface shares — a column on the
//! agents screen and a chat tab alike. What is here is only which conversation a tab has picked,
//! and nothing about the conversation itself.

use ubiq_proto::work::{AgentId, WorkAgent};

use super::agents::{CHATS_MAX, COLUMNS_MAX};
use super::dock::ChatId;
use super::work::AgentLabel;

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

/// What one chat panel is drawn for: a chat tab in the dock, or a column on the agents screen.
///
/// The panel is the same in both (`ui::chat::render`); what differs is where its attachment and
/// composer slot are read from. A column is keyed by its composer **slot**, not its index — the
/// slot stays with the column for its life, while the index shifts as columns open and close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatHost {
    Tab(ChatId),
    Column(usize),
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
    /// `(agent, title, identity)`, in the host's own order, after the filter. The identity is
    /// empty where the agent has none; a row draws it faint after the title.
    pub items: Vec<(AgentId, String, String)>,
    /// Indices into `items` an open panel in the *same* surface already shows — drawn disabled,
    /// never dropped: a row that vanishes reads as a conversation that ended, not one taken.
    pub disabled: Vec<usize>,
    /// The asking panel's own attachment, if it survived the filter. Never in `disabled`, even if
    /// the filter kept it: a picker's current value is always its own to leave.
    pub selected: Option<usize>,
}

/// Build one panel's attach choices out of the project's agents.
///
/// **Only a conversation this window actually holds may be offered.** `live` is
/// [`AgentsView::live`](super::agents::AgentsView::live) — `agents` is wider, because it is the
/// host's projection concatenated with the fixtures the mock work thread seeds into every project,
/// which have a name and an activity and nothing behind them. A row that named one would be a
/// conversation with no harness at the other end, the same reason the agents screen already
/// refuses to draw a column on one.
///
/// `shown` rows are drawn disabled rather than dropped. The window's own attach lists (the `+`
/// menu's second stage and the chat header's chevron) pass [`hidden_agents`] as `live` and nothing
/// as `shown`, so they offer exactly what the titlebar chevron does; `mine` is still selected.
///
/// The label is [`AgentLabel::of`] with no task title; [`attach_choices_with`] takes the window's
/// own resolver (`AppState::agent_label`) so a row reads as the titlebar chevron's does.
pub fn attach_choices(
    agents: &[WorkAgent],
    live: &[AgentId],
    shown: &[AgentId],
    mine: Option<AgentId>,
    query: &str,
) -> AttachChoices {
    attach_choices_with(agents, live, shown, mine, query, |agent| {
        AgentLabel::of(agent, None)
    })
}

/// [`attach_choices`], labelling each row with `label`.
pub fn attach_choices_with(
    agents: &[WorkAgent],
    live: &[AgentId],
    shown: &[AgentId],
    mine: Option<AgentId>,
    query: &str,
    label: impl Fn(&WorkAgent) -> AgentLabel,
) -> AttachChoices {
    let query = query.trim().to_lowercase();
    // Title then identity, and matched on title, identity and handle alike (T-283's B7): a search
    // for the definition or the handle finds the agent as surely as one for its title.
    let items: Vec<(AgentId, String, String)> = agents
        .iter()
        .filter(|agent| live.contains(&agent.id))
        .map(|agent| (agent.id, label(agent)))
        .filter(|(_, label)| query.is_empty() || label.matches(&query))
        .map(|(id, label)| (id, label.title.to_string(), label.identity.to_string()))
        .collect();

    let disabled = items
        .iter()
        .enumerate()
        .filter_map(|(ix, (agent, _, _))| {
            (shown.contains(agent) && mine != Some(*agent)).then_some(ix)
        })
        .collect();
    let selected = mine.and_then(|agent| items.iter().position(|(id, _, _)| *id == agent));

    AttachChoices {
        items,
        disabled,
        selected,
    }
}

/// The live agents one rail mode does not show, in the host's own order — the whole of the
/// hidden-agents rule, behind both the titlebar chevron and *Attach existing agent*.
///
/// **Shown is per mode.** Every rail mode draws its own 0..N agent panels out of the one pool of
/// running agents — a chat panel in its dock, or the columns in Agents mode — so `shown` is what
/// *the current* mode draws and nothing else: an agent open in another mode's dock is hidden here.
/// `AppState::shown_agents` is what reads it off the window.
pub fn hidden_agents(
    agents: &[WorkAgent],
    live: &[AgentId],
    shown: &std::collections::HashSet<AgentId>,
) -> Vec<AgentId> {
    agents
        .iter()
        .map(|agent| agent.id)
        .filter(|id| live.contains(id) && !shown.contains(id))
        .collect()
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
    /// Faint secondary text per row, parallel to `labels`; empty for none.
    pub details: Vec<String>,
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
        details: vec![String::new()],
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
    picks.details.push(String::new());

    for (ix, (agent, name, identity)) in attach.items.iter().enumerate() {
        if attach.selected == Some(ix) {
            picks.selected = Some(picks.rows.len());
        }
        if attach.disabled.contains(&ix) {
            picks.disabled.push(picks.rows.len());
        }
        picks.rows.push(ChatPick::Attach(*agent));
        picks.labels.push(name.clone());
        picks.details.push(identity.clone());
    }

    picks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every list starts with the one thing a tab can do without an agent to move to.
    #[test]
    fn the_first_row_is_always_a_new_agent() {
        let picks = chat_picks(&attach_choices(&[], &[], &[], None, ""));

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
        let live = [mine, theirs];
        let attach = attach_choices(&agents, &live, &[mine, theirs], Some(mine), "");
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
        let other = AgentId::generate();
        let agents = vec![agent(one, "codex-run"), agent(other, "other")];
        let live = [one, other];

        let picks = chat_picks(&attach_choices(&agents, &live, &[], None, "codex"));
        assert_eq!(
            picks.rows,
            vec![ChatPick::New, ChatPick::Inert, ChatPick::Attach(one)]
        );

        let picks = chat_picks(&attach_choices(&agents, &live, &[], None, "nothing"));
        assert_eq!(picks.rows, vec![ChatPick::New]);
    }

    /// An agent the host projects but this window holds no conversation for — a mock fixture, or
    /// any other entry the projection widens beyond the live set — is never a row, whatever was
    /// typed: it is a name with no harness behind it, so it cannot be attached to.
    #[test]
    fn an_agent_outside_the_live_set_is_not_a_row() {
        let live_agent = AgentId::generate();
        let fixture = AgentId::generate();
        let agents = vec![agent(live_agent, "real"), agent(fixture, "Orchestrator")];

        let attach = attach_choices(&agents, &[live_agent], &[], None, "");
        assert_eq!(
            attach.items,
            vec![(live_agent, "real".to_string(), String::new())]
        );

        let picks = chat_picks(&attach);
        assert_eq!(
            picks.rows,
            vec![ChatPick::New, ChatPick::Inert, ChatPick::Attach(live_agent)]
        );
    }

    /// A search finds an agent by its definition and by its handle, not only by its title — and
    /// the row reads title first, identity after.
    #[test]
    fn a_search_matches_the_title_the_identity_and_the_handle() {
        let id = AgentId::generate();
        let mut named = agent(id, "claude 2");
        named.title = Some("Fix tab truncation".to_string());
        named.definition = Some("Reviewer".to_string());
        named.harness = "Claude Code".to_string();
        let agents = vec![named];

        for query in ["fix tab", "reviewer", "claude 2", "claude code"] {
            let attach = attach_choices(&agents, &[id], &[], None, query);
            assert_eq!(attach.items.len(), 1, "{query:?} found nothing");
        }
        let attach = attach_choices(&agents, &[id], &[], None, "");
        assert_eq!(attach.items[0].1, "Fix tab truncation");
        assert_eq!(attach.items[0].2, "Reviewer \u{00b7} Claude Code");
    }

    /// A1, A2, A3 run in the project. IDE shows A2 in its dock, Tasks shows A3: each mode hides
    /// the rest, and what another mode shows is not shown here. A non-live record is never a row.
    #[test]
    fn hidden_is_per_mode() {
        let [a1, a2, a3, ghost] = std::array::from_fn(|_| AgentId::generate());
        let agents = vec![
            agent(a1, "A1"),
            agent(a2, "A2"),
            agent(a3, "A3"),
            agent(ghost, "ghost"),
        ];
        let live = [a1, a2, a3];

        let ide = std::collections::HashSet::from([a2]);
        assert_eq!(hidden_agents(&agents, &live, &ide), vec![a1, a3]);

        let tasks = std::collections::HashSet::from([a3]);
        assert_eq!(hidden_agents(&agents, &live, &tasks), vec![a1, a2]);
    }

    fn agent(id: AgentId, name: &str) -> WorkAgent {
        WorkAgent {
            id,
            session: ubiq_proto::ids::SessionId::generate(),
            task: None,
            parent: None,
            name: name.to_string(),
            summary: None,
            title: None,
            definition: None,
            activity: ubiq_proto::work::Activity::Thinking,
            branch: String::new(),
            tokens: 0.0,
            harness: String::new(),
            account: String::new(),
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            restorable: false,
            debug_dump: None,
            run_dir: None,
            config_dir: None,
            thread: Vec::new(),
        }
    }
}
