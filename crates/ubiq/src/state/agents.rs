//! The agents screen's own view of the work: which agents are on screen, how they are spread
//! across the parallel columns, and which of them the user has put back on the bench.
//!
//! **A column is a place to talk to an agent, not a place an agent lives.** The work itself —
//! sessions, agents, tasks — arrives from the host and lives in [`super::work`]; this is the
//! arrangement over it, which is why every reader takes a [`WorkProjection`] as its first
//! parameter rather than holding one. The split is what keeps both halves testable without a
//! frame, and it is the same shape [`super::orchestration`] and [`super::board`] have.
//!
//! **The arrangement is the interface's own fact.** Nothing outside this window has an opinion
//! about which column an agent's conversation is drawn in, so no message carries it and no drop
//! sends one — the same rule the graph's card positions follow. What *is* the host's is which
//! agents exist and what they are doing, and that is read and never written here.
//!
//! **Closing a tab benches an agent; it does not end it.** A terminal pane's close kills the
//! harness behind it, because a pane *is* the harness's screen. A column tab is a view onto a
//! conversation, so taking it off screen leaves the agent running and puts it on the bench, where
//! the sidebar still lists it and one click brings it back. Nothing on this screen kills an agent.
//!
//! Nothing here draws and nothing here names a colour — an activity says what it *is*, and
//! `ui::work` decides which token that reads in.

use ubiq_proto::ids::SessionId;
use ubiq_proto::work::{AgentId, Bucket, WorkAgent};

use super::work::WorkProjection;

/// The most columns the screen holds at once.
///
/// A ceiling rather than an unbounded row, for two reasons that point the same way. Each column
/// owns a composer, and a composer is a window entity created before the first frame rather than
/// during one — so the pool of them is fixed and the columns draw from it. And a column narrower
/// than [`COLUMN_MIN_WIDTH`] stops being a conversation, which is the same limit arriving from the
/// other side.
pub const COLUMNS_MAX: usize = 8;

/// The composer slot the chat panel types into.
///
/// The panel is a surface that hosts a conversation like a column does, so it needs a field from
/// the same pool — and it is not a column, so it cannot be allocated one by
/// [`AgentsView::free_slot`]. It gets the slot past the last column's, permanently.
pub const CHAT_SLOT: usize = COLUMNS_MAX;

/// How many composer fields the window builds before its first frame: one per column, plus the
/// chat panel's. Every pool indexed by a slot is this long.
pub const COMPOSER_SLOTS: usize = COLUMNS_MAX + 1;

/// The narrowest a column is drawn. Below this a transcript is a word per line, so the row scrolls
/// sideways rather than squeezing further.
pub const COLUMN_MIN_WIDTH: f32 = 360.0;

/// One column: an ordered set of agent tabs, and which of them is in front.
///
/// A column with more than one tab is **grouped** — several agents stacked in one strip, which is
/// what the header counts and what a tab dragged onto another column produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    /// Which of the window's composers this column owns. Allocated when the column opens and freed
    /// when it closes, and **stable for the column's life** — so what was typed into a column does
    /// not move to a neighbour when a column to its left is closed.
    pub slot: usize,
    pub tabs: Vec<AgentId>,
    pub active: usize,
}

impl Column {
    fn new(slot: usize, agent: AgentId) -> Self {
        Self {
            slot,
            tabs: vec![agent],
            active: 0,
        }
    }

    pub fn active_agent(&self) -> Option<AgentId> {
        self.tabs.get(self.active).copied()
    }

    /// Whether this column is holding more than one agent, which is what "grouped" means.
    pub fn grouped(&self) -> bool {
        self.tabs.len() > 1
    }
}

/// What the agents screen is showing: the columns, which one the sidebar opens into, and what is
/// typed in each.
pub struct AgentsView {
    pub columns: Vec<Column>,
    /// Which column the sidebar's and the keyboard's "here" mean. Clamped by every mutator, so it
    /// always names a live column or is zero on an empty screen.
    pub focus: usize,
    /// The sessions the sidebar has shut. Absent is open, so a session that arrives after the
    /// screen was last looked at arrives expanded rather than hidden.
    pub collapsed: Vec<SessionId>,
    /// What is typed in each composer, by slot, mirroring the window's textarea so rendering never
    /// has to read the entity. Always [`COMPOSER_SLOTS`] long; a slot nothing holds is empty.
    pub drafts: Vec<String>,
    /// The tab the pointer is carrying. A tab dropped on another column joins it; dropped past the
    /// last column it opens one of its own.
    pub dragging: Option<AgentId>,
    /// Whether the screen has laid itself out from the work yet. The first `WorkList` arranges the
    /// columns once; every later one only prunes, because an arrangement the user has changed is
    /// not something an arriving record may undo.
    pub arranged: bool,
}

impl Default for AgentsView {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            focus: 0,
            collapsed: Vec::new(),
            drafts: vec![String::new(); COMPOSER_SLOTS],
            dragging: None,
            arranged: false,
        }
    }
}

impl AgentsView {
    // ── what it holds ───────────────────────────────────────────────

    /// Which column and which tab an agent is drawn in, if it is on screen at all.
    pub fn holds(&self, agent: AgentId) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(col, column)| {
            column
                .tabs
                .iter()
                .position(|id| *id == agent)
                .map(|tab| (col, tab))
        })
    }

    pub fn on_screen(&self, agent: AgentId) -> bool {
        self.holds(agent).is_some()
    }

    /// The agent one column is showing.
    pub fn active_agent(&self, column: usize) -> Option<AgentId> {
        self.columns.get(column)?.active_agent()
    }

    /// The agents the host reports that no column is showing. **On the bench, not gone**: each is
    /// still running, still listed in the sidebar, and one click from a column of its own.
    pub fn benched<'a>(&self, work: &'a WorkProjection) -> Vec<&'a WorkAgent> {
        work.agents
            .iter()
            .filter(|agent| !self.on_screen(agent.id))
            .collect()
    }

    /// How many agents are drawn across every column. Not the number the host reports — the bench
    /// is the difference.
    pub fn on_the_field(&self) -> usize {
        self.columns.iter().map(|column| column.tabs.len()).sum()
    }

    /// How many columns hold more than one agent, which is the header's "grouped" count.
    pub fn grouped(&self) -> usize {
        self.columns
            .iter()
            .filter(|column| column.grouped())
            .count()
    }

    /// How many agents on the field are in one bucket — what the status bar counts, and the reason
    /// it counts the field rather than the whole project: the strip reports on what is on screen.
    pub fn count(&self, work: &WorkProjection, bucket: Bucket) -> usize {
        self.columns
            .iter()
            .flat_map(|column| column.tabs.iter())
            .filter_map(|id| work.agent(*id))
            .filter(|agent| agent.activity.bucket() == bucket)
            .count()
    }

    pub fn is_collapsed(&self, session: SessionId) -> bool {
        self.collapsed.contains(&session)
    }

    /// Whether another column can be opened. The screen says so rather than refusing a click with
    /// no explanation: the control is not offered when this is false.
    pub fn has_room(&self) -> bool {
        self.columns.len() < COLUMNS_MAX
    }

    pub fn draft(&self, slot: usize) -> &str {
        self.drafts.get(slot).map(String::as_str).unwrap_or("")
    }

    /// The lowest composer nothing is using. `None` is a full screen.
    fn free_slot(&self) -> Option<usize> {
        (0..COLUMNS_MAX).find(|slot| self.columns.iter().all(|column| column.slot != *slot))
    }

    // ── Mutators ────────────────────────────────────────────────────
    //
    // None of them notifies: they are called from `AppState`, which is what owns the redraw.

    /// Lay the screen out from the work, once.
    ///
    /// **One column per session that has an agent in it, holding every agent in that session, in
    /// the order the host lists them.** It is the arrangement that says the most about a project on
    /// the frame it opens: each column is a piece of work, and a session running several agents
    /// arrives grouped rather than spread across the row. The bench therefore starts empty — it
    /// fills as the user closes tabs, which is the only thing that puts anything on it.
    ///
    /// Sessions past [`COLUMNS_MAX`] have no column, so their agents start benched.
    pub fn arrange(&mut self, work: &WorkProjection) {
        self.columns.clear();
        for session in &work.sessions {
            let members: Vec<AgentId> = work
                .agents
                .iter()
                .filter(|agent| agent.session == session.id)
                .map(|agent| agent.id)
                .collect();
            if members.is_empty() {
                continue;
            }
            let Some(slot) = self.free_slot() else { break };
            self.columns.push(Column {
                slot,
                tabs: members,
                active: 0,
            });
        }
        self.focus = 0;
        self.arranged = true;
    }

    /// Drop every tab naming an agent the host no longer reports, and the columns that empties.
    ///
    /// Answers whether anything went, so a `WorkList` that changed nothing costs no redraw. A
    /// benched agent that disappears needs nothing done: the bench is computed from the work, so it
    /// simply stops being listed.
    pub fn prune(&mut self, work: &WorkProjection) -> bool {
        let before = self.on_the_field() + self.columns.len();
        for column in &mut self.columns {
            column.tabs.retain(|id| work.agent(*id).is_some());
            column.active = column.active.min(column.tabs.len().saturating_sub(1));
        }
        self.columns.retain(|column| !column.tabs.is_empty());
        self.clamp_focus();
        before != self.on_the_field() + self.columns.len()
    }

    /// Bring an agent to the front: the tab of whatever column holds it, or a column of its own.
    ///
    /// The one thing a click in the sidebar does, and it always does something. An agent on the
    /// field is revealed where it is. A benched one gets a column, and on a full row it is grouped
    /// into the focused column instead — the ceiling is on columns, and "show me this agent" is a
    /// request the screen can honour whatever the row looks like.
    ///
    /// Answers whether it is on screen afterwards. The only `false` is a screen with no columns at
    /// all and no room for one, which [`COLUMNS_MAX`] being greater than zero rules out.
    pub fn reveal(&mut self, agent: AgentId) -> bool {
        if let Some((col, tab)) = self.holds(agent) {
            self.columns[col].active = tab;
            self.focus = col;
            return true;
        }
        if self.open(agent) {
            return true;
        }
        let focus = self.focus;
        self.open_in(focus, agent)
    }

    /// Open an agent in a column of its own, at the end of the row.
    pub fn open(&mut self, agent: AgentId) -> bool {
        if self.on_screen(agent) {
            return true;
        }
        let Some(slot) = self.free_slot() else {
            return false;
        };
        self.columns.push(Column::new(slot, agent));
        self.focus = self.columns.len() - 1;
        true
    }

    /// Add an agent to one column's strip, in front. What the column's `+` does, and what a tab
    /// dragged onto another column produces.
    pub fn open_in(&mut self, column: usize, agent: AgentId) -> bool {
        // Taken off wherever it was first, so an agent is never in two columns at once — and the
        // column it came from goes if that emptied it, which can shift the target's index.
        let column = match self.holds(agent) {
            Some((from, _)) if from == column => return true,
            Some(_) => {
                let target = self.columns.get(column).map(|c| c.slot);
                self.bench(agent);
                match target.and_then(|slot| self.columns.iter().position(|c| c.slot == slot)) {
                    Some(column) => column,
                    None => return self.open(agent),
                }
            }
            None => column,
        };
        let Some(target) = self.columns.get_mut(column) else {
            return self.open(agent);
        };
        target.tabs.push(agent);
        target.active = target.tabs.len() - 1;
        self.focus = column;
        true
    }

    /// Take an agent off the field. The agent keeps running — see the module note — and the column
    /// goes with it when it held nothing else.
    ///
    /// The freed composer's draft is dropped, because a slot is handed to the next column that
    /// opens and what was typed at one agent must not turn up addressed to another.
    pub fn bench(&mut self, agent: AgentId) {
        let Some((col, tab)) = self.holds(agent) else {
            return;
        };
        let column = &mut self.columns[col];
        column.tabs.remove(tab);
        if column.tabs.is_empty() {
            let slot = column.slot;
            self.columns.remove(col);
            self.clear_draft(slot);
        } else {
            column.active = column.active.min(column.tabs.len() - 1);
        }
        self.clamp_focus();
    }

    /// Put a tab in a column of its own, at a position in the row. What dropping a tab past the
    /// last column does.
    pub fn split_off(&mut self, agent: AgentId, at: usize) -> bool {
        // A column already holding only this agent is already the answer, and moving it would be a
        // drag that reads as having done nothing but reorder the row.
        if let Some((col, _)) = self.holds(agent)
            && !self.columns[col].grouped()
        {
            return true;
        }
        // Room is checked **before** the tab is taken off its column. Everything reaching this
        // point is in a grouped column, so benching it frees no slot — and a split that ran out of
        // room after benching would leave the tab neither where it was nor where it was going.
        let Some(slot) = self.free_slot() else {
            return false;
        };
        self.bench(agent);
        let at = at.min(self.columns.len());
        self.columns.insert(at, Column::new(slot, agent));
        self.focus = at;
        true
    }

    /// Which tab of a column is in front.
    pub fn select_tab(&mut self, column: usize, tab: usize) {
        if let Some(held) = self.columns.get_mut(column)
            && tab < held.tabs.len()
        {
            held.active = tab;
            self.focus = column;
        }
    }

    pub fn focus_column(&mut self, column: usize) {
        if column < self.columns.len() {
            self.focus = column;
        }
    }

    pub fn toggle_session(&mut self, session: SessionId) {
        if let Some(ix) = self.collapsed.iter().position(|id| *id == session) {
            self.collapsed.remove(ix);
        } else {
            self.collapsed.push(session);
        }
    }

    pub fn set_draft(&mut self, slot: usize, text: String) {
        if let Some(draft) = self.drafts.get_mut(slot) {
            *draft = text;
        }
    }

    pub fn clear_draft(&mut self, slot: usize) {
        if let Some(draft) = self.drafts.get_mut(slot) {
            draft.clear();
        }
    }

    fn clamp_focus(&mut self) {
        self.focus = self.focus.min(self.columns.len().saturating_sub(1));
    }
}
