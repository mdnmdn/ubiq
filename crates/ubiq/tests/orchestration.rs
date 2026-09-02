//! The orchestration screen's logic, without a frame.
//!
//! Everything the graph decides — how it arranges itself, which cards are drawn, where a task's
//! container falls, what a drop means, which tasks a selection lists, how long a grain of sand
//! lasts — is arithmetic over plain data, so it is tested the way the explorer's tree and the
//! window registry are: on the state alone, seeded the way the host would have filled it.
//!
//! The records are `ubiq_proto::work`'s own and arrive through `WorkProjection`, which is what the
//! window does with a `WorkList`; the view over them is a `GraphView`, which holds no records and
//! takes the projection as the first argument of every reader. Ids are minted rather than written
//! down — a `TaskId` is a ULID the host mints, so the fixture binds each one to a name and the
//! tests say `f.parser` where they used to say `2`.
//!
//! Positions are never asserted against the fixture, because the fixture has none. A test that
//! needs a card somewhere in particular puts it there with `place`, which is the same call a drag
//! makes.

use std::time::{Duration, Instant};

use ubiq::state::layout::{CARD_GAP_X, CARD_GAP_Y};
use ubiq::state::orchestration::{
    CARD_HEIGHT, CARD_WIDTH, GRAIN_CEILING, GRAIN_LIFE, GROUP_LABEL, GROUP_PAD, GraphView, Held,
    Selection, ZOOM_MAX, ZOOM_MIN,
};
use ubiq::state::work::WorkProjection;
use ubiq_proto::ids::{SessionId, TaskId};
use ubiq_proto::work::{
    Activity, AgentId, Bucket, Priority, Shape, Status, Step, StepState, TaskRecord, WorkAgent,
    WorkSession,
};

fn session(id: SessionId, name: &str) -> WorkSession {
    WorkSession {
        id,
        name: name.to_string(),
        branch: "main".to_string(),
        worktree: false,
    }
}

fn agent(id: AgentId, session: SessionId, task: Option<TaskId>, name: &str) -> WorkAgent {
    WorkAgent {
        id,
        session,
        task,
        parent: None,
        name: name.to_string(),
        role: "Implementer".to_string(),
        activity: Activity::Writing,
        note: "doing a thing".to_string(),
        branch: "main".to_string(),
        tokens: 10_000.0,
        harness: "Claude Code".to_string(),
        model: "Opus 4.6".to_string(),
        context_pct: 5,
        thread: Vec::new(),
    }
}

/// The same card, doing something else. Written as a builder step so the fixture reads as a list of
/// agents rather than a list of mutations.
fn doing(mut agent: WorkAgent, activity: Activity) -> WorkAgent {
    agent.activity = activity;
    agent
}

fn task(id: TaskId, session: SessionId, title: &str, owners: &[Option<AgentId>]) -> TaskRecord {
    TaskRecord {
        id,
        session: Some(session),
        status: Status::Backlog,
        priority: Priority::Normal,
        shape: Shape::Direct,
        title: title.to_string(),
        description: String::new(),
        steps: owners
            .iter()
            .enumerate()
            .map(|(ix, owner)| {
                let mut step = Step::new(format!("step-{ix}"));
                step.state = if ix == 0 {
                    StepState::Done
                } else {
                    StepState::Idle
                };
                step.owner = *owner;
                step
            })
            .collect(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// A record the host has sent again, changed. The projection replaces on id, so this is the whole
/// of how anything in the work changes — nothing on this side edits a record in place.
fn edit_agent(work: &mut WorkProjection, id: AgentId, edit: impl FnOnce(&mut WorkAgent)) {
    let mut agent = work.agent(id).expect("the fixture has that agent").clone();
    edit(&mut agent);
    work.apply_agent(agent);
}

/// One project's work and the graph's view of it, with a name for every id in it.
struct Fixture {
    work: WorkProjection,
    graph: GraphView,
    refit: SessionId,
    spike: SessionId,
    writer: AgentId,
    waiter: AgentId,
    stopped: AgentId,
    broken: AgentId,
    loose: AgentId,
    plumbing: TaskId,
    parser: TaskId,
    elsewhere: TaskId,
}

/// Two sessions, three tasks, five agents — one in each bucket, and one in the other session that
/// nobody has given work to.
fn seeded() -> Fixture {
    let refit = SessionId::generate();
    let spike = SessionId::generate();
    let writer = AgentId::generate();
    let waiter = AgentId::generate();
    let stopped = AgentId::generate();
    let broken = AgentId::generate();
    let loose = AgentId::generate();
    let plumbing = TaskId::generate();
    let parser = TaskId::generate();
    let elsewhere = TaskId::generate();

    let mut work = WorkProjection::empty();
    work.replace_all(
        vec![session(refit, "refit"), session(spike, "spike")],
        vec![
            doing(
                agent(writer, refit, Some(plumbing), "writer"),
                Activity::Writing,
            ),
            doing(
                agent(waiter, refit, Some(plumbing), "waiter"),
                Activity::NeedsYou,
            ),
            doing(
                agent(stopped, refit, Some(parser), "stopped"),
                Activity::Ended,
            ),
            doing(
                agent(broken, refit, Some(parser), "broken"),
                Activity::Failed,
            ),
            doing(agent(loose, spike, None, "loose"), Activity::Thinking),
        ],
        vec![
            task(plumbing, refit, "plumbing", &[Some(writer), Some(waiter)]),
            task(parser, refit, "parser", &[Some(stopped)]),
            task(elsewhere, spike, "elsewhere", &[Some(loose)]),
        ],
    );

    let mut graph = GraphView::default();
    graph.relayout(&work);
    Fixture {
        work,
        graph,
        refit,
        spike,
        writer,
        waiter,
        stopped,
        broken,
        loose,
        plumbing,
        parser,
        elsewhere,
    }
}

/// The same fixture with the four cards of the first session put where the geometry tests want
/// them.
fn placed() -> Fixture {
    let mut f = seeded();
    f.graph.place(&f.work, f.writer, (0.0, 0.0));
    f.graph.place(&f.work, f.waiter, (400.0, 0.0));
    f.graph.place(&f.work, f.stopped, (0.0, 400.0));
    f.graph.place(&f.work, f.broken, (400.0, 400.0));
    f
}

/// Pointing the screen at the first agent belongs to whoever first learns there is one — the window
/// does it when a `WorkList` arrives. What the view guarantees on its own is the fallback: with
/// nothing selected the screen is about the first session, so a graph is never about no session.
#[test]
fn a_new_screen_selects_nothing_and_shows_the_first_session() {
    let f = seeded();
    assert_eq!(f.graph.selection, None);
    assert_eq!(f.graph.active_session(&f.work), Some(f.refit));
}

/// The screen opens on the whole of the work, and narrows only when asked to.
///
/// Which session is *drawn* is its own field, and it starts absent. Selecting a session does not
/// narrow the canvas by itself — the two questions came apart so that "show me all of it" could
/// stop meaning "stop looking at this".
#[test]
fn the_graph_opens_on_every_session_and_narrows_only_when_told_to() {
    let mut f = seeded();
    assert_eq!(f.graph.session, None);
    assert!(f.graph.visible(f.work.agent(f.writer).unwrap()));
    assert!(f.graph.visible(f.work.agent(f.loose).unwrap()));

    // A selection on its own draws nothing away.
    f.graph.selection = Some(Selection::Session(f.spike));
    assert!(f.graph.visible(f.work.agent(f.writer).unwrap()));

    f.graph.show_session(Some(f.spike));
    assert!(!f.graph.visible(f.work.agent(f.writer).unwrap()));
    assert!(f.graph.visible(f.work.agent(f.loose).unwrap()));

    // And `all` puts them back without touching what is selected.
    f.graph.show_session(None);
    assert!(f.graph.visible(f.work.agent(f.writer).unwrap()));
    assert_eq!(f.graph.selection, Some(Selection::Session(f.spike)));
}

/// A bucket pill narrows to its bucket, and a row with none lit narrows to nothing.
///
/// That is the way back from having turned them all off, and it is why any pill may be the last
/// one: an empty canvas nobody can undo is the failure the old "the last pill stays on" rule was
/// avoiding, and this avoids it without making a pill refuse a click.
#[test]
fn a_bucket_pill_hides_its_bucket_and_a_row_with_none_lit_hides_nothing() {
    let mut f = seeded();
    f.graph.toggle_bucket(Bucket::Error);
    assert!(!f.graph.showing(Bucket::Error));
    assert!(!f.graph.visible(f.work.agent(f.broken).unwrap()));

    f.graph.toggle_bucket(Bucket::Error);
    assert!(f.graph.showing(Bucket::Error));

    // Every one of them off, which the row allows, and then everything is drawn again.
    for bucket in Bucket::all() {
        f.graph.toggle_bucket(bucket);
    }
    assert!(f.graph.buckets.is_empty());
    for bucket in Bucket::all() {
        assert!(f.graph.showing(bucket), "{bucket:?} with no pill lit");
    }
    assert!(f.graph.visible(f.work.agent(f.broken).unwrap()));
}

/// One control puts every filter back, and says whether it has anything to do.
#[test]
fn clearing_the_filters_draws_everything_and_knows_when_it_is_needed() {
    let mut f = seeded();
    assert!(!f.graph.filtered(), "nothing is hidden to begin with");

    f.graph.toggle_bucket(Bucket::Error);
    assert!(f.graph.filtered());
    f.graph.clear_filters();
    assert!(!f.graph.filtered());

    f.graph.show_session(Some(f.spike));
    assert!(f.graph.filtered());
    f.graph.clear_filters();
    assert!(!f.graph.filtered());
    assert_eq!(f.graph.session, None);
    assert_eq!(f.graph.buckets.len(), Bucket::all().len());
}

#[test]
fn the_graph_lays_itself_out_from_the_definitions_alone() {
    let f = seeded();

    // Two cards on one task, neither answering to the other, so they sit side by side.
    let one = f.graph.at_id(&f.work, f.writer).unwrap();
    let two = f.graph.at_id(&f.work, f.waiter).unwrap();
    assert_eq!(one.1, two.1, "same row");
    assert_eq!(two.0 - one.0, CARD_WIDTH + CARD_GAP_X);

    // A hand-off is a column: a card that answers to another in the same container goes under it.
    let session_id = SessionId::generate();
    let lead = AgentId::generate();
    let next = AgentId::generate();
    let handoff = TaskId::generate();
    let mut second = agent(next, session_id, Some(handoff), "next");
    second.parent = Some(lead);
    let mut chained = WorkProjection::empty();
    chained.replace_all(
        vec![session(session_id, "chain")],
        vec![agent(lead, session_id, Some(handoff), "lead"), second],
        vec![task(
            handoff,
            session_id,
            "handoff",
            &[Some(lead), Some(next)],
        )],
    );
    let mut view = GraphView::default();
    view.relayout(&chained);
    let one = view.at_id(&chained, lead).unwrap();
    let two = view.at_id(&chained, next).unwrap();
    assert_eq!(one.0, two.0, "same column");
    assert_eq!(two.1 - one.1, CARD_HEIGHT + CARD_GAP_Y);

    // Two containers in the same session never overlap.
    let (ax, _, aw, _) = f.graph.bounds_of(&f.work, f.plumbing).unwrap();
    let (bx, _, _, _) = f.graph.bounds_of(&f.work, f.parser).unwrap();
    assert!(bx >= ax + aw, "containers are laid out clear of each other");
}

#[test]
fn tidying_puts_a_dragged_card_back() {
    let mut f = seeded();
    let home = f.graph.at_id(&f.work, f.writer).unwrap();

    f.graph.place(&f.work, f.writer, (2_000.0, 2_000.0));
    assert_eq!(f.graph.at_id(&f.work, f.writer), Some((2_000.0, 2_000.0)));

    f.graph.relayout(&f.work);
    assert_eq!(f.graph.at_id(&f.work, f.writer), Some(home));
}

/// An arriving card is given a place of its own, and nothing already on the canvas moves — the
/// difference between `place_new` and `relayout`, which is the difference between a card arriving
/// and the user asking for a tidy.
#[test]
fn a_new_card_is_placed_without_moving_what_is_already_drawn() {
    let mut f = placed();
    let drawn = [f.writer, f.waiter, f.stopped, f.broken];
    let before: Vec<(f32, f32)> = drawn
        .iter()
        .map(|id| f.graph.at_id(&f.work, *id).unwrap())
        .collect();
    let origins: Vec<(f32, f32)> = [f.plumbing, f.parser, f.elsewhere]
        .iter()
        .map(|id| f.graph.layout.task_origin(*id))
        .collect();

    let arriving = AgentId::generate();
    f.work
        .apply_agent(agent(arriving, f.refit, Some(f.plumbing), "just spawned"));
    f.graph.layout.place_new(&f.work.agents, &f.work.tasks);

    assert_ne!(
        f.graph.layout.offset(arriving),
        (0.0, 0.0),
        "an arriving card gets a place rather than being left on its container's origin"
    );
    let at = f.graph.at_id(&f.work, arriving).unwrap();
    assert!(
        before.iter().all(|was| *was != at),
        "and not the place of a card that is already drawn"
    );

    for (id, was) in drawn.iter().zip(&before) {
        assert_eq!(f.graph.at_id(&f.work, *id).as_ref(), Some(was));
    }
    for (id, was) in [f.plumbing, f.parser, f.elsewhere].iter().zip(&origins) {
        assert_eq!(f.graph.layout.task_origin(*id), *was);
    }
}

#[test]
fn a_container_is_the_box_round_the_cards_that_are_drawn() {
    let mut f = placed();
    let (x, y, w, h) = f
        .graph
        .bounds_of(&f.work, f.plumbing)
        .expect("the first task has visible cards");
    assert_eq!(x, -GROUP_PAD);
    assert_eq!(y, -GROUP_PAD - GROUP_LABEL);
    assert_eq!(w, 400.0 + CARD_WIDTH + GROUP_PAD * 2.0);
    assert_eq!(h, CARD_HEIGHT + GROUP_PAD * 2.0 + GROUP_LABEL);

    // A container with no cards in it has no box at all: nobody *serves* `elsewhere`, though an
    // agent owns a step in it. Being in another session is no longer a reason — the canvas draws
    // every session — so what else leaves a task boxless is a filter that hid all of its cards.
    assert!(f.graph.bounds_of(&f.work, f.elsewhere).is_none());
    // `parser` is served by an ended card and a failed one, so turning both buckets off empties
    // it — with two pills still lit, so the row is genuinely filtering rather than cleared.
    f.graph.toggle_bucket(Bucket::Ended);
    f.graph.toggle_bucket(Bucket::Error);
    assert!(f.graph.bounds_of(&f.work, f.parser).is_none());
}

/// Sessions stack, so a canvas drawing all of them is not a pile.
///
/// Each session used to be laid out from the same origin, which nothing noticed while the graph
/// drew one session at a time. It draws every session now, so the second one has to start below the
/// first — and an agent with no task, which is where a project manager coordinating everything
/// ends up, has to clear the containers of the session before it too.
#[test]
fn two_sessions_are_laid_out_clear_of_each_other() {
    let alpha = SessionId::generate();
    let beta = SessionId::generate();
    let one = AgentId::generate();
    let two = AgentId::generate();
    let boss = AgentId::generate();
    let first = TaskId::generate();
    let second = TaskId::generate();

    let mut work = WorkProjection::empty();
    work.replace_all(
        vec![session(alpha, "alpha"), session(beta, "beta")],
        vec![
            agent(one, alpha, Some(first), "one"),
            agent(two, beta, Some(second), "two"),
            // No task: the shape a project manager has, drawn above its session's containers.
            agent(boss, beta, None, "boss"),
        ],
        vec![
            task(first, alpha, "first", &[Some(one)]),
            task(second, beta, "second", &[Some(two)]),
        ],
    );
    let mut view = GraphView::default();
    view.relayout(&work);

    let (_, ay, _, ah) = view.bounds_of(&work, first).unwrap();
    let (_, by, _, _) = view.bounds_of(&work, second).unwrap();
    assert!(
        by >= ay + ah,
        "beta's container starts below alpha's, not on top of it: {by} vs {}",
        ay + ah
    );

    // And the loose card sits above its own session's container rather than inside the one above.
    let at = view.at_id(&work, boss).unwrap();
    assert!(at.1 >= ay + ah, "the unowned card clears alpha too");
    assert!(at.1 + CARD_HEIGHT <= by, "and stays above beta's container");
}

/// An agent coordinating the whole project parents each session's master, and the graph draws that.
///
/// Two things have to hold for the tree to read as a tree. A master answering to a parent **outside**
/// its container stays on the container's top row — otherwise every session's root would be pushed
/// down a level by a parent that is not in the box. And the row of agents with no task is stacked
/// rather than laid side by side, because that row holds a spawn tree of its own: an orchestrator
/// drawn beside its own child would send the connector sideways.
#[test]
fn an_orchestrator_parents_each_sessions_master_without_sinking_it() {
    let alpha = SessionId::generate();
    let beta = SessionId::generate();
    let boss = AgentId::generate();
    let scribe = AgentId::generate();
    let master = AgentId::generate();
    let worker = AgentId::generate();
    let job = TaskId::generate();

    // The orchestrator answers to nobody; the session's master answers to it; the worker answers to
    // the master. `scribe` shares the orchestrator's taskless row.
    let mut scribe_card = agent(scribe, alpha, None, "scribe");
    scribe_card.parent = Some(boss);
    let mut master_card = agent(master, beta, Some(job), "master");
    master_card.parent = Some(boss);
    let mut worker_card = agent(worker, beta, Some(job), "worker");
    worker_card.parent = Some(master);

    let mut work = WorkProjection::empty();
    work.replace_all(
        vec![session(alpha, "alpha"), session(beta, "beta")],
        vec![
            agent(boss, alpha, None, "boss"),
            scribe_card,
            master_card,
            worker_card,
        ],
        vec![task(job, beta, "job", &[Some(master)])],
    );
    let mut view = GraphView::default();
    view.relayout(&work);

    // The master is on its container's top row even though its parent is outside the box, and the
    // worker is the level below it.
    let origin = view.layout.task_origin(job);
    let m = view.at_id(&work, master).unwrap();
    let w = view.at_id(&work, worker).unwrap();
    assert_eq!(m.1, origin.1, "the master sits on the container's top row");
    assert_eq!(
        w.1 - m.1,
        CARD_HEIGHT + CARD_GAP_Y,
        "the worker is under it"
    );

    // And the taskless row stacks: the child is under the orchestrator, not beside it.
    let b = view.at_id(&work, boss).unwrap();
    let sc = view.at_id(&work, scribe).unwrap();
    assert_eq!(
        sc.1 - b.1,
        CARD_HEIGHT + CARD_GAP_Y,
        "stacked, not in a row"
    );
    assert_eq!(b.0, sc.0, "and in the same column");
}

/// **Position is the interface's own fact, membership is the host's.** A drop answers the card and
/// the container it landed in, for the caller to send as an `AssignAgent`, and changes nothing
/// about the work: the card is still recorded against the task it came from until the host says
/// otherwise. What the drop does write is the offset, against the container it landed in — so the
/// card is where it was let go of the moment the answer comes back.
#[test]
fn a_card_dropped_in_another_container_answers_the_pair_and_moves_no_membership() {
    let mut f = placed();
    edit_agent(&mut f.work, f.stopped, |a| a.parent = Some(f.writer));

    f.graph.start_carry(Held::Agent(f.stopped), (10.0, 10.0));
    // Into the middle of the first task's container.
    f.graph
        .carry_to(&f.work, (200.0, 0.0), Some((0.0, 0.0)), Instant::now());
    assert_eq!(f.graph.carry.unwrap().over, Some(f.plumbing));

    assert_eq!(
        f.graph.end_carry(&f.work),
        Some((f.stopped, f.plumbing)),
        "the drop is a request, not a result"
    );
    assert!(f.graph.carry.is_none());

    let dropped = f.work.agent(f.stopped).unwrap();
    assert_eq!(
        dropped.task,
        Some(f.parser),
        "the card is still recorded where it was until the host answers"
    );
    assert_eq!(
        dropped.parent,
        Some(f.writer),
        "and still answers to whoever spawned it"
    );

    // The host confirms, and the offset written at the drop puts the card back under the pointer.
    edit_agent(&mut f.work, f.stopped, |a| a.task = Some(f.plumbing));
    assert_eq!(
        f.graph.at_id(&f.work, f.stopped),
        Some((200.0, 0.0)),
        "re-anchoring to the new container leaves it where it was let go of"
    );
}

#[test]
fn a_card_dropped_on_open_ground_only_moves() {
    let mut f = placed();
    f.graph.start_carry(Held::Agent(f.writer), (0.0, 0.0));
    // Far outside every container, including the one it started in — a carried card is left out
    // of the boxes it is tested against, so it does not sit inside its own.
    f.graph.carry_to(
        &f.work,
        (2_000.0, 2_000.0),
        Some((0.0, 0.0)),
        Instant::now(),
    );
    assert_eq!(f.graph.carry.unwrap().over, None);
    assert_eq!(f.graph.end_carry(&f.work), None);
    assert_eq!(
        f.work.agent(f.writer).unwrap().task,
        Some(f.plumbing),
        "still its own task"
    );
    assert_eq!(f.graph.at_id(&f.work, f.writer), Some((2_000.0, 2_000.0)));
}

#[test]
fn a_carried_container_takes_everything_in_it() {
    let mut f = placed();
    let cards = [f.writer, f.waiter, f.stopped, f.broken];
    let before: Vec<(f32, f32)> = cards
        .iter()
        .map(|id| f.graph.at_id(&f.work, *id).unwrap())
        .collect();
    let (x, y, w, h) = f.graph.bounds_of(&f.work, f.plumbing).unwrap();

    f.graph.start_carry(Held::Task(f.plumbing), (0.0, 0.0));
    f.graph.carry_to(
        &f.work,
        (x + 300.0, y + 120.0),
        Some((0.0, 0.0)),
        Instant::now(),
    );

    // The box went exactly where it was put, and kept its size.
    assert_eq!(
        f.graph.bounds_of(&f.work, f.plumbing),
        Some((x + 300.0, y + 120.0, w, h))
    );

    // Its two cards moved with it, by the same amount, and nothing else moved at all.
    for id in [f.writer, f.waiter] {
        let ix = cards.iter().position(|c| *c == id).unwrap();
        let at = f.graph.at_id(&f.work, id).unwrap();
        let was = before[ix];
        assert_eq!((at.0 - was.0, at.1 - was.1), (300.0, 120.0));
    }
    for id in [f.stopped, f.broken] {
        let ix = cards.iter().position(|c| *c == id).unwrap();
        assert_eq!(f.graph.at_id(&f.work, id), Some(before[ix]));
    }

    // A container is not filed inside another container, so putting it down asks for nothing.
    assert_eq!(f.graph.carry.unwrap().over, None);
    assert_eq!(f.graph.end_carry(&f.work), None);
    assert_eq!(f.work.agent(f.writer).unwrap().task, Some(f.plumbing));
}

#[test]
fn carrying_lays_sand_that_runs_out() {
    let mut f = seeded();
    let start = Instant::now();

    f.graph.start_carry(Held::Agent(f.writer), (0.0, 0.0));
    for step in 0..5 {
        let d = step as f32 * 10.0;
        f.graph.carry_to(&f.work, (d, d), Some((d, d)), start);
    }
    assert_eq!(f.graph.sand.len(), 5);
    assert!(
        f.graph.settle_sand(start),
        "fresh grains are still owed frames"
    );

    let later = start + GRAIN_LIFE + Duration::from_millis(1);
    assert!(!f.graph.settle_sand(later));
    assert!(f.graph.sand.is_empty());

    // Reduced motion asks for no trail, and the card still moves.
    f.graph.start_carry(Held::Agent(f.writer), (0.0, 0.0));
    f.graph.carry_to(&f.work, (90.0, 90.0), None, start);
    assert!(f.graph.sand.is_empty());
    assert_eq!(f.graph.at_id(&f.work, f.writer), Some((90.0, 90.0)));
}

#[test]
fn the_sand_is_capped() {
    let mut f = seeded();
    let now = Instant::now();
    f.graph.start_carry(Held::Agent(f.writer), (0.0, 0.0));
    for step in 0..1_000 {
        let d = step as f32;
        f.graph.carry_to(&f.work, (d, d), Some((d, d)), now);
    }
    assert!(f.graph.sand.len() <= GRAIN_CEILING);
}

#[test]
fn the_tasks_listed_follow_the_selection() {
    let mut f = seeded();

    f.graph.selection = Some(Selection::Session(f.refit));
    let ids: Vec<TaskId> = f.graph.listed_tasks(&f.work).iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        vec![f.plumbing, f.parser],
        "a session lists every task in it"
    );

    f.graph.selection = Some(Selection::Agent(f.stopped));
    let ids: Vec<TaskId> = f.graph.listed_tasks(&f.work).iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        vec![f.parser],
        "an agent lists the tasks it has a step in"
    );
}

#[test]
fn zoom_is_clamped_at_both_ends() {
    let mut f = seeded();
    for _ in 0..50 {
        f.graph.zoom_by(-1.0);
    }
    assert_eq!(f.graph.zoom, ZOOM_MIN);
    for _ in 0..50 {
        f.graph.zoom_by(1.0);
    }
    assert_eq!(f.graph.zoom, ZOOM_MAX);
}
