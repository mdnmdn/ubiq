//! The inbound sync pass, end to end, over the Trello fixtures.
//!
//! **The filter is what these tests run, not what they describe.** The provider below is the real
//! Trello client's parsing and filtering — `parse_cards` applies the binding's own filter, exactly
//! as it does against a live board — wired to recorded JSON instead of to the network. So every
//! assertion here is about the filter's *effect* on a project's board: which tasks exist, which
//! lane they landed in, and which link rows survived. A test that asserted the binding's
//! configuration instead would prove nothing at all.
//!
//! Two of these pin rules that belong to the **layer** rather than to Trello, and would have to
//! hold for any provider:
//!
//! - an unmapped lane parks its task rather than guessing a column, and
//! - an item that leaves the filter keeps its task, its content and its history, and loses only
//!   its link (`R9`). Nothing the filter does ever deletes anything.

use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use ubiq_host::store::memory::MemoryTaskStore;
use ubiq_host::tasksrc::store::{TasksrcFile, seed_lanes};
use ubiq_host::tasksrc::sync::{Pass, Reconcile};
use ubiq_host::tasksrc::trello::{self, FIELD_LABELS, FIELD_LISTS};
use ubiq_host::tasksrc::{
    Binding, Identity, LinkState, POLL_DEFAULT, POLL_FLOOR, TaskLink, TaskProvider,
};
use ubiq_host::work::{Handle, Work};
use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::{ConnectionId, ProjectId, TaskId};
use ubiq_proto::tasksrc::{
    Facets, ProviderCaps, RemoteCheckId, RemoteComment, RemoteContainer, RemoteContainerId,
    RemoteDraft, RemoteItem, RemoteItemId, RemoteLane, RemotePatch, Revision,
};
use ubiq_proto::work::{CommentAuthor, Kind, Status, TaskRecord};

const LISTS: &str = include_str!("fixtures/trello/lists.json");
const MEMBERS: &str = include_str!("fixtures/trello/members.json");
const CARDS: &str = include_str!("fixtures/trello/cards.json");
const ICEBOX: &str = include_str!("fixtures/trello/cards-icebox.json");

const BOARD: &str = "5f2a7b1c9d0e4a0012345601";
const TO_DO: &str = "6a1b2c3d4e5f60007000a101";
const DOING: &str = "6a1b2c3d4e5f60007000a102";
const BUG: &str = "7b2c3d4e5f60700080001201";

// ── the provider: the real Trello client, over recorded JSON ─────────

/// Trello's own parsing and filtering, served from a fixture instead of from the network.
///
/// Only [`TaskProvider::query`] is answered, because only `query` is what an inbound pass makes —
/// and it is answered by calling the client's own `parse_cards`, so the binding's filter runs for
/// real. Everything else is a write or a picker, which belongs to a later slice; leaving them
/// unimplemented is what keeps this from quietly becoming a second, agreeable Trello.
struct Recorded {
    cards: Value,
}

impl Recorded {
    fn new(raw: &str) -> Self {
        Self {
            cards: serde_json::from_str(raw).expect("a fixture is valid JSON"),
        }
    }
}

impl TaskProvider for Recorded {
    fn id(&self) -> &'static str {
        trello::ID
    }

    fn label(&self) -> &'static str {
        "Trello"
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps::default()
    }

    fn config_schema(&self) -> &'static [ubiq_proto::tasksrc::ConfigField] {
        &[]
    }

    fn containers(&self, _who: &Identity) -> Result<Vec<RemoteContainer>> {
        unimplemented!("an inbound pass picks no container")
    }

    fn lanes(&self, _who: &Identity, _binding: &Binding) -> Result<Vec<RemoteLane>> {
        Ok(trello::parse_lists(&json(LISTS)))
    }

    fn facets(&self, _who: &Identity, _binding: &Binding) -> Result<Facets> {
        unimplemented!("an inbound pass builds no picker")
    }

    /// **The filter runs here**, in the client's own code, against the binding it was given.
    fn query(&self, _who: &Identity, binding: &Binding) -> Result<Vec<RemoteItem>> {
        let members = trello::parse_members(&json(MEMBERS));
        let cards = self.cards.as_array().cloned().unwrap_or_default();
        let mut items = trello::parse_cards(&self.cards, binding);
        for item in &mut items {
            if let Some(card) = cards
                .iter()
                .find(|card| card.get("id").and_then(Value::as_str) == Some(item.id.as_str()))
            {
                trello::name_assignees(item, card, &members);
            }
        }
        Ok(items)
    }

    fn fetch(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _ids: &[RemoteItemId],
    ) -> Result<Vec<RemoteItem>> {
        unimplemented!("an inbound pass reads the filter, never the linked set")
    }

    fn create(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _draft: &RemoteDraft,
    ) -> Result<RemoteItem> {
        unimplemented!("outbound is a later slice")
    }

    fn update(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _id: &RemoteItemId,
        _patch: &RemotePatch,
        _expect: &Revision,
    ) -> Result<RemoteItem> {
        unimplemented!("outbound is a later slice")
    }

    fn comment(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _id: &RemoteItemId,
        _text: &str,
    ) -> Result<RemoteComment> {
        unimplemented!("outbound is a later slice")
    }

    fn set_check(
        &self,
        _who: &Identity,
        _binding: &Binding,
        _id: &RemoteItemId,
        _item: &RemoteCheckId,
        _done: bool,
    ) -> Result<()> {
        unimplemented!("outbound is a later slice")
    }
}

fn json(raw: &str) -> Value {
    serde_json::from_str(raw).expect("a fixture is valid JSON")
}

// ── the board under the pass ─────────────────────────────────────────

fn who() -> Identity {
    Identity {
        provider: ProviderId::Trello,
        instance: None,
        account: "ada".to_string(),
        // The fixture provider makes no request, so there is nothing to authenticate with — which
        // is also the point of testing the pass against recorded answers.
        token: None,
        pin: None,
    }
}

/// A binding with the settled default lane map: `To Do`, `Doing` and `Done` bound, the other four
/// Ubiq lanes and every other list left unbound on purpose.
fn binding() -> Binding {
    let mut binding = Binding::new(
        trello::ID,
        ConnectionId::generate(),
        RemoteContainerId::new(BOARD),
    );
    seed_lanes(&mut binding, &trello::parse_lists(&json(LISTS)));
    // Import is explicit (`R12`); these tests turn the switch the user would turn, because what is
    // under test is the filter and not the dialog.
    binding.auto_import = true;
    binding
}

fn work() -> Handle {
    Handle::new(Work::open(Box::new(MemoryTaskStore::new())))
}

fn tasks(work: &Handle, project: ProjectId) -> Vec<TaskRecord> {
    work.lock().tasks(project).1
}

fn titles(tasks: &[TaskRecord]) -> Vec<&str> {
    tasks.iter().map(|task| task.title.as_str()).collect()
}

fn task_by_title<'a>(tasks: &'a [TaskRecord], title: &str) -> &'a TaskRecord {
    tasks
        .iter()
        .find(|task| task.title == title)
        .unwrap_or_else(|| panic!("no task called {title:?} among {:?}", titles(tasks)))
}

fn link_of(file: &TasksrcFile, task: TaskId) -> &TaskLink {
    file.link(task).expect("the task has a link row")
}

/// One pass, over one provider, against one project.
fn pass(
    provider: &dyn TaskProvider,
    work: &Handle,
    project: ProjectId,
    binding: &Binding,
    file: &mut TasksrcFile,
) -> Pass {
    let who = who();
    Reconcile {
        provider,
        who: &who,
        work,
        project,
    }
    .run(binding, file, Utc::now())
    .expect("the fixture provider answers")
}

// ── the filter, by its effect ────────────────────────────────────────

/// The whole of M6 in one test: a filter narrows what the board fetches, and what the board
/// fetches is what lands on the project's tasks.
///
/// The assertion is on the **tasks that exist**, not on the binding — the filter ran or these
/// three cards would all be here.
#[test]
fn the_filter_is_what_the_pass_runs_and_the_board_is_what_it_produces() {
    let provider = Recorded::new(CARDS);
    let work = work();
    let project = ProjectId::generate();
    let mut file = TasksrcFile::empty();

    // One binding, filtered two ways — a project has one, and the link table is keyed by its id.
    let bound = binding();

    // Narrowed to one label: of the board's three open cards, one carries `bug`.
    let mut narrowed = bound.clone();
    narrowed.filter.set(FIELD_LABELS, vec![BUG.to_string()]);
    let ran = pass(&provider, &work, project, &narrowed, &mut file);

    assert_eq!(ran.created.len(), 1);
    assert_eq!(ran.unlinked, Vec::<TaskId>::new());
    let held = tasks(&work, project);
    assert_eq!(titles(&held), ["Pane resize corrupts the alternate screen"]);

    // Everything the card said, on the task: the lane through the map, the key, the link, the
    // labels, the assignee and the body exactly as Trello's markdown gave it.
    let task = &held[0];
    assert_eq!(task.status, Status::Backlog, "`To Do` maps to Backlog");
    assert_eq!(task.key.as_deref(), Some("#41"));
    assert_eq!(task.link.as_deref(), Some("https://trello.com/c/Ab12Cd34"));
    assert_eq!(
        task.labels
            .iter()
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>(),
        ["bug"]
    );
    assert_eq!(task.assigned_to.as_deref(), Some("Ada Lovelace"));
    assert!(task.description.contains("**old**"));
    assert_eq!(link_of(&file, task.id).state, LinkState::Linked);

    // Widen the filter and the same pass brings the rest — which is the filter's effect and
    // nothing else, because nothing about the board or the provider changed.
    let ran = pass(&provider, &work, project, &bound, &mut file);
    assert_eq!(ran.created.len(), 2);
    let held = tasks(&work, project);
    assert_eq!(held.len(), 3, "and never the archived card");
    assert_eq!(
        task_by_title(&held, "Import dialog").status,
        Status::InProgress
    );
    assert_eq!(task_by_title(&held, "Ship the beta").status, Status::Done);
}

/// A lane the map does not name **parks** the task: it is created where a new task goes and left
/// there, and the link row says why. A wrong guess would move somebody's card silently, which is
/// the failure this rule exists to prevent — the settled Trello map leaves four Ubiq lanes and
/// every non-default list unbound on purpose.
#[test]
fn an_unmapped_lane_parks_the_task_with_a_badge_rather_than_guessing_a_column() {
    let provider = Recorded::new(ICEBOX);
    let work = work();
    let project = ProjectId::generate();
    let mut file = TasksrcFile::empty();
    let binding = binding();

    // The board really does have this list, and the seeded map really does leave it out.
    assert!(
        binding
            .lanes
            .keys()
            .all(|lane| lane != "6a1b2c3d4e5f60007000a104"),
        "Icebox is not one of the three lists everybody has"
    );

    let ran = pass(&provider, &work, project, &binding, &mut file);

    let held = tasks(&work, project);
    assert_eq!(titles(&held), ["Someday: a second provider"]);
    let task = &held[0];
    assert_eq!(ran.parked, vec![task.id], "the pass says it parked it");
    assert_eq!(
        link_of(&file, task.id).state,
        LinkState::Parked,
        "the badge is the link row's state"
    );
    // Parked means *where it already was*, which for a new task is the column a new task lands in.
    assert_eq!(task.status, Status::Backlog);

    // And the lane's hash is deliberately not filed, so binding that list later moves the task at
    // once rather than waiting for somebody to touch the card at the remote.
    assert!(
        !link_of(&file, task.id).hash.contains_key("lane"),
        "a parked lane is re-asked on the next pass"
    );

    // Bind the lane, and the very next pass moves it — nothing at the remote changed.
    let mut bound = binding.clone();
    bound
        .lanes
        .insert("6a1b2c3d4e5f60007000a104".to_string(), Status::Blocked);
    let ran = pass(&provider, &work, project, &bound, &mut file);
    assert!(ran.parked.is_empty());
    assert_eq!(tasks(&work, project)[0].status, Status::Blocked);
    assert_eq!(link_of(&file, task.id).state, LinkState::Linked);
}

/// `R9`. **Nothing the filter does ever deletes anything.** An item the user narrowed away keeps
/// its task, its content, its comments and its steps, and loses only its link.
///
/// This is the rule most likely to be got wrong by treating "no longer in the fetch" as "gone".
#[test]
fn an_item_that_leaves_the_filter_keeps_its_task_its_content_and_its_history() {
    let provider = Recorded::new(CARDS);
    let work = work();
    let project = ProjectId::generate();
    let mut file = TasksrcFile::empty();

    let bound = binding();
    pass(&provider, &work, project, &bound, &mut file);
    let held = tasks(&work, project);
    assert_eq!(held.len(), 3);
    let leaving = task_by_title(&held, "Ship the beta").id;

    // A person does some work on the task: a comment and a step of their own, which the remote
    // knows nothing about.
    {
        let mut held = work.lock();
        held.add_comment(
            project,
            leaving,
            CommentAuthor::User,
            "blocked on the installer".to_string(),
        );
        held.add_step(project, leaving, "sign the build".to_string());
    }

    // The filter narrows to two lists, and `Done` is not one of them.
    let mut narrowed = bound.clone();
    narrowed
        .filter
        .set(FIELD_LISTS, vec![TO_DO.to_string(), DOING.to_string()]);
    let ran = pass(&provider, &work, project, &narrowed, &mut file);

    assert_eq!(ran.unlinked, vec![leaving]);
    assert!(ran.created.is_empty(), "the other two are already linked");

    // The task is still there, whole.
    let held = tasks(&work, project);
    assert_eq!(held.len(), 3, "nothing was deleted");
    let kept = held
        .iter()
        .find(|task| task.id == leaving)
        .expect("the task survived losing its link");
    assert_eq!(kept.title, "Ship the beta");
    assert_eq!(kept.status, Status::Done, "its column is untouched too");
    assert_eq!(kept.comments.len(), 1);
    assert_eq!(kept.comments[0].text, "blocked on the installer");
    assert_eq!(kept.steps.len(), 1);
    assert_eq!(kept.steps[0].title, "sign the build");
    assert_eq!(kept.key.as_deref(), Some("#43"));

    // Only the link moved, and the row is kept rather than dropped — an overview needs something
    // to offer a bulk act on (`R9`'s stated cost).
    assert_eq!(link_of(&file, leaving).state, LinkState::Unlinked);

    // And the pass says it once: a second pass over the same narrow filter reports nothing new.
    let again = pass(&provider, &work, project, &narrowed, &mut file);
    assert!(again.unlinked.is_empty());
    assert_eq!(tasks(&work, project).len(), 3);
}

/// Pull only, which is what `Direction::Pull` means: a field the remote did not change is left
/// exactly as the person left it, however they edited it. This is the ground the conflict rules
/// of a later slice stand on — without it every pass would overwrite every local edit.
#[test]
fn a_local_edit_survives_a_pass_the_remote_did_not_change_anything_in() {
    let provider = Recorded::new(CARDS);
    let work = work();
    let project = ProjectId::generate();
    let mut file = TasksrcFile::empty();
    let binding = binding();

    pass(&provider, &work, project, &binding, &mut file);
    let task = task_by_title(&tasks(&work, project), "Import dialog").id;

    work.lock().update(
        project,
        task,
        Some("Import dialog — mine now".to_string()),
        None,
        None,
    );
    work.lock().move_task(project, task, Status::InReview, None);

    let ran = pass(&provider, &work, project, &binding, &mut file);
    assert!(ran.updated.is_empty(), "the remote changed nothing");

    let held = tasks(&work, project);
    let task = held
        .iter()
        .find(|held| held.id == task)
        .expect("still there");
    assert_eq!(task.title, "Import dialog — mine now");
    assert_eq!(task.status, Status::InReview);
}

/// The kind map is a string the *provider* supplied keyed onto Ubiq's closed set (`R5`). Trello
/// has no type field at all, so the string is a label name — and the same map would take a
/// work-item tracker's `kind_hint` without the layer learning which it is holding.
#[test]
fn the_kind_map_reads_a_label_where_the_tracker_has_no_type_field() {
    let provider = Recorded::new(CARDS);
    let work = work();
    let project = ProjectId::generate();
    let mut file = TasksrcFile::empty();

    let mut mapped = binding();
    mapped.kinds.insert("bug".to_string(), Kind::Bug);
    pass(&provider, &work, project, &mapped, &mut file);

    let held = tasks(&work, project);
    assert_eq!(
        task_by_title(&held, "Pane resize corrupts the alternate screen").kind,
        Some(Kind::Bug)
    );
    // A card carrying no mapped label is left unkinded rather than given a default.
    assert_eq!(task_by_title(&held, "Ship the beta").kind, None);
}

/// `R14`. The interval is per binding, five minutes by default, with a floor applied **on read**
/// — and a pass never writes the floor back over what the user asked for, because a later build
/// with a lower floor must not find the value rewritten.
#[test]
fn the_poll_interval_has_a_floor_on_read_and_a_pass_never_writes_it_back() {
    let provider = Recorded::new(CARDS);
    let work = work();
    let project = ProjectId::generate();
    let mut file = TasksrcFile::empty();

    let mut impatient = binding();
    assert_eq!(impatient.poll, POLL_DEFAULT, "five minutes by default");
    impatient.poll = 5;
    assert_eq!(impatient.poll_seconds(), POLL_FLOOR);

    pass(&provider, &work, project, &impatient, &mut file);

    // The pass writes link rows and never the binding, so what the user asked for is still there
    // when the sidecar is saved.
    file.bindings = vec![impatient.clone()];
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("tasksrc.toml");
    ubiq_host::tasksrc::store::save(&path, &file).expect("it writes");
    let loaded = ubiq_host::tasksrc::store::load(&path);
    assert_eq!(loaded.only_binding().expect("a binding").poll, 5);
    assert_eq!(loaded.only_binding().expect("a binding").poll_seconds(), 60);
}
