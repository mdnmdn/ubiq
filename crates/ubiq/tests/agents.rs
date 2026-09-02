//! The agents screen's logic, without a frame.
//!
//! Everything the columns decide — how the screen lays itself out from the work, what a close
//! means, what a tab dragged onto a column means, which composer a column owns, what happens when
//! an agent the host was reporting goes away — is arithmetic over plain data, so it is tested the
//! way the orchestration graph's arrangement is: on the state alone, seeded the way the host would
//! have filled it.
//!
//! The records are `ubiq_proto::work`'s own and arrive through `WorkProjection`, which is what the
//! window does with a `WorkList`; the view over them is an `AgentsView`, which holds no records and
//! takes the projection as the first argument of every reader that needs one.
//!
//! **The one claim worth stating twice**: closing a tab benches an agent and never ends it. Nothing
//! in this file sends anything, because the arrangement is the interface's own fact.

use ubiq::state::agents::{AgentsView, COLUMNS_MAX};
use ubiq::state::work::WorkProjection;
use ubiq_proto::ids::SessionId;
use ubiq_proto::work::{Activity, AgentId, Bucket, WorkAgent, WorkSession};

fn session(id: SessionId, name: &str, worktree: bool) -> WorkSession {
    WorkSession {
        id,
        name: name.to_string(),
        branch: name.to_string(),
        worktree,
    }
}

fn agent(id: AgentId, session: SessionId, name: &str, activity: Activity) -> WorkAgent {
    WorkAgent {
        id,
        session,
        task: None,
        parent: None,
        name: name.to_string(),
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

/// One project's work and the screen's view of it, with a name for every id in it.
struct Fixture {
    work: WorkProjection,
    view: AgentsView,
    refit: SessionId,
    store: SessionId,
    fixer: AgentId,
    spec: AgentId,
    builder: AgentId,
}

/// Two sessions: one with a single agent, one with two — which is what makes a grouped column and
/// a lone one both reachable without editing the fixture.
fn seeded() -> Fixture {
    let refit = SessionId::generate();
    let store = SessionId::generate();
    let fixer = AgentId::generate();
    let spec = AgentId::generate();
    let builder = AgentId::generate();

    let mut work = WorkProjection::empty();
    work.replace_all(
        vec![
            session(refit, "fix/terminal-refit", true),
            session(store, "feat/session-store", true),
        ],
        vec![
            agent(fixer, refit, "Fixer", Activity::Tools),
            agent(spec, store, "Spec", Activity::Ended),
            agent(builder, store, "Builder", Activity::Writing),
        ],
        Vec::new(),
    );

    let mut view = AgentsView::default();
    view.arrange(&work);
    Fixture {
        work,
        view,
        refit,
        store,
        fixer,
        spec,
        builder,
    }
}

/// The screen lays itself out from the work: one column per session that has an agent in it,
/// holding every agent in that session, in the order the host listed them.
///
/// The bench therefore starts **empty**. That is the whole point of the default: the user has not
/// taken anything off screen, so nothing is off screen, and the bench is a record of their own
/// closes rather than a filter nobody chose.
#[test]
fn the_screen_opens_one_column_per_session_and_an_empty_bench() {
    let f = seeded();
    assert_eq!(f.view.columns.len(), 2);
    assert_eq!(f.view.columns[0].tabs, vec![f.fixer]);
    assert_eq!(f.view.columns[1].tabs, vec![f.spec, f.builder]);
    assert_eq!(f.view.columns[1].active, 0);

    assert_eq!(f.view.on_the_field(), 3);
    assert_eq!(
        f.view.grouped(),
        1,
        "the session with two agents is grouped"
    );
    assert!(f.view.benched(&f.work).is_empty());
    assert!(f.view.arranged);
}

/// A session with no agent in it gets no column: the screen is about agents, and a column with
/// nothing to say is a column the user has to close.
#[test]
fn a_session_nobody_is_working_in_gets_no_column() {
    let mut f = seeded();
    let idle = SessionId::generate();
    f.work.sessions.push(session(idle, "main", false));
    f.view.arrange(&f.work);
    assert_eq!(f.view.columns.len(), 2);
}

/// **Closing a tab benches the agent; it does not end it.** The record is untouched — this screen
/// sends nothing and kills nothing — and the sidebar still lists it, because the bench is computed
/// from the work rather than written down.
#[test]
fn closing_a_tab_benches_the_agent_and_leaves_the_record_alone() {
    let mut f = seeded();
    f.view.bench(f.builder);

    assert!(!f.view.on_screen(f.builder));
    assert_eq!(f.view.columns[1].tabs, vec![f.spec]);
    assert!(!f.view.columns[1].grouped());
    let benched: Vec<AgentId> = f.view.benched(&f.work).iter().map(|a| a.id).collect();
    assert_eq!(benched, vec![f.builder]);

    // The record says exactly what it said before: the agent is still running, still in its
    // session, still doing what it was doing.
    let still = f.work.agent(f.builder).expect("the host still reports it");
    assert_eq!(still.activity, Activity::Writing);
    assert_eq!(still.session, f.store);

    // And the last tab of a column takes the column with it rather than leaving an empty one.
    f.view.bench(f.fixer);
    assert_eq!(f.view.columns.len(), 1);
    assert_eq!(f.view.focus, 0, "focus never names a column that has gone");
}

/// Closing the tab that is in front leaves a column reporting on one of the others rather than on
/// nothing.
#[test]
fn closing_the_front_tab_leaves_the_column_on_a_live_one() {
    let mut f = seeded();
    f.view.select_tab(1, 1);
    assert_eq!(f.view.active_agent(1), Some(f.builder));

    f.view.bench(f.builder);
    assert_eq!(f.view.active_agent(1), Some(f.spec));
}

/// A tab dropped on another column joins it, and is in exactly one column afterwards.
///
/// The duplicate is the failure worth naming: an agent drawn in two columns would have two
/// composers addressed at it, and the second would be a conversation nobody could see the whole of.
#[test]
fn a_tab_dropped_on_another_column_groups_and_leaves_no_duplicate() {
    let mut f = seeded();
    f.view.open_in(1, f.fixer);

    assert_eq!(
        f.view.columns.len(),
        1,
        "the column it came from held nothing else and went with it"
    );
    assert_eq!(f.view.columns[0].tabs, vec![f.spec, f.builder, f.fixer]);
    assert_eq!(
        f.view.active_agent(0),
        Some(f.fixer),
        "and comes to the front"
    );
    assert_eq!(f.view.on_the_field(), 3, "three agents, still three");
    assert_eq!(f.view.grouped(), 1);
}

/// A tab dropped past the last column gets one of its own, and the group it left keeps the rest.
#[test]
fn a_grouped_tab_split_off_gets_a_column_and_the_group_keeps_the_rest() {
    let mut f = seeded();
    let at = f.view.columns.len();
    assert!(f.view.split_off(f.builder, at));

    assert_eq!(f.view.columns.len(), 3);
    assert_eq!(f.view.columns[1].tabs, vec![f.spec]);
    assert_eq!(f.view.columns[2].tabs, vec![f.builder]);
    assert_eq!(f.view.grouped(), 0, "nothing is grouped any more");
    assert_eq!(f.view.focus, 2);
}

/// A tab already alone in its column is already what a split would produce, so the gesture changes
/// nothing rather than shuffling the row under the pointer.
#[test]
fn splitting_a_lone_tab_off_changes_nothing() {
    let mut f = seeded();
    let before: Vec<Vec<AgentId>> = f.view.columns.iter().map(|c| c.tabs.clone()).collect();
    assert!(f.view.split_off(f.fixer, 2));
    let after: Vec<Vec<AgentId>> = f.view.columns.iter().map(|c| c.tabs.clone()).collect();
    assert_eq!(before, after);
}

/// **A column's composer is its own for the column's life.** A slot is stable, so closing a column
/// to the left of another does not carry what was typed at one agent into a field addressed at a
/// different one — and the freed slot is cleared, because it is handed to the next column to open.
#[test]
fn a_slot_is_stable_for_a_columns_life_and_cleared_when_it_ends() {
    let mut f = seeded();
    let (first, second) = (f.view.columns[0].slot, f.view.columns[1].slot);
    assert_ne!(first, second);

    f.view.set_draft(second, "run the migration".to_string());
    f.view.bench(f.fixer);

    // The row is one shorter and the surviving column kept its slot, so its draft is still its own.
    assert_eq!(f.view.columns.len(), 1);
    assert_eq!(f.view.columns[0].slot, second);
    assert_eq!(f.view.draft(second), "run the migration");

    // And the slot the closed column had is empty, ready for the next column to be given it.
    f.view.set_draft(first, "typed at the fixer".to_string());
    f.view.bench(f.spec);
    f.view.bench(f.builder);
    assert_eq!(f.view.draft(second), "");
}

/// The host has stopped reporting an agent, so no column may still be drawing it.
///
/// A prune says whether it did anything, which is what lets a re-sent `WorkList` that changed
/// nothing cost no redraw — and it never re-arranges, because the arrangement is the user's.
#[test]
fn pruning_drops_tabs_for_agents_the_host_has_forgotten() {
    let mut f = seeded();
    assert!(!f.view.prune(&f.work), "nothing to do, and it says so");

    f.work.agents.retain(|a| a.id != f.builder);
    assert!(f.view.prune(&f.work));
    assert_eq!(f.view.columns[1].tabs, vec![f.spec]);
    assert_eq!(f.view.on_the_field(), 2);

    // The last agent of a column goes, and the column goes with it rather than drawing a tab
    // naming nothing.
    f.work.agents.retain(|a| a.id != f.fixer);
    assert!(f.view.prune(&f.work));
    assert_eq!(f.view.columns.len(), 1);
    assert_eq!(f.view.columns[0].tabs, vec![f.spec]);
}

/// Revealing is the sidebar's one gesture at both scales: an agent on the field comes to the front
/// of the column holding it, and a benched one is brought on.
#[test]
fn revealing_brings_an_agent_forward_or_brings_it_on() {
    let mut f = seeded();
    assert!(f.view.reveal(f.builder));
    assert_eq!(f.view.active_agent(1), Some(f.builder));
    assert_eq!(
        f.view.focus, 1,
        "and the column it is in is the focused one"
    );

    f.view.bench(f.builder);
    assert!(f.view.reveal(f.builder));
    assert_eq!(
        f.view.columns.len(),
        3,
        "a benched agent gets its own column"
    );
    assert_eq!(f.view.columns[2].tabs, vec![f.builder]);
    assert!(f.view.benched(&f.work).is_empty());
}

/// The screen is full at the ceiling, and the readers say so before a click has to fail.
#[test]
fn the_screen_is_full_at_the_ceiling_and_says_so() {
    let mut work = WorkProjection::empty();
    let one = SessionId::generate();
    let agents: Vec<WorkAgent> = (0..COLUMNS_MAX + 2)
        .map(|n| {
            agent(
                AgentId::generate(),
                one,
                &format!("worker-{n}"),
                Activity::Writing,
            )
        })
        .collect();
    let ids: Vec<AgentId> = agents.iter().map(|a| a.id).collect();
    work.replace_all(vec![session(one, "main", false)], agents, Vec::new());

    // One session, so `arrange` gives it one column holding all of them — the ceiling is on
    // columns, not on tabs.
    let mut view = AgentsView::default();
    view.arrange(&work);
    assert_eq!(view.columns.len(), 1);
    assert!(view.has_room());

    // Split them off one at a time until the row is full. The ones that find no room stay in the
    // column they were in — a split that cannot finish must not leave a tab nowhere.
    for id in &ids {
        let at = view.columns.len();
        view.split_off(*id, at);
        assert!(
            view.on_screen(*id),
            "a refused split leaves the tab where it was"
        );
    }
    assert_eq!(view.columns.len(), COLUMNS_MAX);
    assert!(!view.has_room());
    assert_eq!(view.on_the_field(), ids.len(), "and loses none of them");

    // With the row full, an agent taken off it cannot be given a column of its own.
    let stranded = *ids.last().expect("the fixture has agents");
    view.bench(stranded);
    assert_eq!(view.columns.len(), COLUMNS_MAX, "its column held others");
    assert!(
        !view.open(stranded),
        "a full screen has no ninth column to give"
    );

    // But grouping into a column that is already there always works — the ceiling is on columns —
    // so the sidebar's click still does what it says whatever the row looks like.
    view.focus_column(3);
    assert!(view.reveal(stranded), "reveal always shows the agent");
    assert_eq!(view.holds(stranded).map(|(col, _)| col), Some(3));
    assert_eq!(view.columns.len(), COLUMNS_MAX);
}

/// The counts are about **the field**, not the project, and the bench is exactly the difference.
/// That is what the status bar reports on, and reporting on the project would make the strip say
/// something the screen is not showing.
#[test]
fn the_counts_are_about_the_field_and_the_bench_is_the_difference() {
    let mut f = seeded();
    assert_eq!(f.view.count(&f.work, Bucket::Running), 2);
    assert_eq!(f.view.count(&f.work, Bucket::Ended), 1);
    assert_eq!(f.view.count(&f.work, Bucket::Error), 0);

    f.view.bench(f.builder);
    assert_eq!(f.view.count(&f.work, Bucket::Running), 1);
    assert_eq!(
        f.view.on_the_field() + f.view.benched(&f.work).len(),
        f.work.agents.len()
    );
}

/// A session folds and unfolds, and one that has never been touched is open — so a session that
/// arrives after the screen was last looked at arrives visible rather than hidden.
#[test]
fn a_session_folds_and_starts_open() {
    let mut f = seeded();
    assert!(!f.view.is_collapsed(f.refit));

    f.view.toggle_session(f.refit);
    assert!(f.view.is_collapsed(f.refit));
    assert!(!f.view.is_collapsed(f.store));

    f.view.toggle_session(f.refit);
    assert!(!f.view.is_collapsed(f.refit));
}
