//! The agents screen's logic, without a frame.
//!
//! Everything the graph decides — which cards are drawn, where a task's container falls, what a
//! drop means, which tasks a selection lists, how long a grain of sand lasts — is arithmetic over
//! plain data, so it is tested the way the explorer's tree and the window registry are: on the
//! state alone, seeded the way the fixture seeds it.

use std::time::{Duration, Instant};

use ubiq::state::agents::{
    Activity, Agent, AgentsState, Bucket, CARD_HEIGHT, CARD_WIDTH, GRAIN_CEILING, GRAIN_LIFE,
    GROUP_LABEL, GROUP_PAD, Selection, Session, Shape, Speaker, Step, Task, ZOOM_MAX, ZOOM_MIN,
};

fn session(id: u32, name: &str) -> Session {
    Session {
        id,
        name: name.to_string(),
        branch: "main".to_string(),
    }
}

fn agent(id: u32, session: u32, task: Option<u32>, activity: Activity, at: (f32, f32)) -> Agent {
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
        at,
        thread: Vec::new(),
    }
}

fn task(id: u32, session: u32, owners: &[Option<u32>]) -> Task {
    Task {
        id,
        session,
        shape: Shape::Direct,
        title: format!("task-{id}"),
        steps: owners
            .iter()
            .enumerate()
            .map(|(ix, owner)| Step {
                title: format!("step-{ix}"),
                done: ix == 0,
                owner: *owner,
            })
            .collect(),
    }
}

/// One session, two tasks, four agents — one in each bucket.
fn seeded() -> AgentsState {
    AgentsState::new(
        vec![session(1, "one"), session(2, "two")],
        vec![
            agent(1, 1, Some(1), Activity::Writing, (0.0, 0.0)),
            agent(2, 1, Some(1), Activity::NeedsYou, (400.0, 0.0)),
            agent(3, 1, Some(2), Activity::Ended, (0.0, 400.0)),
            agent(4, 1, Some(2), Activity::Failed, (400.0, 400.0)),
            agent(5, 2, None, Activity::Thinking, (0.0, 0.0)),
        ],
        vec![
            task(1, 1, &[Some(1), Some(2)]),
            task(2, 1, &[Some(3)]),
            task(3, 2, &[Some(5)]),
        ],
    )
}

#[test]
fn a_new_screen_selects_the_first_agent() {
    let state = seeded();
    assert_eq!(state.selection, Some(Selection::Agent(1)));
    assert_eq!(state.active_session(), Some(1));
}

#[test]
fn the_graph_shows_only_the_active_session() {
    let mut state = seeded();
    assert!(state.visible(state.agent(1).unwrap()));
    // The agent in the other session is filtered out by the session, not by its activity.
    assert!(!state.visible(state.agent(5).unwrap()));

    state.selection = Some(Selection::Session(2));
    assert!(!state.visible(state.agent(1).unwrap()));
    assert!(state.visible(state.agent(5).unwrap()));
}

#[test]
fn a_filter_hides_its_bucket_and_the_last_one_cannot_be_turned_off() {
    let mut state = seeded();
    state.toggle_bucket(Bucket::Error);
    assert!(!state.showing(Bucket::Error));
    assert!(!state.visible(state.agent(4).unwrap()));

    state.toggle_bucket(Bucket::Error);
    assert!(state.showing(Bucket::Error));

    // Down to one, and then refusing: an empty graph is a filter bug that reads as an empty
    // session.
    for bucket in [Bucket::Running, Bucket::Waiting, Bucket::Ended] {
        state.toggle_bucket(bucket);
    }
    assert_eq!(state.buckets.len(), 1);
    state.toggle_bucket(Bucket::Error);
    assert_eq!(state.buckets.len(), 1, "the last pill stays on");
}

#[test]
fn a_container_is_the_box_round_the_cards_that_are_drawn() {
    let mut state = seeded();
    let (x, y, w, h) = state.bounds_of(1).expect("task 1 has visible cards");
    assert_eq!(x, -GROUP_PAD);
    assert_eq!(y, -GROUP_PAD - GROUP_LABEL);
    assert_eq!(w, 400.0 + CARD_WIDTH + GROUP_PAD * 2.0);
    assert_eq!(h, CARD_HEIGHT + GROUP_PAD * 2.0 + GROUP_LABEL);

    // A container with nothing drawn in it has no box at all — a task in another session, or one
    // whose cards are all filtered out.
    assert!(state.bounds_of(3).is_none());
    state.toggle_bucket(Bucket::Ended);
    state.toggle_bucket(Bucket::Error);
    assert!(state.bounds_of(2).is_none());
}

#[test]
fn a_card_dropped_in_another_container_changes_task() {
    let mut state = seeded();
    state.agent_mut(3).unwrap().parent = Some(1);

    state.start_carry(3, (10.0, 10.0));
    // Into the middle of task 1's container.
    state.carry_to((200.0, 0.0), (0.0, 0.0), Instant::now());
    assert_eq!(state.carry.unwrap().over, Some(1));

    assert_eq!(state.end_carry(), Some(1));
    assert_eq!(state.agent(3).unwrap().task, Some(1));
    assert_eq!(state.agent(3).unwrap().at, (200.0, 0.0));
    assert!(
        state.agent(3).unwrap().parent.is_none(),
        "a card moved to another task stops answering to whoever spawned it there"
    );
    assert!(state.carry.is_none());
}

#[test]
fn a_card_dropped_on_open_ground_only_moves() {
    let mut state = seeded();
    state.start_carry(1, (0.0, 0.0));
    // Far outside every container, including the one it started in — a carried card is left out
    // of the boxes it is tested against, so it does not sit inside its own.
    state.carry_to((2_000.0, 2_000.0), (0.0, 0.0), Instant::now());
    assert_eq!(state.carry.unwrap().over, None);
    assert_eq!(state.end_carry(), None);
    assert_eq!(state.agent(1).unwrap().task, Some(1), "still its own task");
    assert_eq!(state.agent(1).unwrap().at, (2_000.0, 2_000.0));
}

#[test]
fn carrying_lays_sand_that_runs_out() {
    let mut state = seeded();
    let start = Instant::now();

    state.start_carry(1, (0.0, 0.0));
    for step in 0..5 {
        let d = step as f32 * 10.0;
        state.carry_to((d, d), (d, d), start);
    }
    assert_eq!(state.sand.len(), 5);
    assert!(
        state.settle_sand(start),
        "fresh grains are still owed frames"
    );

    let later = start + GRAIN_LIFE + Duration::from_millis(1);
    assert!(!state.settle_sand(later));
    assert!(state.sand.is_empty());
}

#[test]
fn the_sand_is_capped() {
    let mut state = seeded();
    let now = Instant::now();
    state.start_carry(1, (0.0, 0.0));
    for step in 0..1_000 {
        let d = step as f32;
        state.carry_to((d, d), (d, d), now);
    }
    assert!(state.sand.len() <= GRAIN_CEILING);
}

#[test]
fn the_tasks_listed_follow_the_selection() {
    let mut state = seeded();

    state.selection = Some(Selection::Session(1));
    let ids: Vec<u32> = state.listed_tasks().iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![1, 2], "a session lists every task in it");

    state.selection = Some(Selection::Agent(3));
    let ids: Vec<u32> = state.listed_tasks().iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![2], "an agent lists the tasks it has a step in");
}

#[test]
fn zoom_is_clamped_at_both_ends() {
    let mut state = seeded();
    for _ in 0..50 {
        state.zoom_by(-1.0);
    }
    assert_eq!(state.zoom, ZOOM_MIN);
    for _ in 0..50 {
        state.zoom_by(1.0);
    }
    assert_eq!(state.zoom, ZOOM_MAX);
}

#[test]
fn what_the_composer_sends_lands_in_the_selected_agents_thread() {
    let mut state = seeded();
    state.selection = Some(Selection::Agent(2));

    state.draft = "   ".to_string();
    assert!(!state.send(), "whitespace is not a message");

    state.draft = "  look at the resize path  ".to_string();
    assert!(state.send());
    assert!(state.draft.is_empty());

    let thread = &state.agent(2).unwrap().thread;
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0].from, Speaker::You);
    assert_eq!(thread[0].text, "look at the resize path");

    // A session has no thread to send to.
    state.selection = Some(Selection::Session(1));
    state.draft = "anything".to_string();
    assert!(!state.send());
}
