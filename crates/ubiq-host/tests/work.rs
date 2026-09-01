//! A project's work: the task store, and the service over it.
//!
//! Two halves, because the module has two. The store half is about a real `tasks.toml` — where it
//! lands, what an absent one means, and what happens to one this Ubiq cannot read. The service
//! half is about the seeding rule and the ten edits, against a store with no disk under it.
//!
//! The interesting cases are the ones where doing the obvious thing loses the user's data: seeding
//! over a file that already said "no tasks", writing over a file from a newer Ubiq, or leaving an
//! agent's card pointing at a task that has gone.

use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;
use ubiq_host::reply::Reply;
use ubiq_host::store::file::{FileTaskStore, TASKS_VERSION};
use ubiq_host::store::memory::MemoryTaskStore;
use ubiq_host::store::{StoreError, TaskStore};
use ubiq_host::work::{Work, mock};
use ubiq_proto::ids::{ProjectId, SessionId, StepId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::work::{
    AgentId, Priority, Shape, Speaker, Status, Step, StepState, TaskRecord, WorkAgent, WorkSession,
};

// ── the store, against a real file ──────────────────────────────────

/// A task with fixed timestamps, so a round trip can be compared whole.
fn record(title: &str) -> TaskRecord {
    TaskRecord::new(
        title.to_string(),
        None,
        Utc.with_ymd_and_hms(2026, 8, 14, 9, 12, 44).unwrap(),
    )
}

fn file_store(dir: &TempDir) -> FileTaskStore {
    FileTaskStore::new(dir.path().to_path_buf())
}

fn titles(tasks: &[TaskRecord]) -> Vec<&str> {
    tasks.iter().map(|t| t.title.as_str()).collect()
}

#[test]
fn a_list_of_tasks_survives_the_round_trip_in_the_order_it_was_held() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();

    let mut want = vec![
        record("third from the top"),
        record("second"),
        record("first"),
        record("and one with an outline"),
    ];
    want[0].status = Status::InProgress;
    want[0].priority = Priority::High;
    want[0].shape = Shape::Coordinated;
    want[0].description = "notes\nover two lines".to_string();
    want[0].session = Some(mock::sessions()[1].id);
    want[3].steps = vec![Step::new("one".to_string()), Step::new("two".to_string())];
    want[3].steps[1].state = StepState::Failed;
    want[3].steps[1].owner = Some(AgentId::generate());

    store.save(project, &want).unwrap();

    // The order is the user's: the board draws the tasks in the order it is handed them, so a
    // store that sorted them would silently rearrange the board on the next boot.
    let reread = file_store(&dir);
    assert_eq!(reread.load(project).unwrap(), Some(want));
}

#[test]
fn a_projects_tasks_live_under_its_own_directory_which_the_first_save_creates() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();

    assert_eq!(
        store.path(project),
        dir.path()
            .join("projects")
            .join(project.to_string())
            .join("tasks.toml")
    );

    // A project that never had view state has no directory of its own yet, and must still be able
    // to write a task.
    assert!(!store.path(project).parent().unwrap().exists());
    store.save(project, &[record("the first one")]).unwrap();
    assert!(store.path(project).exists());
}

/// An absent file is `Ok(None)` and a file holding an empty list is `Ok(Some(vec![]))`.
///
/// That is the seeding rule in one assert: the first is a project whose tasks were never written
/// and may be seeded from the fixture; the second is a user who deleted every task, and must get
/// their empty board back rather than the fixture again.
#[test]
fn an_absent_file_and_a_file_with_no_tasks_are_different_things() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();

    assert_eq!(store.load(project).unwrap(), None);

    store.save(project, &[]).unwrap();
    assert_eq!(store.load(project).unwrap(), Some(Vec::new()));
}

#[test]
fn tasks_from_a_newer_ubiq_are_refused_and_never_overwritten() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();
    let path = store.path(project);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = format!(
        "version = {}\n\n[[task]]\ntitle = \"written by something newer\"\n",
        TASKS_VERSION + 1
    );
    fs::write(&path, &body).unwrap();

    let error = store.load(project).unwrap_err();
    match error {
        StoreError::UnknownVersion {
            found, supported, ..
        } => {
            assert_eq!(found, TASKS_VERSION + 1);
            assert_eq!(supported, TASKS_VERSION);
        }
        other => panic!("expected an unknown version, got {other:?}"),
    }

    // Deliberately not treated as corruption: nothing is moved aside and nothing is rewritten,
    // because a format that holds more than this one can must be left exactly as it is.
    assert_eq!(fs::read_to_string(&path).unwrap(), body);
}

#[test]
fn a_corrupt_task_file_is_kept_aside_rather_than_truncated() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();
    let path = store.path(project);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = "version = 1\nthis is not a task list\n";
    fs::write(&path, body).unwrap();

    let error = store.load(project).unwrap_err();

    let preserved = match error {
        StoreError::Parse { preserved_as, .. } => preserved_as.expect("kept aside"),
        other => panic!("expected a parse failure, got {other:?}"),
    };
    assert!(preserved.exists(), "the original must still be on disk");
    assert!(
        preserved.to_string_lossy().contains(".corrupt-"),
        "and be findable: {}",
        preserved.display()
    );
    // The user's tasks are worth as much as the catalogue: what was there is still readable.
    assert_eq!(fs::read_to_string(&preserved).unwrap(), body);
}

#[test]
fn a_rewrite_leaves_no_temporary_file_beside_it() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();

    for n in 0..5 {
        store
            .save(project, &[record(&format!("take {n}"))])
            .unwrap();
    }

    let mut left: Vec<String> = fs::read_dir(store.path(project).parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, vec!["tasks.toml".to_string()]);
}

#[test]
fn clearing_a_project_that_never_wrote_a_task_is_fine() {
    let dir = TempDir::new().unwrap();
    let store = file_store(&dir);
    let project = ProjectId::generate();

    assert!(store.clear(project).is_ok());

    store.save(project, &[record("one")]).unwrap();
    store.clear(project).unwrap();
    assert!(!store.path(project).exists());
    assert_eq!(store.load(project).unwrap(), None);
}

// ── the service, against a store with no disk under it ──────────────

/// A second name for the store the service holds.
///
/// [`Work::open`] takes a `Box<dyn TaskStore>` and keeps it, so a test that wants to count the
/// writes it made — or change what it holds underneath a live service — needs the same store under
/// two owners.
struct Shared(Arc<MemoryTaskStore>);

impl TaskStore for Shared {
    fn load(&self, project: ProjectId) -> Result<Option<Vec<TaskRecord>>, StoreError> {
        self.0.load(project)
    }

    fn save(&self, project: ProjectId, tasks: &[TaskRecord]) -> Result<(), StoreError> {
        self.0.save(project, tasks)
    }

    fn clear(&self, project: ProjectId) -> Result<(), StoreError> {
        self.0.clear(project)
    }
}

/// A service over a store the test keeps a handle on.
fn work(store: &Arc<MemoryTaskStore>) -> Work {
    Work::open(Box::new(Shared(Arc::clone(store))))
}

/// A fresh service on an unwritten project, which is what a first boot is.
fn unseeded() -> (Arc<MemoryTaskStore>, Work, ProjectId) {
    let store = Arc::new(MemoryTaskStore::new());
    let work = work(&store);
    (store, work, ProjectId::generate())
}

/// The three lists out of the one [`Message::WorkList`] in a set of replies.
fn listing(replies: &[Reply]) -> (Vec<WorkSession>, Vec<WorkAgent>, Vec<TaskRecord>) {
    for reply in replies {
        if let Message::WorkList {
            sessions,
            agents,
            tasks,
            ..
        } = reply.message()
        {
            return (sessions.clone(), agents.clone(), tasks.clone());
        }
    }
    panic!("no listing among {replies:?}");
}

/// The board as a `ListWork` would answer it.
fn board(work: &mut Work, project: ProjectId) -> Vec<TaskRecord> {
    listing(&work.list(project)).2
}

/// The one changed task in a set of replies.
fn changed(replies: &[Reply]) -> TaskRecord {
    for reply in replies {
        if let Message::TaskChanged { task, .. } = reply.message() {
            return task.clone();
        }
    }
    panic!("nothing changed among {replies:?}");
}

/// The one created task in a set of replies.
fn created(replies: &[Reply]) -> TaskRecord {
    for reply in replies {
        if let Message::TaskCreated { task, .. } = reply.message() {
            return task.clone();
        }
    }
    panic!("nothing was created among {replies:?}");
}

/// Every agent a set of replies says has moved.
fn moved_agents(replies: &[Reply]) -> Vec<WorkAgent> {
    replies
        .iter()
        .filter_map(|reply| match reply.message() {
            Message::AgentChanged { agent, .. } => Some((**agent).clone()),
            _ => None,
        })
        .collect()
}

/// The one agent a set of replies says has moved.
fn moved_agent(replies: &[Reply]) -> WorkAgent {
    let mut moved = moved_agents(replies);
    assert_eq!(moved.len(), 1, "expected one agent, got {replies:?}");
    moved.remove(0)
}

/// Every refusal in a set of replies, as the task it named and what it said.
fn refusals(replies: &[Reply]) -> Vec<(Option<TaskId>, String)> {
    replies
        .iter()
        .filter_map(|reply| match reply.message() {
            Message::WorkError { task_id, error, .. } => Some((*task_id, error.clone())),
            _ => None,
        })
        .collect()
}

/// The first task holding a step in `state`, and that step.
fn a_step(board: &[TaskRecord], state: StepState) -> (TaskId, StepId) {
    for task in board {
        if let Some(step) = task.steps.iter().find(|s| s.state == state) {
            return (task.id, step.id);
        }
    }
    panic!("the fixture has no {state:?} step");
}

#[test]
fn the_first_ask_answers_the_fixture_and_writes_it_once() {
    let (store, mut work, project) = unseeded();

    let replies = work.list(project);

    let (sessions, agents, tasks) = listing(&replies);
    assert_eq!(sessions.len(), 5);
    assert_eq!(agents.len(), 11);
    assert_eq!(tasks.len(), 10);
    // Written at the first look rather than at the first edit, so what the user sees is already
    // theirs: renamable, movable, deletable, and still there after a restart.
    assert_eq!(store.writes(), 1);
    assert!(refusals(&replies).is_empty());
}

/// A project whose file says "no tasks" is never seeded again.
///
/// The deleted-everything-then-restarted case, and the whole reason the store answers
/// `Option<Vec<_>>` rather than `Vec<_>`: an absent file and an empty one are different things,
/// and getting this wrong hands the user back ten cards they deliberately threw away.
#[test]
fn a_project_that_wrote_an_empty_board_gets_its_empty_board_back() {
    let project = ProjectId::generate();
    let store = Arc::new(MemoryTaskStore::with(project, Vec::new()));
    let mut work = work(&store);

    let replies = work.list(project);

    let (sessions, agents, tasks) = listing(&replies);
    assert!(tasks.is_empty(), "the fixture must not come back");
    assert_eq!(
        store.writes(),
        0,
        "and nothing may be written over the file"
    );
    // The invented half is still answered: it has no file behind it to be empty.
    assert_eq!(sessions.len(), 5);
    assert_eq!(agents.len(), 11);
}

#[test]
fn a_second_boot_answers_what_the_file_holds_in_order() {
    let project = ProjectId::generate();
    let store = Arc::new(MemoryTaskStore::with(
        project,
        vec![record("mine, first"), record("mine, second")],
    ));
    let mut work = work(&store);

    let tasks = board(&mut work, project);

    assert_eq!(titles(&tasks), ["mine, first", "mine, second"]);
    assert_eq!(store.writes(), 0);
}

/// A load that failed is never seeded over.
///
/// Seeding here is how you overwrite the thing preserving it was meant to save. The empty list
/// *is* written down, and deliberately: the corrupt file was moved aside, so the next boot would
/// find no file, call that a new project, and put the fixture on top of what the user just lost.
#[test]
fn a_load_that_fails_answers_an_empty_board_rather_than_the_fixture() {
    let (store, mut work, project) = unseeded();
    store.fail_load(true);

    let replies = work.list(project);

    assert_eq!(refusals(&replies).len(), 1, "the user is told, once");
    assert!(
        listing(&replies).2.is_empty(),
        "the fixture must not appear"
    );
    assert_eq!(
        store.writes(),
        1,
        "and the empty board is what gets written down"
    );
}

/// A read that failed for any other reason leaves the file exactly as it was.
///
/// The corrupt path writes an empty list down deliberately, because `preserve_aside` moved the file
/// away and an absent file would reseed the fixture over what the user lost. Every other failure is
/// the opposite case: `tasks.toml` is still there and still whole — a permissions blip, an EIO, a
/// mount that has not come up — and writing over it would turn one unlucky read into the loss of the
/// board. A directory where the file should be is the portable way to make `read_to_string` fail
/// without corrupting anything.
#[test]
fn a_read_that_failed_without_moving_the_file_leaves_it_alone() {
    let dir = TempDir::new().unwrap();
    let store = FileTaskStore::new(dir.path().to_path_buf());
    let project = ProjectId::generate();
    let path = store.path(project);
    fs::create_dir_all(&path).unwrap();

    let mut work = Work::open(Box::new(FileTaskStore::new(dir.path().to_path_buf())));
    let replies = work.list(project);

    assert_eq!(refusals(&replies).len(), 1, "the user is told, once");
    assert!(
        listing(&replies).2.is_empty(),
        "the fixture must not appear over something that could not be read"
    );
    assert!(
        path.is_dir(),
        "and what was there is still there, untouched"
    );
}

/// Tasks from a newer Ubiq are read by nobody and written by nobody.
///
/// The one load failure that is *not* recoverable, and the one where the file is still there: it
/// was never moved aside, so writing anything at all — even the empty list the corrupt path writes
/// — would replace a format that holds more than this one can with a format that cannot.
#[test]
fn a_project_sealed_by_a_newer_version_is_never_written_to() {
    let dir = TempDir::new().unwrap();
    let store = FileTaskStore::new(dir.path().to_path_buf());
    let project = ProjectId::generate();
    let path = store.path(project);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = format!("version = {}\n", TASKS_VERSION + 1);
    fs::write(&path, &body).unwrap();

    let mut work = Work::open(Box::new(FileTaskStore::new(dir.path().to_path_buf())));

    let replies = work.list(project);
    assert_eq!(refusals(&replies).len(), 1);
    assert!(listing(&replies).2.is_empty());

    // Every later edit refuses too, and says so at most once more.
    let replies = work.create(project, "something of my own".to_string(), None);
    assert!(refusals(&replies).is_empty(), "already said: {replies:?}");

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        body,
        "the newer file must be exactly as it was"
    );
}

#[test]
fn an_update_changes_only_the_fields_it_was_given() {
    let (_store, mut work, project) = unseeded();
    let before = board(&mut work, project).remove(0);
    assert!(
        !before.description.is_empty(),
        "the fixture's first card has notes to clear"
    );

    // An all-whitespace title is a slip rather than an intention, so it is ignored; a description
    // may be emptied, because clearing one is a thing to mean.
    let replies = work.update(
        project,
        before.id,
        Some("   ".to_string()),
        Some(String::new()),
        None,
        None,
    );

    let after = changed(&replies);
    assert_eq!(after.title, before.title);
    assert_eq!(after.description, "");
    assert_eq!(after.priority, before.priority);
    assert_eq!(after.shape, before.shape);
    assert_eq!(after.status, before.status);
    assert_eq!(after.session, before.session);
    assert_eq!(after.steps, before.steps);
    assert_eq!(
        after.created_at, before.created_at,
        "when a task was named does not move"
    );
    assert!(
        after.updated_at > before.updated_at,
        "but when it last changed does"
    );

    // And the fields it does name are trimmed and taken.
    let after = changed(&work.update(
        project,
        before.id,
        Some("  a better name  ".to_string()),
        None,
        Some(Priority::Low),
        Some(Shape::Chain),
    ));
    assert_eq!(after.title, "a better name");
    assert_eq!(after.priority, Priority::Low);
    assert_eq!(after.shape, Shape::Chain);
    assert_eq!(after.description, "", "and nothing else moved with them");
}

#[test]
fn an_edit_to_a_task_that_is_not_there_is_refused_and_writes_nothing() {
    let (store, mut work, project) = unseeded();
    board(&mut work, project);
    let writes = store.writes();
    let stranger = TaskId::generate();

    for replies in [
        work.update(project, stranger, Some("x".to_string()), None, None, None),
        work.move_task(project, stranger, Status::Done),
        work.toggle_step(project, stranger, StepId::generate()),
    ] {
        let refusals = refusals(&replies);
        assert_eq!(refusals.len(), 1, "got {replies:?}");
        // The refusal names the task, because that is the card the message is drawn over.
        assert_eq!(refusals[0].0, Some(stranger));
    }
    assert_eq!(store.writes(), writes);
}

#[test]
fn a_move_into_the_column_a_task_is_already_in_says_nothing() {
    let (store, mut work, project) = unseeded();
    let task = board(&mut work, project).remove(0);
    let writes = store.writes();

    let replies = work.move_task(project, task.id, task.status);

    // A drop that landed where the card already was costs no write and no redraw.
    assert!(replies.is_empty(), "got {replies:?}");
    assert_eq!(store.writes(), writes);

    // A drop that moved it does both.
    let after = changed(&work.move_task(project, task.id, Status::Done));
    assert_eq!(after.status, Status::Done);
    assert_eq!(store.writes(), writes + 1);
}

#[test]
fn unticking_a_step_lands_on_idle_and_ticking_one_lands_on_done() {
    let (_store, mut work, project) = unseeded();
    let board = board(&mut work, project);

    // Unticking cannot know what its owner would go back to doing, so it lands on idle rather
    // than on whatever the step was before.
    let (task, step) = a_step(&board, StepState::Done);
    let after = changed(&work.toggle_step(project, task, step));
    assert_eq!(after.step(step).unwrap().state, StepState::Idle);

    let (task, step) = a_step(&board, StepState::NeedsYou);
    let after = changed(&work.toggle_step(project, task, step));
    assert_eq!(after.step(step).unwrap().state, StepState::Done);
}

#[test]
fn a_step_is_appended_unowned_and_idle_and_needs_a_title() {
    let (store, mut work, project) = unseeded();
    let task = board(&mut work, project).remove(0);
    let writes = store.writes();

    assert_eq!(
        refusals(&work.add_step(project, task.id, "   ".to_string())).len(),
        1
    );
    assert_eq!(store.writes(), writes, "a refusal writes nothing");

    let after = changed(&work.add_step(project, task.id, "  and one more  ".to_string()));

    assert_eq!(after.steps.len(), task.steps.len() + 1);
    let added = after.steps.last().unwrap();
    assert_eq!(added.title, "and one more");
    assert_eq!(added.state, StepState::Idle);
    assert_eq!(added.owner, None, "nobody has picked it up yet");
}

#[test]
fn renaming_a_step_needs_a_title_too() {
    let (_store, mut work, project) = unseeded();
    let task = board(&mut work, project).remove(0);
    let step = task.steps[0].id;

    assert_eq!(
        refusals(&work.rename_step(project, task.id, step, " \t ".to_string())).len(),
        1
    );

    let after = changed(&work.rename_step(project, task.id, step, "  renamed  ".to_string()));
    assert_eq!(after.step(step).unwrap().title, "renamed");
}

#[test]
fn removing_a_step_shortens_the_list() {
    let (_store, mut work, project) = unseeded();
    let task = board(&mut work, project).remove(0);
    let step = task.steps[0].id;

    let after = changed(&work.remove_step(project, task.id, step));

    assert_eq!(after.steps.len(), task.steps.len() - 1);
    assert!(after.step(step).is_none());
}

#[test]
fn moving_a_step_reorders_it_and_is_clamped_past_the_end() {
    let (store, mut work, project) = unseeded();
    let task = board(&mut work, project)
        .into_iter()
        .max_by_key(|t| t.steps.len())
        .unwrap();
    let first = task.steps[0].id;

    let after = changed(&work.move_step(project, task.id, first, 2));
    assert_eq!(after.steps[2].id, first);
    assert_eq!(after.steps.len(), task.steps.len(), "nothing was lost");

    // A list that shortened under a drag is not an error the user can do anything about.
    let after = changed(&work.move_step(project, task.id, first, 999));
    assert_eq!(after.steps.last().unwrap().id, first);

    // And a drag that ended where it started costs no write and no redraw.
    let writes = store.writes();
    assert!(work.move_step(project, task.id, first, 999).is_empty());
    assert_eq!(store.writes(), writes);
}

#[test]
fn every_step_edit_refuses_a_step_that_is_not_there_and_writes_nothing() {
    let (store, mut work, project) = unseeded();
    let task = board(&mut work, project).remove(0);
    let writes = store.writes();
    let stranger = StepId::generate();

    for replies in [
        work.rename_step(project, task.id, stranger, "x".to_string()),
        work.remove_step(project, task.id, stranger),
        work.move_step(project, task.id, stranger, 0),
        work.toggle_step(project, task.id, stranger),
    ] {
        let refusals = refusals(&replies);
        assert_eq!(refusals.len(), 1, "got {replies:?}");
        assert_eq!(refusals[0].0, Some(task.id));
    }
    assert_eq!(store.writes(), writes);
}

#[test]
fn a_task_may_only_be_handed_to_a_session_the_project_has() {
    let (store, mut work, project) = unseeded();
    let task = board(&mut work, project).remove(0);
    assert_eq!(task.session, None, "the fixture's first card is unstarted");
    let session = mock::sessions()[0].id;

    let after = changed(&work.assign(project, task.id, Some(session)));
    assert_eq!(after.session, Some(session));

    // A session nobody has is refused rather than written down, because a task pointing at one
    // would draw an empty pill for the rest of the project's life.
    let writes = store.writes();
    let replies = work.assign(project, task.id, Some(SessionId::generate()));
    assert_eq!(refusals(&replies).len(), 1, "got {replies:?}");
    assert_eq!(store.writes(), writes);
    assert_eq!(board(&mut work, project)[0].session, Some(session));

    // Taking it back is always allowed: a task nobody has started is a state, not an absence.
    let after = changed(&work.assign(project, task.id, None));
    assert_eq!(after.session, None);
}

#[test]
fn a_created_task_is_a_backlog_card_with_nothing_on_it_yet() {
    let (_store, mut work, project) = unseeded();

    assert_eq!(
        refusals(&work.create(project, String::new(), None)).len(),
        1
    );
    assert_eq!(
        refusals(&work.create(project, "  \t ".to_string(), None)).len(),
        1
    );

    let replies = work.create(project, "  Name the events  ".to_string(), None);

    let task = created(&replies);
    assert_eq!(task.title, "Name the events");
    assert_eq!(task.status, Status::Backlog);
    assert_eq!(task.priority, Priority::Normal);
    assert_eq!(task.shape, Shape::Direct);
    assert_eq!(task.session, None);
    assert!(task.steps.is_empty());
    assert!(task.description.is_empty());
    // Its own variant rather than a change: the interface cannot know an id it did not mint, and
    // the board selects the card it just made.
    assert!(
        !replies
            .iter()
            .any(|r| matches!(r.message(), Message::TaskChanged { .. })),
        "a new task is created, never changed"
    );
    assert_eq!(board(&mut work, project).last().unwrap().id, task.id);
}

#[test]
fn deleting_a_task_takes_every_agent_off_it() {
    let (_store, mut work, project) = unseeded();
    let (_, agents, tasks) = listing(&work.list(project));
    let target = tasks
        .iter()
        .find(|t| t.status == Status::InProgress)
        .expect("the fixture has work in flight")
        .clone();
    let on_it: HashSet<AgentId> = agents
        .iter()
        .filter(|a| a.task == Some(target.id))
        .map(|a| a.id)
        .collect();
    assert!(
        !on_it.is_empty(),
        "the fixture puts agents on the task in flight"
    );

    let replies = work.delete(project, target.id);

    assert!(
        replies.iter().any(|r| matches!(
            r.message(),
            Message::TaskDeleted { task_id, .. } if *task_id == target.id
        )),
        "got {replies:?}"
    );
    // A card pointing at a deleted task would be drawn in no container and counted in one, so the
    // repair is said out loud rather than left for the interface to work out.
    let moved = moved_agents(&replies);
    assert_eq!(
        moved.iter().map(|a| a.id).collect::<HashSet<_>>(),
        on_it,
        "every agent that was on it, and no other"
    );
    assert!(moved.iter().all(|a| a.task.is_none()));
    assert!(!board(&mut work, project).iter().any(|t| t.id == target.id));

    assert_eq!(refusals(&work.delete(project, target.id)).len(), 1);
}

#[test]
fn an_unwritable_store_keeps_the_edit_and_says_so_once() {
    let project = ProjectId::generate();
    let store = Arc::new(MemoryTaskStore::with(
        project,
        vec![record("one"), record("two")],
    ));
    let mut work = work(&store);
    let tasks = board(&mut work, project);
    store.fail_writes(true);

    let first = work.update(
        project,
        tasks[0].id,
        Some("renamed".to_string()),
        None,
        None,
        None,
    );
    let second = work.update(
        project,
        tasks[1].id,
        Some("also renamed".to_string()),
        None,
        None,
        None,
    );

    assert_eq!(refusals(&first).len(), 1, "the user hears it once");
    assert!(
        refusals(&second).is_empty(),
        "and not on every keystroke afterwards: {second:?}"
    );
    // Memory is authoritative for the session, so an unwritable store never stops the board
    // working — it only stops it surviving a restart.
    assert_eq!(changed(&first).title, "renamed");
    assert_eq!(changed(&second).title, "also renamed");
    assert_eq!(
        titles(&board(&mut work, project)),
        ["renamed", "also renamed"]
    );
}

#[test]
fn forgetting_a_project_drops_what_was_in_memory() {
    let project = ProjectId::generate();
    let store = Arc::new(MemoryTaskStore::with(project, vec![record("as it was")]));
    let mut work = work(&store);
    assert_eq!(titles(&board(&mut work, project)), ["as it was"]);

    // The file changes underneath, which is what a `ForgetProject` and a re-add amount to.
    store
        .save(project, &[record("and now"), record("something else")])
        .unwrap();
    assert_eq!(
        titles(&board(&mut work, project)),
        ["as it was"],
        "memory is authoritative until it is dropped"
    );

    work.forget(project);

    assert_eq!(
        titles(&board(&mut work, project)),
        ["and now", "something else"]
    );
}

/// Every reply the work makes is for the window that asked.
///
/// Nothing in this family is broadcast, because a project is open in exactly one window at a time:
/// the window that asked is the only one drawing that project's work. The file family's rule, for
/// the file family's reason.
#[test]
fn nothing_in_the_work_family_is_broadcast() {
    let (store, mut work, project) = unseeded();
    let (_, agents, tasks) = listing(&work.list(project));
    let task = tasks[0].clone();
    let agent = agents[0].id;
    let step = task.steps[0].id;

    let mut replies = Vec::new();
    replies.extend(work.list(project));
    replies.extend(work.create(project, "a new one".to_string(), None));
    replies.extend(work.create(project, "  ".to_string(), None));
    replies.extend(work.update(
        project,
        task.id,
        Some("renamed".to_string()),
        Some("notes".to_string()),
        Some(Priority::High),
        Some(Shape::Chain),
    ));
    replies.extend(work.move_task(project, task.id, Status::InReview));
    replies.extend(work.assign(project, task.id, Some(mock::sessions()[2].id)));
    replies.extend(work.assign(project, task.id, Some(SessionId::generate())));
    replies.extend(work.add_step(project, task.id, "one more".to_string()));
    replies.extend(work.rename_step(project, task.id, step, "renamed".to_string()));
    replies.extend(work.move_step(project, task.id, step, 1));
    replies.extend(work.toggle_step(project, task.id, step));
    replies.extend(work.remove_step(project, task.id, step));
    replies.extend(work.assign_agent(project, agent, Some(tasks[1].id)));
    replies.extend(work.assign_agent(project, agent, Some(TaskId::generate())));
    replies.extend(work.send_to_agent(project, agent, "how is it going?".to_string()));
    // The store failing is the last kind of thing this service says.
    store.fail_writes(true);
    replies.extend(work.delete(project, task.id));

    assert!(replies.len() > 15, "a thin sample proves nothing");
    for reply in &replies {
        assert!(
            !reply.is_broadcast(),
            "{:?} went to every window",
            reply.message()
        );
    }
}

#[test]
fn moving_an_agent_to_another_task_clears_its_parent() {
    let (_store, mut work, project) = unseeded();
    let (_, agents, tasks) = listing(&work.list(project));
    let child = agents
        .iter()
        .find(|a| a.parent.is_some())
        .expect("the fixture spawns workers under a lead")
        .clone();
    let elsewhere = tasks
        .iter()
        .find(|t| Some(t.id) != child.task)
        .unwrap()
        .clone();

    let after = moved_agent(&work.assign_agent(project, child.id, Some(elsewhere.id)));

    assert_eq!(after.task, Some(elsewhere.id));
    // An agent that moved to another task no longer answers to whoever spawned it there, so the
    // connector to its old parent must not be drawn any more.
    assert_eq!(after.parent, None);

    // Dropping a card back where it already is costs no redraw.
    let replies = work.assign_agent(project, child.id, Some(elsewhere.id));
    assert!(replies.is_empty(), "got {replies:?}");

    let replies = work.assign_agent(project, child.id, Some(TaskId::generate()));
    assert_eq!(refusals(&replies).len(), 1, "got {replies:?}");

    // And out of every task is always allowed.
    let after = moved_agent(&work.assign_agent(project, child.id, None));
    assert_eq!(after.task, None);
}

/// A line to an agent is appended to its thread, and nothing answers it.
///
/// Not an omission: a fabricated reply is the one thing a screen with no live agent must not draw,
/// so the absence of an answering turn is the behaviour being pinned rather than a gap in it.
#[test]
fn a_line_to_an_agent_is_appended_and_nothing_answers_it() {
    let (_store, mut work, project) = unseeded();
    let (_, agents, _) = listing(&work.list(project));
    let agent = agents[0].clone();
    let before = agent.thread.len();

    assert!(
        work.send_to_agent(project, agent.id, "  \n ".to_string())
            .is_empty(),
        "an empty line is not a turn"
    );

    let after =
        moved_agent(&work.send_to_agent(project, agent.id, "  what is the hold-up?  ".to_string()));

    assert_eq!(after.thread.len(), before + 1);
    let last = after.thread.last().unwrap();
    assert_eq!(last.from, Speaker::You);
    assert_eq!(last.text, "what is the hold-up?");
    assert!(
        after.thread[before..]
            .iter()
            .all(|t| t.from == Speaker::You),
        "nothing answers, and the thread must say so by staying silent"
    );

    assert_eq!(
        refusals(&work.send_to_agent(project, AgentId::generate(), "hello?".to_string())).len(),
        1
    );
}

/// The mock's agents come back attached to a task that is actually on the board, or to none.
///
/// The fixture cannot name a task id — the ids belong to whatever `tasks.toml` holds — so the link
/// is made where both lists are in hand. Without it the graph draws eleven cards and not one
/// outline.
///
/// **No task is a real answer.** An agent whose session has nothing in flight is left unlinked,
/// because the graph draws an unowned card above the containers and that is where the project
/// manager coordinating everything belongs. Reaching for the session's first task instead put it
/// inside a container for work nobody is doing.
#[test]
fn the_mock_agents_come_back_linked_to_a_task_that_exists() {
    let (_store, mut work, project) = unseeded();
    let (sessions, agents, tasks) = listing(&work.list(project));
    let ids: HashSet<TaskId> = tasks.iter().map(|t| t.id).collect();

    // A link, where there is one, names a task the board actually holds.
    for agent in &agents {
        if let Some(task) = agent.task {
            assert!(
                ids.contains(&task),
                "{} names no task on the board",
                agent.name
            );
        }
    }

    // An agent serves the task its session has in flight, and none when its session has not.
    for session in &sessions {
        let flight = tasks.iter().find(|t| {
            t.session == Some(session.id)
                && matches!(t.status, Status::InProgress | Status::InReview)
        });
        for agent in agents.iter().filter(|a| a.session == session.id) {
            assert_eq!(
                agent.task,
                flight.map(|t| t.id),
                "{} is in {}, which has {:?} in flight",
                agent.name,
                session.name,
                flight.map(|t| &t.title)
            );
        }
    }

    // And the fixture really does exercise both arms, so this test cannot pass by drawing no
    // conclusion: the project manager is on `main`, whose work is all finished.
    let boss = agents
        .iter()
        .find(|a| a.role == "Project manager")
        .expect("the fixture has a project manager");
    assert_eq!(
        boss.task, None,
        "the one agent coordinating everything sits above the containers"
    );
    assert!(
        agents.iter().any(|a| a.task.is_some()),
        "and the rest are in a container"
    );
}
