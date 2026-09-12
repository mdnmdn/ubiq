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
//! **There is more than one arrangement, and the relative position is true of all of them.** An
//! [`Algo`] says which one a tidy computes — the calm flow, a compact pack, a spawn-order tree, or
//! narrow columns — and every one of them writes the same two maps: an origin per task, an offset
//! per card. Nothing downstream knows which was chosen, and swapping between them moves cards
//! without changing a record.
//!
//! Coordinates are points at 100% zoom. The zoom control scales them at draw time and a drag
//! writes back the same numbers whatever the zoom was.

use std::collections::HashMap;

use ubiq_proto::ids::{SessionId, TaskId};
use ubiq_proto::work::{AgentId, TaskRecord, WorkAgent};

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

/// What a packer answers: a position per box, in the order it was given them, and the extent the
/// lot takes. Named because five functions return it and the tuple says nothing on its own.
type Packing = (Vec<(f32, f32)>, (f32, f32));

/// The slack a float comparison is allowed. Geometry is accumulated by addition, so two edges that
/// should meet rarely meet exactly, and a packer that believes the difference leaves seams.
const EPS: f32 = 0.01;

/// Which arrangement a tidy computes.
///
/// The same graph reads differently depending on what the user is looking for, and no single
/// arrangement wins: a canvas that fits on screen and a canvas that shows who spawned whom are
/// different pictures of the same records. The choice is the user's, and it is remembered nowhere
/// here — it arrives as an argument to every tidy.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Algo {
    #[default]
    Flow,
    Packed,
    Tree,
    Columns,
}

impl Algo {
    pub const ALL: [Algo; 4] = [Algo::Flow, Algo::Packed, Algo::Tree, Algo::Columns];

    /// The name the menu row carries.
    pub fn label(self) -> &'static str {
        match self {
            Algo::Flow => "Flow",
            Algo::Packed => "Packed",
            Algo::Tree => "Tree",
            Algo::Columns => "Columns",
        }
    }

    /// One line saying what it does, for the row's second line / a tooltip.
    pub fn hint(self) -> &'static str {
        match self {
            Algo::Flow => "containers in record order, wrapping across the canvas",
            Algo::Packed => "the tightest fit, for when the whitespace bothers you",
            Algo::Tree => "containers hang under whoever spawned them",
            Algo::Columns => "one card per row, for a narrow window",
        }
    }

    /// Arrange one container's cards. The inner arrangement fixes the group's box, which is what
    /// the session-level packer then treats as rigid — the whole thing is solved bottom-up.
    fn inside(self, task: TaskId, agents: &[WorkAgent]) -> Contents {
        let members: Vec<&WorkAgent> = agents.iter().filter(|a| a.task == Some(task)).collect();
        match self {
            // Tree wants the connectors to run straight down, which is what the plain stack draws.
            Algo::Flow | Algo::Tree => stack(&members),
            Algo::Packed => stack_wrapped(&members),
            Algo::Columns => column(&members),
        }
    }
}

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
    pub fn auto(agents: &[WorkAgent], tasks: &[TaskRecord], algo: Algo) -> Self {
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
        // Sessions stack, each starting below the one before it. They used to be laid out from
        // the same origin, which was invisible while the canvas drew one at a time and is a pile
        // the moment it draws them all.
        let mut y = LAYOUT_MARGIN;
        for session in sessions {
            y = layout.arrange(session, y, agents, tasks, algo);
        }
        layout
    }

    /// Give anything the arrangement has never seen a place of its own, without moving what is
    /// already placed. `auto` is the whole arrangement recomputed and discards every hand-placed
    /// position, which is why only `tidy` may call it; this is what an arriving card gets.
    ///
    /// The place it gets is the one `auto` would have given it, taken from a tidy arrangement of
    /// everything and adopted for the unseen keys alone — so a new task takes the next container
    /// slot in the same order the chosen arrangement would have used, and a new agent the next
    /// offset inside its task, with no second geometry to keep in step with the first.
    pub fn place_new(&mut self, agents: &[WorkAgent], tasks: &[TaskRecord], algo: Algo) {
        let tidy = Self::auto(agents, tasks, algo);
        for (task, origin) in tidy.tasks {
            self.tasks.entry(task).or_insert(origin);
        }
        for (agent, offset) in tidy.agents {
            self.agents.entry(agent).or_insert(offset);
        }
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
    pub fn at(&self, agent: &WorkAgent) -> (f32, f32) {
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

    /// One session: the agents nobody gave work to along the top, then the containers underneath,
    /// packed the way `algo` asks for. Lay it out starting at `top`, and answer the `y` the next
    /// session starts at.
    fn arrange(
        &mut self,
        session: SessionId,
        top: f32,
        agents: &[WorkAgent],
        tasks: &[TaskRecord],
        algo: Algo,
    ) -> f32 {
        let mut y = top;

        // An agent with no task is usually the one handing work out, so it goes above the
        // containers rather than below them: a connector that runs down into a box reads better
        // than one that climbs out of the bottom of the graph. They are stacked rather than merely
        // laid in a row, because the set can be a spawn tree of its own — and stacked whichever
        // arrangement is chosen, because this block is the frame the session hangs off.
        let loose: Vec<&WorkAgent> = agents
            .iter()
            .filter(|a| a.session == session && a.task.is_none())
            .collect();
        let Contents { cards, height, .. } = stack(&loose);
        if !cards.is_empty() {
            for (agent, offset) in cards {
                self.agents
                    .insert(agent, (LAYOUT_MARGIN + offset.0, y + offset.1));
            }
            y += height + TASK_GAP;
        }

        // Bottom-up: pack the cards inside each container first, because that is what fixes the
        // box the session-level packer is then free to treat as rigid.
        let boxes: Vec<&TaskRecord> = tasks
            .iter()
            .filter(|t| t.session == Some(session))
            .collect();
        let contents: Vec<Contents> = boxes.iter().map(|t| algo.inside(t.id, agents)).collect();
        let sizes: Vec<(f32, f32)> = contents
            .iter()
            .map(|c| {
                if c.cards.is_empty() {
                    (0.0, 0.0)
                } else {
                    (
                        c.width + GROUP_PAD * 2.0,
                        c.height + GROUP_PAD * 2.0 + GROUP_LABEL,
                    )
                }
            })
            .collect();

        let (at, extent) = match algo {
            // The shelf keeps record order, which is what makes Flow the calm one to read.
            Algo::Flow | Algo::Columns => {
                pack_shelf(&sizes, LAYOUT_WIDTH - LAYOUT_MARGIN, TASK_GAP)
            }
            Algo::Packed => without_empties(&sizes, |kept| pack_best(kept, TASK_GAP)),
            Algo::Tree => {
                let parents = task_forest(&boxes, agents);
                without_empties(&sizes, |kept| {
                    let kept_parents = remap(&sizes, &parents);
                    pack_tree(kept, &kept_parents, TASK_GAP)
                })
            }
        };

        for (ix, task) in boxes.iter().enumerate() {
            // A container with nothing in it is not drawn, so it takes no room — but it still gets
            // an origin, because a card dropped into it needs a frame to hang off.
            self.tasks.insert(
                task.id,
                (
                    LAYOUT_MARGIN + at[ix].0 + GROUP_PAD,
                    y + at[ix].1 + GROUP_PAD + GROUP_LABEL,
                ),
            );
            for (agent, offset) in &contents[ix].cards {
                self.agents.insert(*agent, *offset);
            }
        }

        // Past the last row this session drew, so the next session clears it. A session that drew
        // nothing at all leaves `y` where it found it and costs no gap.
        if extent.1 > 0.0 {
            y += extent.1 + TASK_GAP;
        }
        y
    }
}

/// One container's contents: an offset per card, and the size those cards take up.
struct Contents {
    cards: Vec<(AgentId, (f32, f32))>,
    width: f32,
    height: f32,
}

impl Contents {
    fn empty() -> Self {
        Contents {
            cards: Vec::new(),
            width: 0.0,
            height: 0.0,
        }
    }
}

/// Stack one set of cards by how far each is from whoever started the work.
///
/// Cards are stacked by how far they are from whoever started the work — roots on the top row,
/// their children on the next — which draws the three task shapes without knowing about any of
/// them. One agent is one card. A chain is a column, because each link answers to the last. A
/// coordinated task is a coordinator over a row of workers.
///
/// Shared by a container and by the row of agents that have no task, because the second one holds a
/// spawn tree too: the agent coordinating a project parents each session's master, and drawing it
/// beside its own child rather than above it would send the connector sideways.
fn stack(members: &[&WorkAgent]) -> Contents {
    let rows = rows_by_depth(members);
    if rows.is_empty() {
        return Contents::empty();
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

/// The same depth rows, each folded into a near-square block instead of one long line.
///
/// **A wide row is what makes a canvas wide.** Eight workers on one task drag every other container
/// out past them, and the whitespace that leaves is the thing Packed exists to remove — so a row of
/// `n` breaks at about `ceil(sqrt(n))` cards, which is the squarest break there is.
fn stack_wrapped(members: &[&WorkAgent]) -> Contents {
    let rows = rows_by_depth(members);
    if rows.is_empty() {
        return Contents::empty();
    }

    let per_line: Vec<usize> = rows.iter().map(|row| break_at(row.len())).collect();
    let width = per_line
        .iter()
        .map(|n| row_width(*n))
        .fold(0.0f32, f32::max);

    let mut cards = Vec::new();
    let mut line = 0usize;
    for (row, per) in rows.iter().zip(&per_line) {
        for chunk in row.chunks(*per) {
            // Every line is centred, for the same reason a short row is: the card handing work out
            // belongs over the middle of the cards taking it.
            let start = (width - row_width(chunk.len())) / 2.0;
            for (ix, agent) in chunk.iter().enumerate() {
                cards.push((
                    *agent,
                    (
                        start + ix as f32 * (CARD_WIDTH + CARD_GAP_X),
                        line as f32 * (CARD_HEIGHT + CARD_GAP_Y),
                    ),
                ));
            }
            line += 1;
        }
    }

    let height = line as f32 * CARD_HEIGHT + line.saturating_sub(1) as f32 * CARD_GAP_Y;
    Contents {
        cards,
        width,
        height,
    }
}

/// One card per row, in spawn order. The tall, narrow container a small window has room for.
fn column(members: &[&WorkAgent]) -> Contents {
    let rows = rows_by_depth(members);
    if rows.is_empty() {
        return Contents::empty();
    }

    let cards: Vec<(AgentId, (f32, f32))> = rows
        .iter()
        .flatten()
        .enumerate()
        .map(|(ix, agent)| (*agent, (0.0, ix as f32 * (CARD_HEIGHT + CARD_GAP_Y))))
        .collect();
    let height =
        cards.len() as f32 * CARD_HEIGHT + cards.len().saturating_sub(1) as f32 * CARD_GAP_Y;
    Contents {
        cards,
        width: CARD_WIDTH,
        height,
    }
}

/// The cards of one set, gathered into a row per hand-off depth. What every inner arrangement
/// starts from, so they disagree about shape rather than about who answers to whom.
fn rows_by_depth(members: &[&WorkAgent]) -> Vec<Vec<AgentId>> {
    if members.is_empty() {
        return Vec::new();
    }

    // Only a parent inside the same set counts. An agent answering to one outside it is a root
    // here, and the connector to its parent is drawn across the boundary.
    let mut parents: HashMap<AgentId, AgentId> = HashMap::new();
    for agent in members {
        if let Some(parent) = agent.parent
            && members.iter().any(|m| m.id == parent)
        {
            parents.insert(agent.id, parent);
        }
    }

    let mut rows: Vec<Vec<AgentId>> = Vec::new();
    for agent in members {
        let depth = depth_of(agent.id, &parents);
        if rows.len() <= depth {
            rows.resize(depth + 1, Vec::new());
        }
        rows[depth].push(agent.id);
    }
    rows
}

fn row_width(cards: usize) -> f32 {
    cards as f32 * CARD_WIDTH + cards.saturating_sub(1) as f32 * CARD_GAP_X
}

/// How many cards a wrapped row takes before it folds: the side of the smallest square that holds
/// them all.
fn break_at(cards: usize) -> usize {
    (cards as f32).sqrt().ceil().max(1.0) as usize
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

/// Position boxes in the order given, wrapping onto a new shelf when the next one would pass
/// `width`, and answer the extent the lot takes.
///
/// **A box with no size takes the cursor and no room.** A container with nothing in it is not
/// drawn, so it must not push the next one along — but it still needs a frame, and the frame it
/// gets is where the next container would have started.
fn pack_shelf(sizes: &[(f32, f32)], width: f32, gap: f32) -> Packing {
    let mut at = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
    let mut extent = (0.0f32, 0.0f32);
    for &(w, h) in sizes {
        if w <= 0.0 && h <= 0.0 {
            at.push((x, y));
            continue;
        }
        if x > 0.0 && x + w > width {
            x = 0.0;
            y += row_h + gap;
            row_h = 0.0;
        }
        at.push((x, y));
        x += w + gap;
        row_h = row_h.max(h);
        extent.0 = extent.0.max(x - gap);
        extent.1 = extent.1.max(y + row_h);
    }
    (at, extent)
}

/// Best-fit skyline packing into a container `width`.
///
/// Boxes are placed biggest first — a large box dropped into a nearly full canvas has nowhere to go
/// — and each takes the ledge that grows the bounding box least, so the result stays compact
/// instead of merely filling rows. **Positions come back indexed the same as `sizes`** even though
/// the packer visits them sorted, because the caller is placing containers it named in record
/// order.
fn pack_skyline(sizes: &[(f32, f32)], width: f32, gap: f32) -> Packing {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|&a, &b| {
        longest(sizes[b])
            .total_cmp(&longest(sizes[a]))
            .then(a.cmp(&b))
    });

    // Every box carries its gap on the right and below, so nothing downstream has to reason about
    // the space between two of them: boxes that touch in the packer are `gap` apart on the canvas.
    let room = width + gap;
    // The skyline is a fence: each node is (x, height) and holds until the next node's x.
    let mut fence: Vec<(f32, f32)> = vec![(0.0, 0.0)];
    let mut at = vec![(0.0f32, 0.0f32); sizes.len()];
    let mut extent = (0.0f32, 0.0f32);

    for ix in order {
        let (w, h) = (sizes[ix].0 + gap, sizes[ix].1 + gap);
        let mut best: Option<(f32, f32, f32)> = None;
        for node in 0..fence.len() {
            let x = fence[node].0;
            if x + w > room + EPS {
                continue;
            }
            let y = ledge(&fence, x, w);
            let grown = extent.0.max(x + w - gap) * extent.1.max(y + h - gap);
            // Ties go to the topmost, then the leftmost: two placements that cost the same should
            // resolve the same way every tidy, and reading order is the one the eye expects.
            if best.is_none_or(|(bg, bx, by)| {
                grown < bg - EPS || (grown < bg + EPS && (y < by - EPS || (y < by + EPS && x < bx)))
            }) {
                best = Some((grown, x, y));
            }
        }
        // Nothing fits only when a single box is wider than the container, which `pack_best` rules
        // out by clamping. On top of the pile is then the one placement that cannot overlap.
        let (x, y) = match best {
            Some((_, x, y)) => (x, y),
            None => (0.0, fence.iter().map(|n| n.1).fold(0.0f32, f32::max)),
        };
        raise(&mut fence, x, w, y + h);
        at[ix] = (x, y);
        extent.0 = extent.0.max(x + w - gap);
        extent.1 = extent.1.max(y + h - gap);
    }
    (at, extent)
}

fn longest(size: (f32, f32)) -> f32 {
    size.0.max(size.1)
}

/// How high a box starting at `x` and `w` wide has to sit to clear the fence under it.
fn ledge(fence: &[(f32, f32)], x: f32, w: f32) -> f32 {
    let mut y = 0.0f32;
    for (ix, &(sx, height)) in fence.iter().enumerate() {
        let end = fence.get(ix + 1).map(|n| n.0).unwrap_or(f32::INFINITY);
        if end <= x + EPS || sx >= x + w - EPS {
            continue;
        }
        y = y.max(height);
    }
    y
}

/// Raise the fence over `[x, x + w)` to `top`, which is what placing a box does to it.
fn raise(fence: &mut Vec<(f32, f32)>, x: f32, w: f32, top: f32) {
    let right = x + w;
    let beyond = height_at(fence, right);
    fence.retain(|&(sx, _)| sx < x - EPS || sx > right + EPS);
    fence.push((x, top));
    fence.push((right, beyond));
    fence.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Two nodes at one x, or a node at the height it already was, say nothing.
    fence.dedup_by(|b, a| b.0 <= a.0 + EPS || (b.1 - a.1).abs() < EPS);
}

fn height_at(fence: &[(f32, f32)], x: f32) -> f32 {
    fence
        .iter()
        .rev()
        .find(|(sx, _)| *sx <= x + EPS)
        .map(|(_, height)| *height)
        .unwrap_or(0.0)
}

/// How bad a packing is — smaller is better.
///
/// Area alone would take a long ribbon over a square of the same size, and a square is what fits on
/// a screen, so the bounding box is charged for how far it is from square and for the room it
/// wastes. Both charges are fractions of the area, which keeps them unitless: **these two weights
/// are the whole tuning surface**, and the gaps between boxes count as waste because they are
/// constant across the candidates being compared.
fn score(extent: (f32, f32), content: f32) -> f32 {
    /// What a bounding box twice as wide as it is tall costs: a third of its area.
    const OFF_SQUARE: f32 = 0.35;
    /// What a bounding box of pure whitespace would cost: half as much again.
    const WASTED: f32 = 0.5;

    let area = extent.0 * extent.1;
    if area <= 0.0 {
        return 0.0;
    }
    let aspect = (extent.0 / extent.1).max(extent.1 / extent.0);
    let waste = ((area - content) / area).clamp(0.0, 1.0);
    area * (1.0 + OFF_SQUARE * (aspect - 1.0) + WASTED * waste)
}

/// Pack against a handful of container widths and keep the best-scoring result.
///
/// Nothing here searches. Four candidates either side of the square root of the content's area,
/// never narrower than the widest single box, is enough to beat any fixed width; a local search
/// over the same boxes would cost a frame's arithmetic for a difference nobody can see.
fn pack_best(sizes: &[(f32, f32)], gap: f32) -> Packing {
    let content: f32 = sizes.iter().map(|(w, h)| w * h).sum();
    let widest = sizes.iter().map(|s| s.0).fold(0.0f32, f32::max);
    let side = content.sqrt();

    let mut best: Option<(Packing, f32)> = None;
    for ratio in [0.7f32, 1.0, 1.4, 1.9] {
        let packed = pack_skyline(sizes, (side * ratio).max(widest), gap);
        let marked = score(packed.1, content);
        if best.as_ref().is_none_or(|(_, was)| marked < *was) {
            best = Some((packed, marked));
        }
    }
    best.map(|(packed, _)| packed)
        .unwrap_or((Vec::new(), (0.0, 0.0)))
}

/// Lay a forest of boxes out tidy-tree style: a parent centred over the span of its children.
///
/// The boxes are different sizes, so a subtree is as wide as the wider of its own box and the row
/// of children under it, and that width is what siblings are laid out against. Depth advances by
/// the tallest box on the row above, so a deep row never rides up into a short one.
fn pack_tree(sizes: &[(f32, f32)], parents: &[Option<usize>], gap: f32) -> Packing {
    let n = sizes.len();
    if n == 0 {
        return (Vec::new(), (0.0, 0.0));
    }

    // A parent relation read off records can close a loop, which is not a tree and must not be a
    // hang: the edge that closes one is dropped and its node becomes a root.
    let mut parent = parents.to_vec();
    for node in 0..n {
        let mut at = node;
        let mut steps = 0;
        while let Some(up) = parent[at] {
            if up == node || steps > n {
                parent[node] = None;
                break;
            }
            at = up;
            steps += 1;
        }
        if parent[node] == Some(node) {
            parent[node] = None;
        }
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (node, up) in parent.iter().enumerate() {
        if let Some(up) = up {
            children[*up].push(node);
        }
    }

    let mut depth = vec![0usize; n];
    for (node, deep) in depth.iter_mut().enumerate() {
        let mut at = parent[node];
        while let Some(up) = at {
            *deep += 1;
            at = parent[up];
        }
    }

    // Rows are as tall as their tallest box, so the whole depth reads as one band.
    let deepest = depth.iter().copied().max().unwrap_or(0);
    let mut row_h = vec![0.0f32; deepest + 1];
    for (size, &row) in sizes.iter().zip(&depth) {
        row_h[row] = row_h[row].max(size.1);
    }
    let mut row_y = vec![0.0f32; deepest + 1];
    for row in 1..=deepest {
        row_y[row] = row_y[row - 1] + row_h[row - 1] + gap;
    }

    // Deepest first, so a subtree's width is known before its parent asks for it.
    let mut span: Vec<f32> = sizes.iter().map(|s| s.0).collect();
    let mut deep_first: Vec<usize> = (0..n).collect();
    deep_first.sort_by_key(|&node| std::cmp::Reverse(depth[node]));
    for &node in &deep_first {
        if !children[node].is_empty() {
            span[node] = span[node].max(brood(&children[node], &span, gap));
        }
    }

    let mut at = vec![(0.0f32, 0.0f32); n];
    let mut cursor = 0.0f32;
    let mut extent = (0.0f32, 0.0f32);
    let mut pending: Vec<(usize, f32)> = Vec::new();
    for node in (0..n).filter(|&node| parent[node].is_none()) {
        pending.push((node, cursor));
        cursor += span[node] + gap;
    }
    while let Some((node, left)) = pending.pop() {
        let x = left + (span[node] - sizes[node].0) / 2.0;
        let y = row_y[depth[node]];
        at[node] = (x, y);
        extent.0 = extent.0.max(x + sizes[node].0);
        extent.1 = extent.1.max(y + sizes[node].1);

        let mut under = left + (span[node] - brood(&children[node], &span, gap)) / 2.0;
        for &child in &children[node] {
            pending.push((child, under));
            under += span[child] + gap;
        }
    }
    (at, extent)
}

/// How wide a row of children is, gaps included.
fn brood(children: &[usize], span: &[f32], gap: f32) -> f32 {
    if children.is_empty() {
        return 0.0;
    }
    children.iter().map(|&c| span[c]).sum::<f32>() + gap * (children.len() - 1) as f32
}

/// Pack only the containers that are actually drawn, and answer positions for all of them.
///
/// An empty container has no box, so a packer that reasons about area would be packing nothing and
/// charging a gap for it. It gets the session's own corner instead — a frame for a card to be
/// dropped into, costing no room.
fn without_empties(sizes: &[(f32, f32)], pack: impl FnOnce(&[(f32, f32)]) -> Packing) -> Packing {
    let kept: Vec<usize> = (0..sizes.len())
        .filter(|&ix| sizes[ix].0 > 0.0 || sizes[ix].1 > 0.0)
        .collect();
    let dense: Vec<(f32, f32)> = kept.iter().map(|&ix| sizes[ix]).collect();
    let (packed, extent) = pack(&dense);

    let mut at = vec![(0.0f32, 0.0f32); sizes.len()];
    for (dense_ix, &ix) in kept.iter().enumerate() {
        at[ix] = packed[dense_ix];
    }
    (at, extent)
}

/// The parent relation over the containers that are drawn, renumbered to match what
/// [`without_empties`] passes the packer. An empty container is never a parent — a parent is a
/// container holding the card somebody answers to — so nothing is lost by dropping it.
fn remap(sizes: &[(f32, f32)], parents: &[Option<usize>]) -> Vec<Option<usize>> {
    let mut dense = vec![None; sizes.len()];
    let mut next = 0usize;
    for ix in 0..sizes.len() {
        if sizes[ix].0 > 0.0 || sizes[ix].1 > 0.0 {
            dense[ix] = Some(next);
            next += 1;
        }
    }
    (0..sizes.len())
        .filter(|&ix| dense[ix].is_some())
        .map(|ix| parents[ix].and_then(|up| dense[up]))
        .collect()
}

/// Which container hangs under which.
///
/// A container's root cards answer to somebody outside it, and whatever container holds that
/// somebody is this one's parent. A parent with no task of its own — the loose block along the top
/// — leaves the container a root, because that block is the frame the whole session already hangs
/// off and drawing a second one under it would say the same thing twice.
fn task_forest(tasks: &[&TaskRecord], agents: &[WorkAgent]) -> Vec<Option<usize>> {
    let mut parents = vec![None; tasks.len()];
    for (ix, task) in tasks.iter().enumerate() {
        let members: Vec<&WorkAgent> = agents.iter().filter(|a| a.task == Some(task.id)).collect();
        for member in &members {
            let Some(spawner) = member.parent else {
                continue;
            };
            // A card answering to one in the same container is not a root of it.
            if members.iter().any(|m| m.id == spawner) {
                continue;
            }
            let above = agents
                .iter()
                .find(|a| a.id == spawner)
                .and_then(|a| a.task)
                .and_then(|task| tasks.iter().position(|t| t.id == task));
            if let Some(above) = above
                && above != ix
            {
                parents[ix] = Some(above);
                break;
            }
        }
    }
    parents
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::work::{Activity, Priority, Status};

    fn agent(session: SessionId, task: Option<TaskId>, parent: Option<AgentId>) -> WorkAgent {
        WorkAgent {
            id: AgentId::generate(),
            session,
            task,
            parent,
            name: "card".to_string(),
            summary: None,
            role: "Implementer".to_string(),
            activity: Activity::Writing,
            note: String::new(),
            branch: "main".to_string(),
            tokens: 0.0,
            harness: "Claude Code".to_string(),
            account: String::new(),
            model: String::new(),
            context_pct: 0,
            persistent: false,
            accept_all: false,
            debug_dump: None,
            thread: Vec::new(),
        }
    }

    fn task(id: TaskId, session: SessionId) -> TaskRecord {
        TaskRecord {
            id,
            session: Some(session),
            status: Status::Backlog,
            priority: Priority::Normal,
            shape: None,
            kind: None,
            key: None,
            link: None,
            labels: Vec::new(),
            title: "task".to_string(),
            description: String::new(),
            steps: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// A spread of boxes wide, tall and square, which is what a real session looks like.
    fn spread() -> Vec<(f32, f32)> {
        vec![
            (300.0, 200.0),
            (120.0, 480.0),
            (640.0, 160.0),
            (240.0, 240.0),
            (180.0, 90.0),
            (520.0, 320.0),
            (90.0, 700.0),
        ]
    }

    fn overlaps(a: ((f32, f32), (f32, f32)), b: ((f32, f32), (f32, f32)), gap: f32) -> bool {
        let apart_x = a.0.0 + a.1.0 + gap <= b.0.0 + EPS || b.0.0 + b.1.0 + gap <= a.0.0 + EPS;
        let apart_y = a.0.1 + a.1.1 + gap <= b.0.1 + EPS || b.0.1 + b.1.1 + gap <= a.0.1 + EPS;
        !(apart_x || apart_y)
    }

    fn clear(at: &[(f32, f32)], sizes: &[(f32, f32)], gap: f32) {
        for i in 0..sizes.len() {
            for j in i + 1..sizes.len() {
                assert!(
                    !overlaps((at[i], sizes[i]), (at[j], sizes[j]), gap),
                    "box {i} at {:?} and box {j} at {:?} are closer than {gap}",
                    at[i],
                    at[j]
                );
            }
        }
    }

    #[test]
    fn a_shelf_keeps_its_boxes_apart_and_wraps() {
        let sizes = spread();
        let (at, extent) = pack_shelf(&sizes, 900.0, TASK_GAP);
        clear(&at, &sizes, TASK_GAP);
        assert!(extent.0 <= 900.0, "nothing spills past the shelf's width");
        assert!(at.iter().any(|p| p.1 > 0.0), "and a second row was needed");
    }

    #[test]
    fn a_skyline_keeps_its_boxes_apart() {
        let sizes = spread();
        let (at, extent) = pack_skyline(&sizes, 800.0, TASK_GAP);
        clear(&at, &sizes, TASK_GAP);
        for (ix, (w, h)) in sizes.iter().enumerate() {
            assert!(at[ix].0 >= -EPS && at[ix].1 >= -EPS);
            assert!(at[ix].0 + w <= extent.0 + EPS && at[ix].1 + h <= extent.1 + EPS);
        }
    }

    /// Packed is only worth having if it beats the shelf it replaces, and only readable if the
    /// result still fits on a screen rather than trailing off one edge.
    #[test]
    fn packing_beats_the_shelf_and_stays_squarish() {
        let sizes = spread();
        let (packed, tight) = pack_best(&sizes, TASK_GAP);
        clear(&packed, &sizes, TASK_GAP);

        let (_, shelf) = pack_shelf(&sizes, LAYOUT_WIDTH - LAYOUT_MARGIN, TASK_GAP);
        assert!(
            tight.0 * tight.1 <= shelf.0 * shelf.1 + EPS,
            "packed {tight:?} is no larger than the shelf's {shelf:?}"
        );
        let aspect = (tight.0 / tight.1).max(tight.1 / tight.0);
        assert!(
            aspect <= 2.5,
            "and reads as a block, not a ribbon: {aspect}"
        );
    }

    #[test]
    fn a_tree_centres_a_parent_over_its_children() {
        // One root with three children under it, all the same size.
        let sizes = vec![(200.0, 100.0); 4];
        let parents = vec![None, Some(0), Some(0), Some(0)];
        let (at, extent) = pack_tree(&sizes, &parents, TASK_GAP);
        clear(&at, &sizes, TASK_GAP);

        let middle = |ix: usize| at[ix].0 + sizes[ix].0 / 2.0;
        let span = (middle(1) + middle(3)) / 2.0;
        assert!(
            (middle(0) - span).abs() < EPS,
            "the parent's middle {} is over its children's {span}",
            middle(0)
        );
        assert!(
            (middle(0) - middle(2)).abs() < EPS,
            "which is over the middle child"
        );
        assert!(at[1].1 - at[0].1 >= sizes[0].1, "and a row below it");
        assert!(extent.1 >= sizes[0].1 + sizes[1].1);
    }

    /// A parent relation that loops is a record nobody can draw, and the packer must answer rather
    /// than spin.
    #[test]
    fn a_tree_survives_a_loop() {
        let sizes = vec![(200.0, 100.0); 3];
        let parents = vec![Some(2), Some(0), Some(1)];
        let (at, _) = pack_tree(&sizes, &parents, TASK_GAP);
        assert_eq!(at.len(), 3);
        clear(&at, &sizes, TASK_GAP);
    }

    #[test]
    fn columns_put_every_card_in_one_column() {
        let session = SessionId::generate();
        let job = TaskId::generate();
        let lead = agent(session, Some(job), None);
        let members = vec![
            lead.clone(),
            agent(session, Some(job), Some(lead.id)),
            agent(session, Some(job), Some(lead.id)),
        ];
        let contents = Algo::Columns.inside(job, &members);
        assert_eq!(contents.cards.len(), 3);
        assert!(contents.cards.iter().all(|(_, at)| at.0 == 0.0));
        assert_eq!(contents.width, CARD_WIDTH);
        for pair in contents.cards.windows(2) {
            assert_eq!(pair[1].1.1 - pair[0].1.1, CARD_HEIGHT + CARD_GAP_Y);
        }
    }

    /// A wide row folds instead of dragging the canvas out after it, and the fold is square.
    #[test]
    fn packed_folds_a_wide_row() {
        let session = SessionId::generate();
        let job = TaskId::generate();
        let lead = agent(session, Some(job), None);
        let mut members = vec![lead.clone()];
        members.extend((0..8).map(|_| agent(session, Some(job), Some(lead.id))));

        let wide = Algo::Flow.inside(job, &members);
        let folded = Algo::Packed.inside(job, &members);
        assert_eq!(wide.width, row_width(8));
        assert_eq!(folded.width, row_width(3), "eight workers break three wide");
        assert!(folded.height > wide.height, "which costs rows");
        assert_eq!(folded.cards.len(), 9);
    }

    /// **Flow is the arrangement nothing may quietly change.** The numbers are written out rather
    /// than derived, so an "improvement" to the shared packers has to admit to moving them.
    #[test]
    fn flow_is_unchanged() {
        let session = SessionId::generate();
        let first = TaskId::generate();
        let second = TaskId::generate();
        let one = agent(session, Some(first), None);
        let two = agent(session, Some(first), None);
        let three = agent(session, Some(second), Some(one.id));
        let agents = vec![one.clone(), two.clone(), three.clone()];
        let tasks = vec![task(first, session), task(second, session)];

        let layout = Layout::auto(&agents, &tasks, Algo::Flow);

        // The first container starts at the margin, inside its own padding and under its label.
        assert_eq!(layout.task_origin(first), (46.0, 72.0));
        // Two cards side by side, so the box is 2 * 264 + 32 + 2 * 22 = 604 wide, and the next
        // container starts a TASK_GAP past it.
        assert_eq!(layout.task_origin(second), (46.0 + 604.0 + 56.0, 72.0));

        assert_eq!(layout.offset(one.id), (0.0, 0.0));
        assert_eq!(layout.offset(two.id), (CARD_WIDTH + CARD_GAP_X, 0.0));
        // The third card's parent is in another container, so it is a root of its own and centred
        // alone.
        assert_eq!(layout.offset(three.id), (0.0, 0.0));
        assert_eq!(layout.at(&three), (706.0, 72.0));
    }

    /// The loose block is the frame a session hangs off, whichever arrangement is chosen: it stays
    /// above the containers, and every arrangement puts a container clear of the block and of every
    /// other container.
    #[test]
    fn every_arrangement_clears_the_loose_block() {
        let session = SessionId::generate();
        let first = TaskId::generate();
        let second = TaskId::generate();
        let boss = agent(session, None, None);
        let one = agent(session, Some(first), Some(boss.id));
        let two = agent(session, Some(second), Some(one.id));
        let agents = vec![boss.clone(), one.clone(), two.clone()];
        let tasks = vec![task(first, session), task(second, session)];

        for algo in Algo::ALL {
            let layout = Layout::auto(&agents, &tasks, algo);
            let at = layout.at(&boss);
            assert_eq!(at, (LAYOUT_MARGIN, LAYOUT_MARGIN), "{}", algo.label());
            for card in [&one, &two] {
                assert!(
                    layout.at(card).1 >= at.1 + CARD_HEIGHT,
                    "{} draws a container under the loose block",
                    algo.label()
                );
            }
            assert_ne!(
                layout.task_origin(first),
                layout.task_origin(second),
                "{} keeps its containers apart",
                algo.label()
            );
        }
    }

    /// An empty container is drawn nowhere and still has a frame, so a card dropped into it lands
    /// somewhere sensible rather than at the canvas origin.
    #[test]
    fn an_empty_container_still_gets_a_frame() {
        let session = SessionId::generate();
        let job = TaskId::generate();
        let nobodys = TaskId::generate();
        let agents = vec![agent(session, Some(job), None)];
        let tasks = vec![task(job, session), task(nobodys, session)];

        for algo in Algo::ALL {
            let layout = Layout::auto(&agents, &tasks, algo);
            let origin = layout.task_origin(nobodys);
            assert!(
                origin.0 >= LAYOUT_MARGIN && origin.1 >= LAYOUT_MARGIN,
                "{} left the empty container off the canvas",
                algo.label()
            );
        }
    }

    /// Tree hangs a container under the one holding whoever spawned its cards.
    #[test]
    fn a_tree_hangs_a_container_under_its_spawner() {
        let session = SessionId::generate();
        let up = TaskId::generate();
        let down = TaskId::generate();
        let lead = agent(session, Some(up), None);
        let worker = agent(session, Some(down), Some(lead.id));
        let agents = vec![lead.clone(), worker.clone()];
        let tasks = vec![task(up, session), task(down, session)];

        let parents = task_forest(&tasks.iter().collect::<Vec<_>>(), &agents);
        assert_eq!(parents, vec![None, Some(0)]);

        let layout = Layout::auto(&agents, &tasks, Algo::Tree);
        let above = layout.at(&lead);
        let below = layout.at(&worker);
        assert!(below.1 > above.1, "the child container is a row down");
        assert!(
            (above.0 - below.0).abs() < EPS,
            "and centred under it: {above:?} over {below:?}"
        );
    }
}
