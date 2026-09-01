//! Where the graph puts things — kept apart from what they are.
//!
//! An agent's definition says what it is, whose work it serves and who spawned it. Nothing in it
//! says where it sits: position lives here, and it is **relative**. A task owns an origin on the
//! canvas and an agent owns an offset inside the task it serves. That one indirection buys two
//! things. Dragging a task moves its origin and every card in it follows, with nothing to keep in
//! step. And the whole arrangement can be thrown away and recomputed by [`Layout::auto`] without
//! touching a single agent, because no agent ever knew where it was.
//!
//! An agent with no task keeps an absolute position — there is no origin to hang it off — which is
//! the one case where the offset is read as a point on the canvas.
//!
//! Coordinates are points at 100% zoom. The zoom control scales them at draw time and a drag
//! writes back the same numbers whatever the zoom was.

use std::collections::HashMap;

use super::agents::{Agent, AgentId, SessionId, Task, TaskId};

/// The card's size at 100% zoom. The graph's arithmetic — containers, connectors, hit testing —
/// all works from these, so a card that changes size changes them in one place.
pub const CARD_WIDTH: f32 = 264.0;
pub const CARD_HEIGHT: f32 = 116.0;

/// What a task's container leaves round its cards, and the room its label takes above them.
pub const GROUP_PAD: f32 = 22.0;
pub const GROUP_LABEL: f32 = 26.0;

/// What the automatic arrangement leaves between cards, between containers, and round the lot.
pub const CARD_GAP_X: f32 = 32.0;
pub const CARD_GAP_Y: f32 = 44.0;
pub const TASK_GAP: f32 = 56.0;
pub const LAYOUT_MARGIN: f32 = 24.0;

/// How wide a row of containers may get before the next one wraps onto a new row.
pub const LAYOUT_WIDTH: f32 = 1_320.0;

/// Every position the graph draws from.
#[derive(Default, Debug)]
pub struct Layout {
    /// Where a task's first card sits — the top-left of the container's contents, not of its box.
    /// The box is derived from the cards, so this is the frame they hang off rather than an
    /// outline anybody drew.
    tasks: HashMap<TaskId, (f32, f32)>,
    /// An agent's offset inside its task, or its absolute position when it has no task.
    agents: HashMap<AgentId, (f32, f32)>,
}

impl Layout {
    /// Arrange every session from scratch, reading only the definitions.
    ///
    /// Each session is laid out from the same top-left corner, because only one is on screen at a
    /// time and a session that starts where the last one ended would open scrolled away from its
    /// own work.
    pub fn auto(agents: &[Agent], tasks: &[Task]) -> Self {
        let mut layout = Self::default();
        let mut sessions: Vec<SessionId> = Vec::new();
        for session in agents
            .iter()
            .map(|a| a.session)
            .chain(tasks.iter().filter_map(|t| t.session))
        {
            if !sessions.contains(&session) {
                sessions.push(session);
            }
        }
        for session in sessions {
            layout.arrange(session, agents, tasks);
        }
        layout
    }

    pub fn task_origin(&self, task: TaskId) -> (f32, f32) {
        self.tasks.get(&task).copied().unwrap_or((
            LAYOUT_MARGIN + GROUP_PAD,
            LAYOUT_MARGIN + GROUP_PAD + GROUP_LABEL,
        ))
    }

    /// An agent's offset inside its task, or its position when it has none.
    pub fn offset(&self, agent: AgentId) -> (f32, f32) {
        self.agents.get(&agent).copied().unwrap_or_default()
    }

    /// Where a card is drawn, on the canvas.
    pub fn at(&self, agent: &Agent) -> (f32, f32) {
        let origin = agent
            .task
            .map(|task| self.task_origin(task))
            .unwrap_or((0.0, 0.0));
        let offset = self.offset(agent.id);
        (origin.0 + offset.0, origin.1 + offset.1)
    }

    pub fn place_task(&mut self, task: TaskId, origin: (f32, f32)) {
        self.tasks.insert(task, origin);
    }

    pub fn place_agent(&mut self, agent: AgentId, offset: (f32, f32)) {
        self.agents.insert(agent, offset);
    }

    /// One session: the agents nobody gave work to along the top, then the containers flowing
    /// left to right underneath.
    fn arrange(&mut self, session: SessionId, agents: &[Agent], tasks: &[Task]) {
        let mut y = LAYOUT_MARGIN;

        // An agent with no task is usually the one handing work out, so it goes above the
        // containers rather than below them: a connector that runs down into a box reads better
        // than one that climbs out of the bottom of the graph.
        let loose: Vec<&Agent> = agents
            .iter()
            .filter(|a| a.session == session && a.task.is_none())
            .collect();
        if !loose.is_empty() {
            for (ix, agent) in loose.iter().enumerate() {
                let x = LAYOUT_MARGIN + ix as f32 * (CARD_WIDTH + CARD_GAP_X);
                self.agents.insert(agent.id, (x, y));
            }
            y += CARD_HEIGHT + TASK_GAP;
        }

        let mut x = LAYOUT_MARGIN;
        let mut row_height = 0.0f32;
        for task in tasks.iter().filter(|t| t.session == Some(session)) {
            let Contents {
                cards,
                width,
                height,
            } = inside(task.id, agents);

            // A container with nothing in it is not drawn, so it takes no room in the flow — but
            // it still gets an origin, because a card dropped into it needs a frame to hang off.
            if cards.is_empty() {
                self.tasks
                    .insert(task.id, (x + GROUP_PAD, y + GROUP_PAD + GROUP_LABEL));
                continue;
            }

            let box_w = width + GROUP_PAD * 2.0;
            let box_h = height + GROUP_PAD * 2.0 + GROUP_LABEL;
            if x > LAYOUT_MARGIN && x + box_w > LAYOUT_WIDTH {
                x = LAYOUT_MARGIN;
                y += row_height + TASK_GAP;
                row_height = 0.0;
            }

            self.tasks
                .insert(task.id, (x + GROUP_PAD, y + GROUP_PAD + GROUP_LABEL));
            for (agent, offset) in cards {
                self.agents.insert(agent, offset);
            }

            x += box_w + TASK_GAP;
            row_height = row_height.max(box_h);
        }
    }
}

/// One container's contents: an offset per card, and the size those cards take up.
struct Contents {
    cards: Vec<(AgentId, (f32, f32))>,
    width: f32,
    height: f32,
}

/// Arrange one container.
///
/// Cards are stacked by how far they are from whoever started the work — roots on the top row,
/// their children on the next — which draws the three task shapes without knowing about any of
/// them. One agent is one card. A chain is a column, because each link answers to the last. A
/// coordinated task is a coordinator over a row of workers.
fn inside(task: TaskId, agents: &[Agent]) -> Contents {
    let members: Vec<&Agent> = agents.iter().filter(|a| a.task == Some(task)).collect();
    if members.is_empty() {
        return Contents {
            cards: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
    }

    // Only a parent inside the same container counts. An agent answering to one outside it is a
    // root here, and the connector to its parent is drawn across the boundary.
    let mut parents: HashMap<AgentId, AgentId> = HashMap::new();
    for agent in &members {
        if let Some(parent) = agent.parent
            && members.iter().any(|m| m.id == parent)
        {
            parents.insert(agent.id, parent);
        }
    }

    let mut rows: Vec<Vec<AgentId>> = Vec::new();
    for agent in &members {
        let depth = depth_of(agent.id, &parents);
        if rows.len() <= depth {
            rows.resize(depth + 1, Vec::new());
        }
        rows[depth].push(agent.id);
    }

    let width = rows
        .iter()
        .map(|row| row_width(row.len()))
        .fold(0.0f32, f32::max);

    let mut cards = Vec::new();
    for (depth, row) in rows.iter().enumerate() {
        // Short rows are centred over long ones, so a coordinator sits above the middle of its
        // workers rather than over the leftmost one.
        let start = (width - row_width(row.len())) / 2.0;
        for (ix, agent) in row.iter().enumerate() {
            cards.push((
                *agent,
                (
                    start + ix as f32 * (CARD_WIDTH + CARD_GAP_X),
                    depth as f32 * (CARD_HEIGHT + CARD_GAP_Y),
                ),
            ));
        }
    }

    let height = rows.len() as f32 * CARD_HEIGHT + rows.len().saturating_sub(1) as f32 * CARD_GAP_Y;
    Contents {
        cards,
        width,
        height,
    }
}

fn row_width(cards: usize) -> f32 {
    cards as f32 * CARD_WIDTH + cards.saturating_sub(1) as f32 * CARD_GAP_X
}

/// How many hand-offs deep an agent is inside its own container. The walk is bounded by the number
/// of edges, so a parent chain that loops stops rather than spinning.
fn depth_of(agent: AgentId, parents: &HashMap<AgentId, AgentId>) -> usize {
    let mut depth = 0;
    let mut at = agent;
    while let Some(&parent) = parents.get(&at) {
        depth += 1;
        at = parent;
        if depth > parents.len() {
            break;
        }
    }
    depth
}
