//! The tasks board's logic, without a frame.
//!
//! Everything the board decides — which cards a column draws, what the filter field and the
//! session pills leave on screen, where a dragged card lands, what a card's meter and pulse read,
//! which columns and cards are shut — is arithmetic over the same projection the graph reads, so
//! it is tested the way the graph is: on the state alone, seeded the way the host would have
//! filled it.
//!
//! The records are the host's and arrive through `WorkProjection`; `BoardState` is the view over
//! them and takes the projection as the first argument of every reader. What a drop and a `New
//! task` *ask for* is asserted here; what they *do* to a record is asserted in
//! `crates/ubiq-host/tests/work.rs`, because on this side it would be asserting a mock. Ids are
//! minted rather than written down, and the fixture binds each one to a name.
//!
//! A column is asserted by the ids in it and never by where it is drawn. The board has no
//! geometry of its own: a card's place is its status, and its status is the task's.

use ubiq::state::board::BoardState;
use ubiq::state::work::{WorkProjection, fraction};
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

fn agent(
    id: AgentId,
    session: SessionId,
    task: Option<TaskId>,
    name: &str,
    activity: Activity,
) -> WorkAgent {
    WorkAgent {
        id,
        session,
        task,
        parent: None,
        name: name.to_string(),
        role: "Implementer".to_string(),
        activity,
        note: "doing a thing".to_string(),
        branch: "main".to_string(),
        tokens: 10_000.0,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: "Opus 4.6".to_string(),
        context_pct: 5,
        thread: Vec::new(),
    }
}

fn task(
    id: TaskId,
    title: &str,
    session: Option<SessionId>,
    status: Status,
    steps: &[StepState],
) -> TaskRecord {
    TaskRecord {
        id,
        session,
        status,
        priority: Priority::Normal,
        shape: Shape::Direct,
        title: title.to_string(),
        description: String::new(),
        steps: steps
            .iter()
            .enumerate()
            .map(|(ix, state)| {
                let mut step = Step::new(format!("step-{ix}"));
                step.state = *state;
                step
            })
            .collect(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// A record the host has sent again, changed. Replacing on id is the whole of how anything in the
/// work changes — nothing on this side edits a record in place.
fn edit_task(work: &mut WorkProjection, id: TaskId, edit: impl FnOnce(&mut TaskRecord)) {
    let mut task = work.task(id).expect("the fixture has that task").clone();
    edit(&mut task);
    work.apply_task(task);
}

fn edit_agent(work: &mut WorkProjection, id: AgentId, edit: impl FnOnce(&mut WorkAgent)) {
    let mut agent = work.agent(id).expect("the fixture has that agent").clone();
    edit(&mut agent);
    work.apply_agent(agent);
}

fn pulse(work: &WorkProjection, id: TaskId) -> Bucket {
    work.pulse(work.task(id).expect("the fixture has that task"))
}

fn now(work: &WorkProjection, id: TaskId) -> Option<AgentId> {
    work.now(work.task(id).expect("the fixture has that task"))
        .map(|agent| agent.id)
}

/// `app.rs`'s `TaskChanged` arm, without the frame: the mark comes off whatever column the answer
/// reports, and then the record is applied.
fn task_changed(board: &mut BoardState, work: &mut WorkProjection, task: TaskRecord) {
    if board.is_moving(task.id) {
        board.moving = None;
    }
    work.apply_task(task);
}

/// `app.rs`'s `TaskCreated` arm, without the frame.
fn task_created(board: &mut BoardState, work: &mut WorkProjection, task: TaskRecord) {
    let id = task.id;
    work.apply_task(task);
    if board.awaiting_new {
        board.awaiting_new = false;
        board.select(id);
    }
}

fn ids(tasks: &[&TaskRecord]) -> Vec<TaskId> {
    tasks.iter().map(|t| t.id).collect()
}

/// One project's work, with a name for every id in it.
struct Fixture {
    work: WorkProjection,
    cold: SessionId,
    resize: SessionId,
    writer: AgentId,
    waiter: AgentId,
    stopped: AgentId,
    shrinker: AgentId,
    cache: TaskId,
    parser: TaskId,
    pane: TaskId,
    unstarted: TaskId,
}

/// Two sessions and four tasks: two in one session, one in the other, and one nobody has started.
/// The titles say something, because the filter matches what the card prints.
fn seeded() -> Fixture {
    let cold = SessionId::generate();
    let resize = SessionId::generate();
    let writer = AgentId::generate();
    let waiter = AgentId::generate();
    let stopped = AgentId::generate();
    let shrinker = AgentId::generate();
    let cache = TaskId::generate();
    let parser = TaskId::generate();
    let pane = TaskId::generate();
    let unstarted = TaskId::generate();

    let mut work = WorkProjection::empty();
    work.replace_all(
        vec![session(cold, "cold-start"), session(resize, "resize")],
        vec![
            agent(writer, cold, Some(cache), "writer", Activity::Writing),
            agent(waiter, cold, Some(cache), "waiter", Activity::NeedsYou),
            agent(stopped, cold, Some(parser), "stopped", Activity::Ended),
            agent(shrinker, resize, Some(pane), "shrinker", Activity::Writing),
        ],
        vec![
            task(
                cache,
                "warm the cache",
                Some(cold),
                Status::InProgress,
                &[StepState::Done, StepState::Idle],
            ),
            task(
                parser,
                "drop the parser",
                Some(cold),
                Status::Backlog,
                &[StepState::Done],
            ),
            task(
                pane,
                "shrink the pane",
                Some(resize),
                Status::InProgress,
                &[StepState::Failed],
            ),
            task(
                unstarted,
                "nobody has started this",
                None,
                Status::Backlog,
                &[],
            ),
        ],
    );

    Fixture {
        work,
        cold,
        resize,
        writer,
        waiter,
        stopped,
        shrinker,
        cache,
        parser,
        pane,
        unstarted,
    }
}

#[test]
fn a_column_draws_only_the_tasks_that_are_in_it() {
    let f = seeded();
    let board = BoardState::default();

    assert_eq!(
        ids(&board.column(&f.work, Status::Backlog)),
        vec![f.parser, f.unstarted]
    );
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.cache, f.pane]
    );
    assert!(board.column(&f.work, Status::Done).is_empty());
}

#[test]
fn the_session_pill_leaves_out_every_other_session_and_the_unstarted() {
    let f = seeded();
    let mut board = BoardState {
        session: Some(f.cold),
        ..Default::default()
    };
    assert_eq!(
        ids(&board.column(&f.work, Status::Backlog)),
        vec![f.parser],
        "the task nobody has started belongs to no session"
    );
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.cache]
    );

    board.session = Some(f.resize);
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.pane]
    );
    assert!(board.column(&f.work, Status::Backlog).is_empty());

    // No pill is every session, including the one that has none.
    board.session = None;
    assert_eq!(
        ids(&board.column(&f.work, Status::Backlog)),
        vec![f.parser, f.unstarted]
    );
}

#[test]
fn the_filter_matches_the_title_and_the_session_name_in_either_case() {
    let f = seeded();
    let mut board = BoardState {
        filter: "CACHE".to_string(),
        ..Default::default()
    };
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.cache]
    );

    // Nothing in the third task says "resize" except the session it names.
    board.filter = "  resize ".to_string();
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.pane]
    );
    assert!(board.column(&f.work, Status::Backlog).is_empty());

    board.filter = "Shrink".to_string();
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.pane]
    );

    board.filter = "nothing says this".to_string();
    assert!(board.column(&f.work, Status::Backlog).is_empty());

    // The two filters are read together, not in turn.
    board.filter = "the".to_string();
    board.session = Some(f.cold);
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.cache]
    );
}

/// The filter matches what a card actually prints, and a card prints no description — so a needle
/// that only the notes say is not a hit. Matching it would leave a column showing cards with
/// nothing on them to say why they are there.
#[test]
fn the_filter_does_not_match_a_description() {
    let mut f = seeded();
    edit_task(&mut f.work, f.cache, |task| {
        task.description = "the eviction policy needs a rethink".to_string();
    });
    let board = BoardState {
        filter: "eviction".to_string(),
        ..Default::default()
    };

    assert!(
        !board.matches(&f.work, f.work.task(f.cache).unwrap()),
        "a description is not printed on the card, so it is not filtered on"
    );
    assert!(board.column(&f.work, Status::InProgress).is_empty());
}

/// A projection that appended on a re-send would draw the same card twice, which is the classic
/// duplicate-card bug: every apply replaces on id, so the same record twice is the same projection.
#[test]
fn applying_a_task_twice_leaves_one_card() {
    let mut f = seeded();
    let board = BoardState::default();
    let before = f.work.tasks.len();

    let held = f.work.task(f.cache).unwrap().clone();
    assert!(
        !f.work.apply_task(held.clone()),
        "a record already held is not new"
    );
    assert!(!f.work.apply_task(held.clone()));
    assert_eq!(f.work.tasks.len(), before);
    assert_eq!(
        ids(&board.column(&f.work, Status::InProgress)),
        vec![f.cache, f.pane],
        "and it is still in the one column, once"
    );

    // The later record wins, in the place the first one held.
    let mut moved = held;
    moved.status = Status::Done;
    assert!(!f.work.apply_task(moved));
    assert_eq!(f.work.tasks.len(), before);
    assert_eq!(f.work.task(f.cache).unwrap().status, Status::Done);
    assert_eq!(ids(&board.column(&f.work, Status::Done)), vec![f.cache]);

    // A record nobody has heard of is the new one.
    let fresh = TaskId::generate();
    assert!(f.work.apply_task(task(
        fresh,
        "just arrived",
        Some(f.cold),
        Status::Backlog,
        &[]
    )));
    assert_eq!(f.work.tasks.len(), before + 1);
}

/// A whole list that no longer names a task is a task that has gone, and a panel pointed at one
/// that has gone reports on nothing rather than on the card that took its place.
#[test]
fn a_task_the_next_list_leaves_out_is_gone_and_the_panel_says_nothing() {
    let mut f = seeded();
    let mut board = BoardState::default();
    board.select(f.parser);
    assert_eq!(board.open_task(&f.work).map(|t| t.id), Some(f.parser));

    let kept: Vec<TaskRecord> = f
        .work
        .tasks
        .iter()
        .filter(|t| t.id != f.parser)
        .cloned()
        .collect();
    let sessions = f.work.sessions.clone();
    let agents = f.work.agents.clone();
    f.work.replace_all(sessions, agents, kept);

    assert!(f.work.task(f.parser).is_none());
    assert!(
        board
            .column(&f.work, Status::Backlog)
            .iter()
            .all(|t| t.id != f.parser)
    );
    assert_eq!(board.selected, Some(f.parser), "the selection is untouched");
    assert!(
        board.open_task(&f.work).is_none(),
        "a panel pointed at a task that has gone reports on nothing"
    );
}

/// The card says it is on its way until the host answers, and the mark comes off whatever column
/// the answer reports — the old one included. A refusal that left the card where it was must not
/// leave it saying it is still moving.
#[test]
fn a_mark_for_a_move_comes_off_even_when_the_host_refuses_it() {
    let mut f = seeded();
    let mut board = BoardState::default();

    board.start_carry(f.cache);
    assert!(board.carry_over(Status::Done));
    let (id, status) = board.end_carry().expect("the card landed in a column");
    assert_eq!((id, status), (f.cache, Status::Done));

    board.moving = Some((id, status));
    assert!(board.is_moving(f.cache));
    assert!(!board.is_moving(f.pane), "only the card that was dropped");

    // The answer reports the column it was already in: the host refused the move.
    let refused = f.work.task(f.cache).unwrap().clone();
    assert_eq!(refused.status, Status::InProgress);
    task_changed(&mut board, &mut f.work, refused);

    assert!(!board.is_moving(f.cache), "the mark cannot stick");
    assert_eq!(board.moving, None);
    assert_eq!(f.work.task(f.cache).unwrap().status, Status::InProgress);
}

/// A `New task` cannot select what it asked for, because the id is the host's to mint — so the task
/// that arrives is the one to select. Exactly once: the next task to arrive is somebody else's and
/// must not move a selection the user has since made.
#[test]
fn awaiting_a_new_task_selects_the_one_that_arrives_once() {
    let mut f = seeded();
    let mut board = BoardState {
        awaiting_new: true,
        ..Default::default()
    };

    let first = TaskId::generate();
    task_created(
        &mut board,
        &mut f.work,
        task(first, "look at the resize path", None, Status::Backlog, &[]),
    );
    assert_eq!(board.selected, Some(first));
    assert!(board.show_detail, "and the panel is open on it");
    assert!(!board.awaiting_new, "nothing is being waited for now");

    let second = TaskId::generate();
    task_created(
        &mut board,
        &mut f.work,
        task(second, "and another", Some(f.cold), Status::Backlog, &[]),
    );
    assert_eq!(
        board.selected,
        Some(first),
        "a task nobody asked for does not take the selection"
    );
}

#[test]
fn a_failed_step_blocks_the_task_and_the_card_reads_as_an_error() {
    let mut f = seeded();
    assert!(f.work.task(f.pane).unwrap().blocked());
    assert_eq!(
        f.work.agent(f.shrinker).unwrap().activity,
        Activity::Writing
    );
    assert_eq!(
        pulse(&f.work, f.pane),
        Bucket::Error,
        "the step failed even though the agent on it is writing"
    );

    // Nothing else is blocked, and the step coming back done clears it.
    assert!(!f.work.task(f.cache).unwrap().blocked());
    edit_task(&mut f.work, f.pane, |task| {
        task.steps[0].state = StepState::Done;
    });
    assert!(!f.work.task(f.pane).unwrap().blocked());
}

#[test]
fn a_pulse_is_the_worst_thing_happening_in_the_task() {
    let mut f = seeded();

    // Waiting beats running: one member is writing, the other needs the user.
    assert_eq!(pulse(&f.work, f.cache), Bucket::Waiting);

    // Running beats ended, once nothing is waiting.
    edit_agent(&mut f.work, f.waiter, |agent| {
        agent.activity = Activity::Writing;
    });
    assert_eq!(pulse(&f.work, f.cache), Bucket::Running);

    // Everything finished, and a ticked step is not a running one.
    assert_eq!(pulse(&f.work, f.parser), Bucket::Ended);

    // A step alone is enough: no member of that task is waiting, but a step of it is.
    edit_task(&mut f.work, f.parser, |task| {
        task.steps[0].state = StepState::NeedsYou;
    });
    assert_eq!(pulse(&f.work, f.parser), Bucket::Waiting);
}

#[test]
fn a_task_speaks_through_whoever_is_holding_it() {
    let mut f = seeded();

    assert_eq!(now(&f.work, f.cache), Some(f.writer));

    // The first member that has not ended, not simply the first.
    edit_agent(&mut f.work, f.writer, |agent| {
        agent.activity = Activity::Ended;
    });
    assert_eq!(now(&f.work, f.cache), Some(f.waiter));

    // Everybody has ended, so the task still answers rather than going silent.
    assert_eq!(now(&f.work, f.parser), Some(f.stopped));

    // A task with nobody on it has nobody to speak through.
    assert_eq!(now(&f.work, f.unstarted), None);
}

#[test]
fn a_coordinated_task_speaks_through_its_coordinator() {
    let mut f = seeded();
    let worker = AgentId::generate();
    edit_task(&mut f.work, f.cache, |task| task.shape = Shape::Coordinated);
    f.work.apply_agent(agent(
        worker,
        f.cold,
        Some(f.cache),
        "worker",
        Activity::Writing,
    ));
    edit_agent(&mut f.work, f.waiter, |agent| agent.parent = Some(f.writer));
    edit_agent(&mut f.work, worker, |agent| agent.parent = Some(f.writer));
    // The coordinator answers even though it is not the first member still going.
    edit_agent(&mut f.work, f.writer, |agent| {
        agent.activity = Activity::Ended;
    });

    assert_eq!(now(&f.work, f.cache), Some(f.writer));

    // With nobody spawned by anybody there is no coordinator, so it falls back to who is holding
    // it — which is the same answer every other shape gives.
    edit_agent(&mut f.work, f.waiter, |agent| agent.parent = None);
    edit_agent(&mut f.work, worker, |agent| agent.parent = None);
    assert_eq!(now(&f.work, f.cache), Some(f.waiter));
}

/// A task with no steps answers zero, which is not the same claim as "none of them done" — it has
/// nothing to be a fraction of. A caller that would be saying the second thing checks `steps` first
/// and draws no meter at all.
#[test]
fn the_meter_reads_the_steps_and_a_task_with_none_has_nothing_to_be_a_fraction_of() {
    let mut f = seeded();
    assert_eq!(f.work.task(f.cache).unwrap().done(), 1);
    assert_eq!(fraction(f.work.task(f.cache).unwrap()), 0.5);

    edit_task(&mut f.work, f.cache, |task| {
        task.steps[1].state = StepState::Done;
    });
    assert_eq!(f.work.task(f.cache).unwrap().done(), 2);
    assert_eq!(fraction(f.work.task(f.cache).unwrap()), 1.0);

    assert!(f.work.task(f.unstarted).unwrap().steps.is_empty());
    assert_eq!(fraction(f.work.task(f.unstarted).unwrap()), 0.0);
}

#[test]
fn dragging_a_card_answers_the_column_it_landed_in() {
    let f = seeded();
    let mut board = BoardState::default();

    assert!(
        !board.carry_over(Status::Ready),
        "nothing is being carried yet"
    );

    board.start_carry(f.parser);
    assert!(board.carry_over(Status::Ready));
    assert!(
        !board.carry_over(Status::Ready),
        "a drag across one column does not ask for a frame per pixel"
    );
    assert!(board.carry_over(Status::Done));
    assert_eq!(board.end_carry(), Some((f.parser, Status::Done)));
    assert!(board.carry.is_none());

    // Let go over no column at all, and nothing is asked for.
    board.start_carry(f.parser);
    assert_eq!(board.end_carry(), None);
    assert!(board.carry.is_none());

    assert_eq!(board.end_carry(), None, "nothing was being carried");
}

#[test]
fn shutting_a_column_and_folding_a_card_undo_themselves() {
    let f = seeded();
    let mut board = BoardState::default();

    assert!(!board.is_shut(Status::Done));
    board.toggle_column(Status::Done);
    assert!(board.is_shut(Status::Done));
    assert!(
        !board.is_shut(Status::Backlog),
        "only the one that was shut"
    );
    board.toggle_column(Status::Done);
    assert!(!board.is_shut(Status::Done));
    assert!(board.shut.is_empty());

    assert!(!board.is_folded(f.pane));
    board.toggle_fold(f.pane);
    assert!(board.is_folded(f.pane));
    assert!(!board.is_folded(f.cache));
    board.toggle_fold(f.pane);
    assert!(!board.is_folded(f.pane));
    assert!(board.folded.is_empty());
}

#[test]
fn the_status_bar_counts_only_the_cards_the_filters_leave_on_screen() {
    let f = seeded();
    let mut board = BoardState::default();

    assert_eq!(
        board.counts(&f.work),
        vec![
            (Status::Backlog, 2),
            (Status::Ready, 0),
            (Status::InProgress, 2),
            (Status::InReview, 0),
            (Status::Done, 0),
        ]
    );
    assert_eq!(board.steps(&f.work), (2, 4));
    assert_eq!(board.blocked(&f.work), 1);

    // A shut column still counts: it is a strip, not an absence.
    board.toggle_column(Status::Backlog);
    assert_eq!(board.counts(&f.work)[0], (Status::Backlog, 2));

    board.session = Some(f.cold);
    assert_eq!(
        board.counts(&f.work),
        vec![
            (Status::Backlog, 1),
            (Status::Ready, 0),
            (Status::InProgress, 1),
            (Status::InReview, 0),
            (Status::Done, 0),
        ]
    );
    assert_eq!(board.steps(&f.work), (2, 3));
    assert_eq!(
        board.blocked(&f.work),
        0,
        "the blocked card is in the other session"
    );

    board.session = None;
    board.filter = "cache".to_string();
    assert_eq!(board.steps(&f.work), (1, 2));
    assert_eq!(board.blocked(&f.work), 0);
}

#[test]
fn picking_a_card_opens_the_panel_and_a_shut_panel_reports_nothing() {
    let f = seeded();
    let mut board = BoardState::default();

    assert!(
        board.open_task(&f.work).is_none(),
        "nothing is selected yet"
    );

    board.show_detail = false;
    board.select(f.pane);
    assert_eq!(board.selected, Some(f.pane));
    assert!(
        board.show_detail,
        "a selection nothing reports on is not a selection"
    );
    assert_eq!(board.open_task(&f.work).map(|t| t.id), Some(f.pane));

    board.show_detail = false;
    assert!(board.open_task(&f.work).is_none());

    board.show_detail = true;
    board.selected = Some(TaskId::generate());
    assert!(board.open_task(&f.work).is_none(), "no such task");
}
