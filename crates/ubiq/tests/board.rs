//! The tasks board's logic, without a frame.
//!
//! Everything the board decides — which cards a column draws, what the filter field and the
//! session pills leave on screen, where a dragged card lands, what a card's meter and pulse read,
//! which columns and cards are shut — is arithmetic over the same task vector the graph reads, so
//! it is tested the way the graph is: on the state alone, seeded the way the fixture seeds it.
//!
//! A column is asserted by the ids in it and never by where it is drawn. The board has no
//! geometry of its own: a card's place is its status, and its status is the task's.

use ubiq::state::agents::{
    Activity, Agent, AgentsState, Bucket, Priority, Session, SessionId, Shape, Status, Step,
    StepState, Task, TaskId,
};
use ubiq::state::board::BoardState;

fn session(id: SessionId, name: &str) -> Session {
    Session {
        id,
        name: name.to_string(),
        branch: "main".to_string(),
        worktree: false,
    }
}

fn agent(id: u32, session: SessionId, task: Option<TaskId>, activity: Activity) -> Agent {
    Agent {
        id,
        session,
        task,
        parent: None,
        name: format!("agent-{id}"),
        role: "Implementer".to_string(),
        activity,
        note: "doing a thing".to_string(),
        branch: "main".to_string(),
        tokens: 10_000.0,
        harness: "Claude Code".to_string(),
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
) -> Task {
    Task {
        id,
        session,
        status,
        priority: Priority::Normal,
        shape: Shape::Direct,
        title: title.to_string(),
        steps: steps
            .iter()
            .enumerate()
            .map(|(ix, state)| Step {
                title: format!("step-{ix}"),
                state: *state,
                owner: None,
            })
            .collect(),
    }
}

/// Two sessions and four tasks: two in one session, one in the other, and one nobody has started.
/// The titles say something, because the filter matches what the card prints.
fn seeded() -> AgentsState {
    AgentsState::new(
        vec![session(1, "cold-start"), session(2, "resize")],
        vec![
            agent(1, 1, Some(1), Activity::Writing),
            agent(2, 1, Some(1), Activity::NeedsYou),
            agent(3, 1, Some(2), Activity::Ended),
            agent(4, 2, Some(3), Activity::Writing),
        ],
        vec![
            task(
                1,
                "warm the cache",
                Some(1),
                Status::InProgress,
                &[StepState::Done, StepState::Idle],
            ),
            task(
                2,
                "drop the parser",
                Some(1),
                Status::Backlog,
                &[StepState::Done],
            ),
            task(
                3,
                "shrink the pane",
                Some(2),
                Status::InProgress,
                &[StepState::Failed],
            ),
            task(4, "nobody has started this", None, Status::Backlog, &[]),
        ],
    )
}

fn ids(tasks: &[&Task]) -> Vec<TaskId> {
    tasks.iter().map(|t| t.id).collect()
}

#[test]
fn a_column_draws_only_the_tasks_that_are_in_it() {
    let state = seeded();
    let board = BoardState::default();

    assert_eq!(ids(&board.column(&state, Status::Backlog)), vec![2, 4]);
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![1, 3]);
    assert!(board.column(&state, Status::Done).is_empty());
}

#[test]
fn the_session_pill_leaves_out_every_other_session_and_the_unstarted() {
    let state = seeded();
    let mut board = BoardState {
        session: Some(1),
        ..Default::default()
    };
    assert_eq!(
        ids(&board.column(&state, Status::Backlog)),
        vec![2],
        "the task nobody has started belongs to no session"
    );
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![1]);

    board.session = Some(2);
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![3]);
    assert!(board.column(&state, Status::Backlog).is_empty());

    // No pill is every session, including the one that has none.
    board.session = None;
    assert_eq!(ids(&board.column(&state, Status::Backlog)), vec![2, 4]);
}

#[test]
fn the_filter_matches_the_title_and_the_session_name_in_either_case() {
    let state = seeded();
    let mut board = BoardState {
        filter: "CACHE".to_string(),
        ..Default::default()
    };
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![1]);

    // Nothing in task 3 says "resize" except the session it names.
    board.filter = "  resize ".to_string();
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![3]);
    assert!(board.column(&state, Status::Backlog).is_empty());

    board.filter = "Shrink".to_string();
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![3]);

    board.filter = "nothing says this".to_string();
    assert!(board.column(&state, Status::Backlog).is_empty());

    // The two filters are read together, not in turn.
    board.filter = "the".to_string();
    board.session = Some(1);
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![1]);
}

#[test]
fn a_new_task_lands_in_the_backlog_with_an_id_of_its_own() {
    let mut state = seeded();
    let board = BoardState::default();

    let id = state.add_task("look at the resize path".to_string(), None);
    assert_eq!(
        state.tasks.iter().filter(|t| t.id == id).count(),
        1,
        "nothing else is using the id"
    );

    let fresh = state.task(id).expect("the new task is there");
    assert_eq!(fresh.status, Status::Backlog);
    assert_eq!(
        fresh.session, None,
        "a task nobody has started stays that way"
    );
    assert!(fresh.steps.is_empty());
    assert!(ids(&board.column(&state, Status::Backlog)).contains(&id));

    // The session it was handed is the session it keeps.
    let second = state.add_task("and another".to_string(), Some(2));
    assert_ne!(second, id);
    assert_eq!(state.task(second).unwrap().session, Some(2));
}

#[test]
fn moving_a_task_changes_its_column_once() {
    let mut state = seeded();
    let board = BoardState::default();

    assert!(state.move_task(1, Status::Done));
    assert_eq!(state.task(1).unwrap().status, Status::Done);
    assert_eq!(ids(&board.column(&state, Status::Done)), vec![1]);
    assert_eq!(ids(&board.column(&state, Status::InProgress)), vec![3]);

    assert!(
        !state.move_task(1, Status::Done),
        "a drop where the card already was costs no redraw"
    );
    assert!(!state.move_task(404, Status::Ready), "no such task");
}

#[test]
fn ticking_a_step_moves_the_meter_and_unticking_lands_on_idle() {
    let mut state = seeded();
    assert_eq!(state.task(1).unwrap().done(), 1);
    assert_eq!(state.task(1).unwrap().fraction(), 0.5);

    assert!(state.toggle_step(1, 1));
    assert_eq!(state.task(1).unwrap().done(), 2);
    assert_eq!(state.task(1).unwrap().fraction(), 1.0);

    assert!(state.toggle_step(1, 0));
    assert_eq!(
        state.task(1).unwrap().steps[0].state,
        StepState::Idle,
        "unticking cannot know what its owner would go back to doing"
    );
    assert_eq!(state.task(1).unwrap().fraction(), 0.5);

    assert!(!state.toggle_step(1, 9), "no such step");
    assert!(!state.toggle_step(404, 0), "no such task");

    assert_eq!(
        state.task(4).unwrap().fraction(),
        0.0,
        "a task with no steps has nothing to be a fraction of"
    );
}

#[test]
fn a_failed_step_blocks_the_task_and_the_card_reads_as_an_error() {
    let mut state = seeded();
    let blocked = state.task(3).unwrap().clone();
    assert!(blocked.blocked());
    assert_eq!(
        state.pulse(&blocked),
        Bucket::Error,
        "the step failed even though the agent on it is writing"
    );

    // Nothing else is blocked, and ticking the failed step clears it.
    assert!(!state.task(1).unwrap().blocked());
    assert!(state.toggle_step(3, 0));
    assert!(!state.task(3).unwrap().blocked());
}

#[test]
fn a_pulse_is_the_worst_thing_happening_in_the_task() {
    let mut state = seeded();

    // Waiting beats running: one member is writing, the other needs the user.
    let one = state.task(1).unwrap().clone();
    assert_eq!(state.pulse(&one), Bucket::Waiting);

    // Running beats ended, once nothing is waiting.
    state.agent_mut(2).unwrap().activity = Activity::Writing;
    assert_eq!(state.pulse(&one), Bucket::Running);

    // Everything finished, and a ticked step is not a running one.
    let two = state.task(2).unwrap().clone();
    assert_eq!(state.pulse(&two), Bucket::Ended);

    // A step alone is enough: no member of task 2 is waiting, but a step of it is.
    state.tasks[1].steps[0].state = StepState::NeedsYou;
    let two = state.task(2).unwrap().clone();
    assert_eq!(state.pulse(&two), Bucket::Waiting);
}

#[test]
fn a_task_speaks_through_whoever_is_holding_it() {
    let mut state = seeded();

    let one = state.task(1).unwrap().clone();
    assert_eq!(state.now(&one).map(|a| a.id), Some(1));

    // The first member that has not ended, not simply the first.
    state.agent_mut(1).unwrap().activity = Activity::Ended;
    assert_eq!(state.now(&one).map(|a| a.id), Some(2));

    // Everybody has ended, so the task still answers rather than going silent.
    let two = state.task(2).unwrap().clone();
    assert_eq!(state.now(&two).map(|a| a.id), Some(3));

    // A task with nobody on it has nobody to speak through.
    let four = state.task(4).unwrap().clone();
    assert!(state.now(&four).is_none());
}

#[test]
fn a_coordinated_task_speaks_through_its_coordinator() {
    let mut state = seeded();
    state.tasks[0].shape = Shape::Coordinated;
    state.agents.push(agent(5, 1, Some(1), Activity::Writing));
    state.agent_mut(2).unwrap().parent = Some(1);
    state.agent_mut(5).unwrap().parent = Some(1);
    // The coordinator answers even though it is not the first member still going.
    state.agent_mut(1).unwrap().activity = Activity::Ended;

    let one = state.task(1).unwrap().clone();
    assert_eq!(state.now(&one).map(|a| a.id), Some(1));

    // With nobody spawned by anybody there is no coordinator, so it falls back to who is holding
    // it — which is the same answer every other shape gives.
    state.agent_mut(2).unwrap().parent = None;
    state.agent_mut(5).unwrap().parent = None;
    assert_eq!(state.now(&one).map(|a| a.id), Some(2));
}

#[test]
fn dragging_a_card_answers_the_column_it_landed_in() {
    let mut board = BoardState::default();

    assert!(
        !board.carry_over(Status::Ready),
        "nothing is being carried yet"
    );

    board.start_carry(2);
    assert!(board.carry_over(Status::Ready));
    assert!(
        !board.carry_over(Status::Ready),
        "a drag across one column does not ask for a frame per pixel"
    );
    assert!(board.carry_over(Status::Done));
    assert_eq!(board.end_carry(), Some((2, Status::Done)));
    assert!(board.carry.is_none());

    // Let go over no column at all, and nothing moves.
    board.start_carry(2);
    assert_eq!(board.end_carry(), None);
    assert!(board.carry.is_none());

    assert_eq!(board.end_carry(), None, "nothing was being carried");
}

#[test]
fn shutting_a_column_and_folding_a_card_undo_themselves() {
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

    assert!(!board.is_folded(3));
    board.toggle_fold(3);
    assert!(board.is_folded(3));
    assert!(!board.is_folded(1));
    board.toggle_fold(3);
    assert!(!board.is_folded(3));
    assert!(board.folded.is_empty());
}

#[test]
fn the_status_bar_counts_only_the_cards_the_filters_leave_on_screen() {
    let state = seeded();
    let mut board = BoardState::default();

    assert_eq!(
        board.counts(&state),
        vec![
            (Status::Backlog, 2),
            (Status::Ready, 0),
            (Status::InProgress, 2),
            (Status::InReview, 0),
            (Status::Done, 0),
        ]
    );
    assert_eq!(board.steps(&state), (2, 4));
    assert_eq!(board.blocked(&state), 1);

    // A shut column still counts: it is a strip, not an absence.
    board.toggle_column(Status::Backlog);
    assert_eq!(board.counts(&state)[0], (Status::Backlog, 2));

    board.session = Some(1);
    assert_eq!(
        board.counts(&state),
        vec![
            (Status::Backlog, 1),
            (Status::Ready, 0),
            (Status::InProgress, 1),
            (Status::InReview, 0),
            (Status::Done, 0),
        ]
    );
    assert_eq!(board.steps(&state), (2, 3));
    assert_eq!(
        board.blocked(&state),
        0,
        "the blocked card is in the other session"
    );

    board.session = None;
    board.filter = "cache".to_string();
    assert_eq!(board.steps(&state), (1, 2));
    assert_eq!(board.blocked(&state), 0);
}

#[test]
fn picking_a_card_opens_the_panel_and_a_shut_panel_reports_nothing() {
    let state = seeded();
    let mut board = BoardState::default();

    assert!(board.open_task(&state).is_none(), "nothing is selected yet");

    board.show_detail = false;
    board.select(3);
    assert_eq!(board.selected, Some(3));
    assert!(
        board.show_detail,
        "a selection nothing reports on is not a selection"
    );
    assert_eq!(board.open_task(&state).map(|t| t.id), Some(3));

    board.show_detail = false;
    assert!(board.open_task(&state).is_none());

    board.show_detail = true;
    board.selected = Some(404);
    assert!(board.open_task(&state).is_none(), "no such task");
}
