//! The outbound half, the authority switch and drift — over the same Trello fixtures (`D188`).
//!
//! **The provider here is a board, not a mock.** It parses the recorded cards with the real Trello
//! client exactly as `tasksrc_sync.rs` does, and it then *keeps* what a write does to them: an
//! `update` applies the patch to its own copy, moves the revision on, and answers the card as it
//! now is. So every assertion below is about the **board and the project afterwards** — what the
//! remote holds, what the task holds, and what the link row says — and not about whether a method
//! was called.
//!
//! That is what makes the conflict table testable at all. A mock that recorded calls would pass
//! with the table inverted; a board that remembers cannot.
//!
//! Four rules are pinned here, and none of them belongs to Trello:
//!
//! - a field only the task changed is **pushed**, and only where the provider takes it;
//! - a field only the remote changed is **pulled**;
//! - a field both sides changed is settled by the binding's switch, the loser's value is kept in a
//!   comment, and the task is filed `Drifted`;
//! - a refused write is a **`Conflict`** and **nothing is overwritten on either side**.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Result, bail};
use chrono::Utc;
use serde_json::Value;
use ubiq_host::store::memory::MemoryTaskStore;
use ubiq_host::tasksrc::store::{TasksrcFile, seed_lanes};
use ubiq_host::tasksrc::sync::{Pass, Reconcile};
use ubiq_host::tasksrc::trello;
use ubiq_host::tasksrc::{
    Authority, Binding, Direction, DriftSide, Identity, LinkState, TaskLink, TaskProvider,
};
use ubiq_host::work::{Handle, Work};
use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::{ConnectionId, ProjectId, TaskId};
use ubiq_proto::tasksrc::{
    Facets, ProviderCaps, RemoteCheckId, RemoteComment, RemoteContainer, RemoteContainerId,
    RemoteDraft, RemoteItem, RemoteItemId, RemoteLane, RemoteLaneId, RemotePatch, Revision,
};
use ubiq_proto::work::{Status, TaskRecord};

const LISTS: &str = include_str!("fixtures/trello/lists.json");
const CARDS: &str = include_str!("fixtures/trello/cards.json");

const BOARD: &str = "5f2a7b1c9d0e4a0012345601";
const DOING: &str = "6a1b2c3d4e5f60007000a102";
const DONE: &str = "6a1b2c3d4e5f60007000a103";

// ── the board that remembers ─────────────────────────────────────────

/// Trello's own parsing, over recorded JSON, with a board that keeps what a write did to it.
struct Board {
    items: Mutex<Vec<RemoteItem>>,
    caps: ProviderCaps,
    /// When set, every `update` is refused — which is what a provider does when somebody wrote
    /// between the read and the push, whether it honours a precondition or emulates one.
    refuse: bool,
}

impl Board {
    fn new(caps: ProviderCaps) -> Self {
        let binding = plain();
        let items = trello::parse_cards(&json(CARDS), &binding);
        Self {
            items: Mutex::new(items),
            caps,
            refuse: false,
        }
    }

    fn refusing(caps: ProviderCaps) -> Self {
        Self {
            refuse: true,
            ..Self::new(caps)
        }
    }

    /// What the board holds for one card right now.
    fn card(&self, id: &RemoteItemId) -> RemoteItem {
        self.items
            .lock()
            .expect("the board")
            .iter()
            .find(|held| held.id == *id)
            .cloned()
            .expect("the fixture has this card")
    }

    /// Somebody else edits the card at the remote, between two passes.
    fn edited_at_the_remote(&self, id: &RemoteItemId, edit: impl FnOnce(&mut RemoteItem)) {
        let mut items = self.items.lock().expect("the board");
        let item = items
            .iter_mut()
            .find(|held| held.id == *id)
            .expect("the fixture has this card");
        edit(item);
        bump(item);
    }
}

/// A card's version token moves on any action, which is the only thing Trello gives.
fn bump(item: &mut RemoteItem) {
    item.revision = Revision::new(format!("{}+", item.revision.as_str()));
    item.updated_at = Utc::now();
}

impl TaskProvider for Board {
    fn id(&self) -> &'static str {
        trello::ID
    }

    fn label(&self) -> &'static str {
        "Trello"
    }

    fn caps(&self) -> ProviderCaps {
        self.caps
    }

    fn config_schema(&self) -> &'static [ubiq_proto::tasksrc::ConfigField] {
        &[]
    }

    fn containers(&self, _who: &Identity) -> Result<Vec<RemoteContainer>> {
        unimplemented!("a pass picks no container")
    }

    fn lanes(&self, _who: &Identity, _binding: &Binding) -> Result<Vec<RemoteLane>> {
        Ok(trello::parse_lists(&json(LISTS)))
    }

    fn facets(&self, _who: &Identity, _binding: &Binding) -> Result<Facets> {
        unimplemented!("a pass builds no picker")
    }

    fn query(&self, _who: &Identity, _binding: &Binding) -> Result<Vec<RemoteItem>> {
        Ok(self.items.lock().expect("the board").clone())
    }

    fn fetch(
        &self,
        _who: &Identity,
        _binding: &Binding,
        ids: &[RemoteItemId],
    ) -> Result<Vec<RemoteItem>> {
        Ok(self
            .items
            .lock()
            .expect("the board")
            .iter()
            .filter(|held| ids.contains(&held.id))
            .cloned()
            .collect())
    }

    fn create(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _draft: &RemoteDraft,
    ) -> Result<RemoteItem> {
        unimplemented!("nothing here creates at the remote")
    }

    /// **The write, kept.** The patch is applied to the board's own copy and the revision moves,
    /// so the next read sees what the push did — which is the only way a test can tell a push that
    /// happened from one that was planned.
    fn update(
        &self,
        _who: &Identity,
        _binding: &Binding,
        id: &RemoteItemId,
        patch: &RemotePatch,
        expect: &Revision,
    ) -> Result<RemoteItem> {
        let mut items = self.items.lock().expect("the board");
        let item = items
            .iter_mut()
            .find(|held| held.id == *id)
            .expect("the fixture has this card");
        if self.refuse || item.revision != *expect {
            bail!("this card changed at Trello since it was last read");
        }
        if let Some(lane) = &patch.lane {
            item.lane = lane.clone();
        }
        if let Some(title) = &patch.title {
            item.title = title.clone();
        }
        if let Some(body) = &patch.body {
            item.body = body.clone();
        }
        if let Some(labels) = &patch.labels {
            item.labels = labels.clone();
        }
        if let Some(assignees) = &patch.assignees {
            item.assignees = assignees.clone();
        }
        bump(item);
        Ok(item.clone())
    }

    fn comment(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _id: &RemoteItemId,
        _text: &str,
    ) -> Result<RemoteComment> {
        unimplemented!("comments are slice 7's")
    }

    fn set_check(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _id: &RemoteItemId,
        _item: &RemoteCheckId,
        _done: bool,
    ) -> Result<()> {
        unimplemented!("the checklist is slice 7's")
    }
}

fn json(raw: &str) -> Value {
    serde_json::from_str(raw).expect("a fixture is valid JSON")
}

/// Everything Trello really answers for, minus the assignee it will not take (`D188`).
fn trello_caps() -> ProviderCaps {
    ProviderCaps {
        write_lane: true,
        write_title: true,
        write_body: true,
        write_labels: true,
        write_assignee: false,
        comments_read: true,
        comments_write: true,
        checklist_read: true,
        checklist_write: true,
        conditional_write: false,
        server_query: false,
        hierarchy: false,
    }
}

fn who() -> Identity {
    Identity {
        provider: ProviderId::Trello,
        instance: None,
        account: "ada".to_string(),
        token: None,
        pin: None,
    }
}

fn plain() -> Binding {
    Binding::new(
        trello::ID,
        ConnectionId::generate(),
        RemoteContainerId::new(BOARD),
    )
}

/// A two-way binding over the settled lane map, auto-importing so the board arrives in one pass.
fn binding() -> Binding {
    let mut binding = plain();
    seed_lanes(&mut binding, &trello::parse_lists(&json(LISTS)));
    binding.auto_import = true;
    binding.direction = Direction::TwoWay;
    binding
}

fn work() -> Handle {
    Handle::new(Work::open(Box::new(MemoryTaskStore::new())))
}

fn tasks(work: &Handle, project: ProjectId) -> Vec<TaskRecord> {
    work.lock().tasks(project).1
}

fn task_by_title<'a>(tasks: &'a [TaskRecord], title: &str) -> &'a TaskRecord {
    tasks
        .iter()
        .find(|task| task.title == title)
        .unwrap_or_else(|| panic!("no task called {title:?}"))
}

fn link_of(file: &TasksrcFile, task: TaskId) -> &TaskLink {
    file.link(task).expect("the task has a link row")
}

fn run<'a>(provider: &'a dyn TaskProvider, work: &'a Handle, project: ProjectId) -> Runner<'a> {
    Runner {
        provider,
        work,
        project,
        who: who(),
    }
}

struct Runner<'a> {
    provider: &'a dyn TaskProvider,
    work: &'a Handle,
    project: ProjectId,
    who: Identity,
}

impl Runner<'_> {
    fn pass(&self, binding: &Binding, file: &mut TasksrcFile) -> Pass {
        self.reconcile()
            .run(binding, file, Utc::now())
            .expect("the board answers")
    }

    fn one(&self, binding: &Binding, file: &mut TasksrcFile, task: TaskId) -> Pass {
        self.reconcile()
            .one(binding, file, task, Utc::now())
            .expect("the board answers")
    }

    fn resolve(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        task: TaskId,
        side: DriftSide,
    ) -> Pass {
        self.reconcile()
            .resolve(binding, file, task, side, Utc::now())
            .expect("the board answers")
    }

    fn reconcile(&self) -> Reconcile<'_> {
        Reconcile {
            provider: self.provider,
            who: &self.who,
            work: self.work,
            project: self.project,
        }
    }
}

/// Import the board once, and hand back the task the tests work on and its remote id.
fn imported(
    runner: &Runner<'_>,
    binding: &Binding,
    file: &mut TasksrcFile,
) -> (TaskId, RemoteItemId) {
    runner.pass(binding, file);
    let task = task_by_title(&tasks(runner.work, runner.project), "Import dialog").id;
    let item = link_of(file, task).item.clone();
    (task, item)
}

// ── the four cells of the table ──────────────────────────────────────

/// **The task moved and the remote did not: push.** The board afterwards holds what the task
/// holds, in the fields the provider takes — and in none it does not.
#[test]
fn a_field_only_the_task_changed_is_written_to_the_remote() {
    let board = Board::new(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let binding = binding();
    let (task, item) = imported(&runner, &binding, &mut file);

    // The card starts in `Doing`. A person renames the task, rewrites its description and drags it
    // to `Done`.
    assert_eq!(board.card(&item).lane, RemoteLaneId::new(DOING));
    work.lock().update(
        project,
        task,
        Some("Import dialog — reviewed".to_string()),
        Some("now with a filter".to_string()),
        None,
    );
    work.lock().move_task(project, task, Status::Done, None);

    let ran = runner.pass(&binding, &mut file);

    assert_eq!(ran.pushed, vec![task], "the pass says it pushed");
    assert!(ran.conflicted.is_empty());
    let card = board.card(&item);
    assert_eq!(card.title, "Import dialog — reviewed");
    assert_eq!(card.body, "now with a filter");
    assert_eq!(card.lane, RemoteLaneId::new(DONE), "through the lane map");
    // Nothing is left over: a push that landed is not drift.
    assert!(link_of(&file, task).drift.is_empty());
    assert_eq!(link_of(&file, task).state, LinkState::Linked);

    // And the very next pass does nothing at all, which is what says the hashes were re-taken on
    // both sides rather than only on one.
    let again = runner.pass(&binding, &mut file);
    assert!(again.pushed.is_empty());
    assert!(again.updated.is_empty());
}

/// **`R13`.** A pull-only binding decides the same fields and sends nothing. It also flags nothing:
/// a local edit under pull-only is the user's to keep, and a badge on every edited card is a badge
/// nobody reads.
#[test]
fn a_pull_only_binding_sends_nothing_and_flags_nothing() {
    let board = Board::new(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let mut binding = binding();
    binding.direction = Direction::Pull;
    let (task, item) = imported(&runner, &binding, &mut file);

    work.lock()
        .update(project, task, Some("mine alone".to_string()), None, None);
    let ran = runner.pass(&binding, &mut file);

    assert!(ran.pushed.is_empty(), "nothing was sent");
    assert!(ran.drifted.is_empty(), "and nothing was flagged");
    assert_eq!(board.card(&item).title, "Import dialog");
    assert_eq!(link_of(&file, task).state, LinkState::Linked);
    // The task keeps what the person typed, which is the whole point of pull-only.
    assert_eq!(
        tasks(&work, project)
            .iter()
            .find(|held| held.id == task)
            .expect("still there")
            .title,
        "mine alone"
    );
}

/// **The remote moved and the task did not: pull.** `D185`'s half, unchanged by outbound.
#[test]
fn a_field_only_the_remote_changed_is_written_to_the_task() {
    let board = Board::new(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let binding = binding();
    let (task, item) = imported(&runner, &binding, &mut file);

    board.edited_at_the_remote(&item, |card| card.title = "Import dialog v2".to_string());
    let ran = runner.pass(&binding, &mut file);

    assert_eq!(ran.updated, vec![task]);
    assert!(ran.pushed.is_empty(), "nothing moved the other way");
    assert_eq!(
        tasks(&work, project)
            .iter()
            .find(|held| held.id == task)
            .expect("still there")
            .title,
        "Import dialog v2"
    );
    assert!(link_of(&file, task).drift.is_empty());
}

/// **Both sides moved: the switch decides, and the loser's value is kept in a comment.**
///
/// Run twice over the same starting state, once each way, because the *only* difference between
/// the two outcomes must be the switch.
#[test]
fn where_both_sides_changed_the_switch_settles_it_and_the_loser_is_kept_in_a_comment() {
    for authority in [Authority::RemoteWins, Authority::UbiqWins] {
        let board = Board::new(trello_caps());
        let work = work();
        let project = ProjectId::generate();
        let runner = run(&board, &work, project);
        let mut file = TasksrcFile::empty();
        let mut binding = binding();
        binding.authority = authority;
        let (task, item) = imported(&runner, &binding, &mut file);

        work.lock()
            .update(project, task, Some("mine".to_string()), None, None);
        board.edited_at_the_remote(&item, |card| card.title = "theirs".to_string());

        let ran = runner.pass(&binding, &mut file);
        assert_eq!(ran.drifted, vec![task], "{authority:?} still drifts");

        let held = tasks(&work, project);
        let record = held
            .iter()
            .find(|held| held.id == task)
            .expect("still there");
        let (winner, loser) = match authority {
            Authority::RemoteWins => ("theirs", "mine"),
            Authority::UbiqWins => ("mine", "theirs"),
        };
        assert_eq!(record.title, winner, "the task holds the winner");
        assert_eq!(
            board.card(&item).title,
            winner,
            "and so does the card — the switch settles one value, not two"
        );
        // The loser is not gone. `Nothing the layer does ever deletes anything` reaches here too:
        // a value the switch overrode is recoverable by reading.
        let said = record
            .comments
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(said.contains(loser), "the loser's value is kept: {said}");
        assert!(said.contains("title"), "and it says which field: {said}");

        // The card says so until somebody looks, and it says in what.
        let row = link_of(&file, task);
        assert_eq!(row.state, LinkState::Drifted);
        assert_eq!(row.drift.len(), 1);
        assert_eq!(row.drift[0].field, "title");
        assert_eq!(
            row.drift[0].settles,
            Some(match authority {
                Authority::RemoteWins => DriftSide::Pull,
                Authority::UbiqWins => DriftSide::Push,
            })
        );
    }
}

/// **A refused write is a `Conflict`, and nothing is overwritten on either side.**
///
/// The one rule a "just retry it" reading of `R8` would break: the whole reason the emulated
/// conditional write is stated as a *narrowed* window rather than a closed one is that the layer
/// must not paper over the refusal it does catch.
#[test]
fn a_refused_write_is_a_conflict_and_destroys_nothing() {
    let board = Board::refusing(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let binding = binding();
    let (task, item) = imported(&runner, &binding, &mut file);
    let before = board.card(&item);

    work.lock()
        .update(project, task, Some("mine, refused".to_string()), None, None);
    let ran = runner.pass(&binding, &mut file);

    assert_eq!(ran.conflicted, vec![task]);
    assert!(ran.pushed.is_empty());
    assert_eq!(link_of(&file, task).state, LinkState::Conflict);
    // Neither side lost anything.
    assert_eq!(board.card(&item).title, before.title);
    assert_eq!(
        tasks(&work, project)
            .iter()
            .find(|held| held.id == task)
            .expect("still there")
            .title,
        "mine, refused",
        "the local edit is untouched — a push failure destroys no local content"
    );
    // And it is not retried by overwriting: a second pass refuses again and still writes nothing.
    let again = runner.pass(&binding, &mut file);
    assert_eq!(again.conflicted, vec![task]);
    assert_eq!(board.card(&item).title, before.title);
}

/// **A capability the provider does not have is not attempted, and not silently dropped either.**
///
/// Trello takes a title and refuses an assignee (`D188` turned that flag off, because `update`
/// never sent one). The title lands; the assignee becomes a drift row that settles nowhere, which
/// is the honest shape of "no switch setting closes this".
#[test]
fn a_capability_the_provider_lacks_is_stated_rather_than_dropped() {
    let board = Board::new(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let binding = binding();
    let (task, item) = imported(&runner, &binding, &mut file);

    work.lock()
        .update(project, task, Some("taken".to_string()), None, None);
    work.lock().set_field(
        project,
        task,
        ubiq_proto::messages::TaskField::AssignedTo(Some("Grace Hopper".to_string())),
    );

    let ran = runner.pass(&binding, &mut file);

    assert_eq!(ran.pushed, vec![task], "the title went");
    assert_eq!(board.card(&item).title, "taken");
    assert_ne!(
        board.card(&item).assignees.first().map(String::as_str),
        Some("Grace Hopper"),
        "and the assignee did not"
    );

    let row = link_of(&file, task);
    let stuck: Vec<&str> = row
        .drift
        .iter()
        .filter(|drift| drift.settles.is_none())
        .map(|drift| drift.field.as_str())
        .collect();
    assert_eq!(stuck, ["assignees"], "said out loud, not dropped: {row:?}");
    assert!(
        row.drift[0].note.contains("assignee"),
        "with a reason: {}",
        row.drift[0].note
    );
    // The task keeps it. A write the remote will not take is not a reason to lose a local value.
    assert_eq!(
        tasks(&work, project)
            .iter()
            .find(|held| held.id == task)
            .expect("still there")
            .assigned_to
            .as_deref(),
        Some("Grace Hopper")
    );
}

// ── drift, and settling it by hand ───────────────────────────────────

/// **The force buttons.** A drifted task settled by hand goes the way the *person* said, not the
/// way the switch would have — and the row is cleared, because it has been answered.
#[test]
fn a_drifted_task_settles_the_way_a_person_says_whatever_the_switch_says() {
    for side in [DriftSide::Push, DriftSide::Pull] {
        let board = Board::new(trello_caps());
        let work = work();
        let project = ProjectId::generate();
        let runner = run(&board, &work, project);
        let mut file = TasksrcFile::empty();
        let mut binding = binding();
        // The switch says the remote wins. The person says otherwise, in one of the two runs.
        binding.authority = Authority::RemoteWins;
        let (task, item) = imported(&runner, &binding, &mut file);

        work.lock()
            .update(project, task, Some("mine".to_string()), None, None);
        board.edited_at_the_remote(&item, |card| card.title = "theirs".to_string());
        runner.pass(&binding, &mut file);
        assert_eq!(link_of(&file, task).state, LinkState::Drifted);

        // Both sides now read "theirs" — the switch settled it. The person disagrees and pushes,
        // or agrees and pulls.
        work.lock()
            .update(project, task, Some("mine, again".to_string()), None, None);
        runner.pass(&binding, &mut file);

        runner.resolve(&binding, &mut file, task, side);

        let title = tasks(&work, project)
            .iter()
            .find(|held| held.id == task)
            .expect("still there")
            .title
            .clone();
        match side {
            DriftSide::Push => {
                assert_eq!(title, "mine, again");
                assert_eq!(board.card(&item).title, "mine, again");
            }
            DriftSide::Pull => {
                assert_eq!(title, board.card(&item).title);
            }
        }
        assert!(
            link_of(&file, task).drift.is_empty(),
            "a settled row is answered and cleared"
        );
    }
}

/// **A single-item sync reads through `fetch`, so a task the filter no longer offers still syncs.**
///
/// That is the whole difference between [`Reconcile::one`] and a full pass, and the reason `fetch`
/// exists on the trait at all: a filter narrowed past a linked task is the user's opinion about
/// what the *board* is, not about whether this card may be synced on purpose.
#[test]
fn a_single_item_sync_reaches_a_task_the_filter_no_longer_offers() {
    let board = Board::new(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let binding = binding();
    let (task, item) = imported(&runner, &binding, &mut file);

    board.edited_at_the_remote(&item, |card| card.title = "moved on".to_string());
    let ran = runner.one(&binding, &mut file, task);

    assert_eq!(ran.updated, vec![task]);
    assert_eq!(
        tasks(&work, project)
            .iter()
            .find(|held| held.id == task)
            .expect("still there")
            .title,
        "moved on"
    );
    // And it unlinks nothing: only the *filter's* pass has an opinion about what belongs.
    assert!(ran.unlinked.is_empty());
    assert_eq!(link_of(&file, task).state, LinkState::Linked);
}

/// The two hash maps are what make the table work, and this is the proof: **a task that nobody
/// touched on either side produces nothing at all**, pass after pass.
///
/// Without a local map, every linked task would read as changed the moment the task differed from
/// the remote in any normalised way — the lane above all, because the lane map is many-to-one.
#[test]
fn a_pass_over_a_board_nobody_touched_decides_nothing() {
    let board = Board::new(trello_caps());
    let work = work();
    let project = ProjectId::generate();
    let runner = run(&board, &work, project);
    let mut file = TasksrcFile::empty();
    let binding = binding();
    runner.pass(&binding, &mut file);

    for _ in 0..3 {
        let ran = runner.pass(&binding, &mut file);
        assert!(ran.updated.is_empty(), "nothing pulled");
        assert!(ran.pushed.is_empty(), "nothing pushed");
        assert!(ran.drifted.is_empty(), "nothing drifted");
        assert!(ran.conflicted.is_empty(), "nothing refused");
    }
    let states: HashMap<TaskId, LinkState> =
        file.links.iter().map(|row| (row.task, row.state)).collect();
    assert!(
        states.values().all(|state| *state == LinkState::Linked),
        "every row is in step: {states:?}"
    );
}
