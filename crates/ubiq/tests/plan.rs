//! The plan modal's app-level wiring: opening a task's plan asks the host for its body and its
//! annotations together, `Message::Plan` and `Message::PlanAnnotations` fill them in, and
//! `Message::PlanChanged` / `Message::PlanAnnotationsChanged` re-ask rather than being trusted to
//! carry content.
//!
//! `state::plan` has no arithmetic of its own worth testing on the state alone — so what is
//! tested here is `AppState`'s wiring: that opening a plan sends exactly the family's two reads,
//! that every reply lands on the viewer it belongs to and nowhere else (`kb.rs`'s own shape for
//! the same question about a document), and that the composer sends the right verb for what it
//! is answering.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use gpui_component::input::CompletionProvider as _;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::document::{
    AnnotationsBody, ComposerTarget, DocumentBody, Notice, RowDecor, SLASH_COMMANDS,
    slash_commands, slash_prefix,
};
use ubiq::ui::editor::SlashCommands;
use ubiq::ui::mdview::blockedit;
use ubiq::ui::mdview::view::MdView;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{
    Annotation, AnnotationMark, AnnotationState, BlockHighlight, DocBinding, DocDelivery,
    DocThreadRef, DocumentHandle, HighlightColour, PlanBlock, PlanChangeStats, PlanChangedRegion, SaveOrigin,
};
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{
    Activity, Addressee, AgentId, CommentAuthor, Level, Priority, Status, TaskRecord, WorkAgent,
    WorkSession,
};

const PATIENCE: Duration = Duration::from_millis(500);

/// A task's plan, as the whole family names it: one handle, both directions.
fn doc_of(project: ProjectId, task: TaskId) -> DocumentHandle {
    DocumentHandle::Plan {
        project_id: project,
        task_id: task,
    }
}

/// A mission document (M8), the same way.
fn mission_doc_of(project: ProjectId, task: TaskId, name: &str) -> DocumentHandle {
    DocumentHandle::MissionDoc {
        project_id: project,
        task_id: task,
        name: name.to_string(),
    }
}

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            window,
            host,
            project,
        }
    }

    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>) -> R,
    ) -> R {
        let out = self
            .window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| f(state, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        out
    }

    fn seed_task(&self, level: Option<Level>, cx: &mut TestAppContext) -> TaskId {
        let id = TaskId::generate();
        self.deliver(
            Message::WorkList {
                project_id: self.project,
                sessions: Vec::new(),
                agents: Vec::new(),
                tasks: vec![a_task(id, level)],
            },
            cx,
        );
        id
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn a_task(id: TaskId, level: Option<Level>) -> TaskRecord {
    TaskRecord {
        id,
        session: None,
        status: Status::Backlog,
        priority: Priority::Normal,
        shape: None,
        kind: None,
        level,
        parent: None,
        references: Vec::new(),
        prerequisites: Vec::new(),
        attachments: Vec::new(),
        complexity: None,
        key: None,
        link: None,
        assigned_to: None,
        labels: Vec::new(),
        colour: None,
        title: "Ship the thing".to_string(),
        description: String::new(),
        steps: Vec::new(),
        comments: Vec::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// One run of changed lines, in the numbering a reader sees in the gutter.
fn a_region(first: u32, last: u32, origin: SaveOrigin) -> PlanChangedRegion {
    PlanChangedRegion {
        first_line: first,
        last_line: last,
        revision: 4,
        origin,
        block_id: None,
        text: String::new(),
    }
}

/// Opening a levelled task's plan asks the host for the body and for its annotation panel in the
/// same breath, and shows the modal as loading both — what the affordance in `ui::board::detail`
/// relies on to decide whether it has anything to draw, and what the annotation panel relies on
/// to show its own "reading" state until `PlanAnnotations` lands.
#[gpui::test]
fn open_plan_asks_and_loads(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said(); // drain the `WorkList` fixture noise

    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));

    let said = fixture.said();
    assert_eq!(said.len(), 2);
    assert!(said.iter().any(|message| matches!(
        message,
        Message::LoadPlan { doc }
            if *doc == doc_of(fixture.project, task)
    )));
    assert!(said.iter().any(|message| matches!(
        message,
        Message::ListPlanAnnotations { doc, .. }
            if *doc == doc_of(fixture.project, task)
    )));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("the modal is open");
        assert_eq!(plan.task_id(), Some(task));
        assert!(matches!(plan.body, DocumentBody::Loading));
        assert!(matches!(plan.annotations, AnnotationsBody::Loading));
    });

    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "# The plan".to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(plan.body, DocumentBody::Loaded(ref body) if body == "# The plan"));
    });
}

/// Opening a mission document asks the host for its body and its annotations exactly the way
/// opening a plan does — `open_mission_doc`'s whole job is naming a different `DocumentHandle`,
/// on the wire the plan already rides (M8).
#[gpui::test]
fn open_mission_doc_asks_and_loads(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said(); // drain the `WorkList` fixture noise

    fixture.with(cx, |state, _, cx| state.open_mission_doc(task, "notes", cx));

    let said = fixture.said();
    assert_eq!(said.len(), 2);
    let handle = mission_doc_of(fixture.project, task, "notes");
    assert!(
        said.iter()
            .any(|message| matches!(message, Message::LoadPlan { doc } if *doc == handle))
    );
    assert!(
        said.iter().any(
            |message| matches!(message, Message::ListPlanAnnotations { doc, .. } if *doc == handle)
        )
    );
    fixture.state.read_with(cx, |state, _| {
        let doc = state.workbench.plan.as_ref().expect("the modal is open");
        assert_eq!(doc.doc, handle);
        assert!(doc.is_modal(), "opened on the plan dialog's own footing");
    });

    fixture.deliver(
        Message::Plan {
            doc: handle.clone(),
            body: "# Notes".to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let doc = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(doc.body, DocumentBody::Loaded(ref body) if body == "# Notes"));
    });
}

/// `PlanChanged` carries no body — another window's save, or an agent's `write_plan` — so the
/// window holding that plan open re-asks rather than being handed content it never received.
#[gpui::test]
fn plan_changed_reasks(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    fixture.deliver(
        Message::PlanChanged {
            doc: doc_of(fixture.project, task),
            revision: 2,
            origin: SaveOrigin::Agent,
        },
        cx,
    );

    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::LoadPlan { doc }
            if *doc == doc_of(fixture.project, task)
    ));

    // A `PlanChanged` for a task whose plan is not open asks for nothing: nobody is waiting on
    // it.
    fixture.with(cx, |state, _, cx| state.close_plan(cx));
    fixture.deliver(
        Message::PlanChanged {
            doc: doc_of(fixture.project, task),
            revision: 2,
            origin: SaveOrigin::Agent,
        },
        cx,
    );
    assert!(fixture.said().is_empty());
}

/// A plan deleted out from under an open modal reads as "no plan yet", not an empty pane left
/// drawing stale content.
#[gpui::test]
fn plan_deleted_clears_the_body(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "# The plan".to_string(),
            revision: 1,
        },
        cx,
    );

    fixture.deliver(
        Message::PlanDeleted {
            doc: doc_of(fixture.project, task),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(plan.body, DocumentBody::Loaded(ref body) if body.is_empty()));
    });
}

/// A failure while the modal is still waiting on its first load is the load's own failure; the
/// same message arriving once a body is already showing is an export's, because nothing else in
/// this slice can fail once the plan and its annotations are both on screen.
#[gpui::test]
fn plan_error_lands_on_load_then_on_export(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));

    fixture.deliver(
        Message::PlanError {
            project_id: fixture.project,
            doc: Some(doc_of(fixture.project, task)),
            error: "no such task".to_string(),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(plan.body, DocumentBody::Failed(ref reason) if reason == "no such task"));
    });

    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "# The plan".to_string(),
            revision: 1,
        },
        cx,
    );
    // The annotation panel has to have a body of its own — `Loaded` or `Failed` — before a third
    // `PlanError` can be read as the export's: while either half is still `Loading`, the error is
    // claimed by that half first.
    fixture.deliver(
        Message::PlanAnnotations {
            doc: doc_of(fixture.project, task),
            blocks: Vec::new(),
            annotations: Vec::new(),
            highlights: Vec::new(),
            binding: Default::default(),
        },
        cx,
    );
    fixture.deliver(
        Message::PlanError {
            project_id: fixture.project,
            doc: Some(doc_of(fixture.project, task)),
            error: "the path is outside the project".to_string(),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(plan.body, DocumentBody::Loaded(_)));
        assert!(
            matches!(plan.notice, Some(Notice::Err(ref reason)) if reason == "the path is outside the project")
        );
    });
}

/// `close_plan` is the whole of what the modal's own Close and Escape do.
#[gpui::test]
fn close_plan_clears_the_modal(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.with(cx, |state, _, cx| state.close_plan(cx));
    fixture
        .state
        .read_with(cx, |state, _| assert!(state.workbench.plan.is_none()));
}

// ── The annotation panel (slice 4) ──────────────────────────────────────────

/// The answer to `ListPlanAnnotations` — the block index and every thread on it — lands on the
/// viewer that asked, `Message::Plan`'s own discipline.
#[gpui::test]
fn plan_annotations_land_on_the_open_viewer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let block = PlanBlock {
        id: BlockId::generate(),
        kind: "paragraph".to_string(),
        text: "A passage.".to_string(),
        lineage: String::new(),
    };
    let annotation = Annotation::new(
        block.id,
        None,
        CommentAuthor::User,
        "Why this?".to_string(),
        Utc::now(),
    );

    fixture.deliver(
        Message::PlanAnnotations {
            doc: doc_of(fixture.project, task),
            blocks: vec![block.clone()],
            annotations: vec![annotation.clone()],
            highlights: Vec::new(),
            binding: Default::default(),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        match &plan.annotations {
            AnnotationsBody::Loaded {
                blocks,
                annotations,
                ..
            } => {
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].id, block.id);
                assert_eq!(annotations.len(), 1);
                assert_eq!(annotations[0].id, annotation.id);
            }
            _ => panic!("expected the annotations to have loaded"),
        }
    });
}

/// `PlanAnnotationsChanged` carries nothing, the same as `PlanChanged` — a window with that
/// plan's panel open re-asks, and one with it closed asks for nothing.
#[gpui::test]
fn plan_annotations_changed_reasks(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    fixture.deliver(
        Message::PlanAnnotationsChanged {
            doc: doc_of(fixture.project, task),
        },
        cx,
    );
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::ListPlanAnnotations { doc, .. }
            if *doc == doc_of(fixture.project, task)
    ));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(plan.annotations, AnnotationsBody::Loading));
    });

    fixture.with(cx, |state, _, cx| state.close_plan(cx));
    fixture.deliver(
        Message::PlanAnnotationsChanged {
            doc: doc_of(fixture.project, task),
        },
        cx,
    );
    assert!(fixture.said().is_empty());
}

/// Claiming a block opens the composer pointed at it; submitting sends `AnnotatePlan` with no
/// quote — block granularity is what this slice draws from the rendered view — and clears the
/// field.
#[gpui::test]
fn compose_annotation_sends_and_clears(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let block_id = BlockId::generate();
    fixture.with(cx, |state, window, cx| {
        state.compose_annotation(block_id, window, cx)
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.composer, Some(ComposerTarget::Block(block_id)));
    });

    fixture.with(cx, |state, _, _| {
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "Why here?".to_string();
        }
    });
    fixture.with(cx, |state, window, cx| {
        state.submit_annotation_composer(window, cx)
    });

    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::AnnotatePlan { doc, block_id: b, quote: None, text, .. }
            if *doc == doc_of(fixture.project, task) && *b == block_id && text == "Why here?"
    ));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.composer.is_none());
        assert!(plan.composer_text.is_empty());
    });
}

/// Replying opens the composer pointed at the thread instead of a block, and submitting sends
/// `ReplyToAnnotation`.
#[gpui::test]
fn compose_reply_sends_and_clears(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let annotation_id = AnnotationId::generate();
    fixture.with(cx, |state, window, cx| {
        state.compose_reply(annotation_id, window, cx)
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.composer, Some(ComposerTarget::Reply(annotation_id)));
    });

    fixture.with(cx, |state, _, _| {
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "Agreed.".to_string();
        }
    });
    fixture.with(cx, |state, window, cx| {
        state.submit_annotation_composer(window, cx)
    });

    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::ReplyToAnnotation { doc, annotation_id: a, text, .. }
            if *doc == doc_of(fixture.project, task) && *a == annotation_id && text == "Agreed."
    ));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.composer.is_none());
    });
}

/// Cancelling the composer sends nothing at all — it is a change of mind, not an empty comment.
#[gpui::test]
fn cancel_annotation_composer_sends_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let block_id = BlockId::generate();
    fixture.with(cx, |state, window, cx| {
        state.compose_annotation(block_id, window, cx)
    });
    fixture.with(cx, |state, _, _| {
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "never mind".to_string();
        }
    });
    fixture.with(cx, |state, window, cx| {
        state.cancel_annotation_composer(window, cx)
    });

    assert!(fixture.said().is_empty());
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.composer.is_none());
    });
}

/// The minimap's show/hide toggle is the one bit of persistent state the card calls for — it
/// writes the Ui settings layer the same way every other reading habit on this layer does.
#[gpui::test]
fn toggling_the_minimap_writes_ui_settings(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.state.read_with(cx, |state, _| {
        assert!(
            state.workbench.settings.ui.md_minimap,
            "shown is the default"
        );
    });
    fixture.said();

    fixture.with(cx, |state, _, cx| state.toggle_md_minimap(cx));
    fixture.state.read_with(cx, |state, _| {
        assert!(!state.workbench.settings.ui.md_minimap);
    });
    assert!(
        fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::SetSettings { layer, .. } if *layer == ubiq_proto::settings::SettingsLayer::Ui)),
        "the toggle is remembered",
    );

    fixture.with(cx, |state, _, cx| state.toggle_md_minimap(cx));
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.settings.ui.md_minimap);
    });
}

/// Resolving and reopening are the same verb both ways — no author check on this side either:
/// "who may resolve" is settled at "anyone", so there is nothing here to gate.
#[gpui::test]
fn resolve_and_reopen_send_resolve_annotation(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let annotation_id = AnnotationId::generate();
    fixture.with(cx, |state, _, cx| {
        state.set_annotation_resolved(annotation_id, true, cx)
    });
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::ResolveAnnotation { doc, annotation_id: a, resolved: true }
            if *doc == doc_of(fixture.project, task) && *a == annotation_id
    ));

    fixture.with(cx, |state, _, cx| {
        state.set_annotation_resolved(annotation_id, false, cx)
    });
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::ResolveAnnotation {
            resolved: false,
            ..
        }
    ));
}

/// `orphaned` and `state` are two independent facts, not one status folded together: a resolved
/// thread whose block vanished stays flagged both ways, and neither field is derived from the
/// other on the way in.
#[gpui::test]
fn orphaned_and_resolved_are_independent_facts(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let vanished_block = BlockId::generate();
    let mut annotation = Annotation::new(
        vanished_block,
        None,
        CommentAuthor::User,
        "Still true?".to_string(),
        Utc::now(),
    );
    annotation.orphaned = true;
    annotation.state = AnnotationState::Resolved;

    fixture.deliver(
        Message::PlanAnnotations {
            doc: doc_of(fixture.project, task),
            // The block itself is gone from the index — that absence is what "orphaned" means —
            // but the flag on the annotation is what says so, not a lookup miss the interface
            // would have to reconstruct.
            blocks: Vec::new(),
            annotations: vec![annotation.clone()],
            highlights: Vec::new(),
            binding: Default::default(),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        let landed = &plan.annotations.annotations()[0];
        assert!(landed.orphaned, "orphaned must survive the wire, verbatim");
        assert_eq!(
            landed.state,
            AnnotationState::Resolved,
            "and state is its own fact, independent of it"
        );
    });
}

// ── The editor (slice 6) ────────────────────────────────────────────────────

/// The whole of the writing loop, end to end: the host's body reaches the buffer, an edit makes
/// the surface dirty, ⌘S sends `SavePlan` with what was typed — this is that message's first
/// caller in the tree — and the host's answer settles the surface clean again.
#[gpui::test]
fn edit_and_save_sends_save_plan(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "# The plan\n".to_string(),
            revision: 1,
        },
        cx,
    );
    // The buffer is seeded in the frame after the answer, where there is a `Window` to write one
    // with — `attach_arrived_files`' own arrangement.
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.state.read_with(cx, |state, cx| {
        assert_eq!(
            state.plan_editor.read(cx).value().to_string(),
            "# The plan\n"
        );
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(!plan.dirty, "a freshly seeded buffer is not an edit");
    });

    // Drain the `ListPlanChanges` the arriving body asks for — provenance rides with every load.
    fixture.said();

    // A clean surface saves nothing: ⌘S over an unchanged document is not a write.
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    assert!(fixture.said().is_empty());

    fixture.with(cx, |state, window, cx| {
        let editor = state.plan_editor.clone();
        editor.update(cx, |buffer, cx| {
            buffer.set_value("# The plan\n\nFirst step.\n", window, cx)
        });
        // A programmatic write raises no change event; the frame is where the surface reads the
        // buffer back. Typing reaches the same place through the subscription.
        state.settle_plan_editor(window, cx);
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.dirty, "typing is what makes the surface dirty");
    });

    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::SavePlan { doc, body, expected }
            if *doc == doc_of(fixture.project, task)
                && body == "# The plan\n\nFirst step.\n"
                // An ordinary save names the revision the buffer was seeded from, which is what
                // the host checks the plan still stands at.
                && *expected == 1
    ));

    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "# The plan\n\nFirst step.\n".to_string(),
            revision: 2,
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(!plan.dirty, "the host's answer is what settles the surface");
        assert!(!plan.saving);
        assert!(!plan.stale);
    });
}

/// A plan changed elsewhere while it was being edited never overwrites the edit. The surface
/// says its copy moved and keeps what was typed — the one thing an editor may not do is throw an
/// unsaved change away to win a race.
#[gpui::test]
fn a_remote_change_never_eats_an_unsaved_edit(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "original\n".to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.with(cx, |state, window, cx| {
        let editor = state.plan_editor.clone();
        editor.update(cx, |buffer, cx| buffer.set_value("mine\n", window, cx));
        state.settle_plan_editor(window, cx);
    });
    fixture.said();

    // An agent's `write_plan`, echoed back as the reload the window asks for itself.
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "theirs\n".to_string(),
            revision: 2,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.state.read_with(cx, |state, cx| {
        assert_eq!(state.plan_editor.read(cx).value().to_string(), "mine\n");
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.stale, "the surface says the other copy moved");
        assert!(plan.dirty);
    });
}

/// A plan's blocks and threads, loaded into an open dialog over `body`: the body seeded into the
/// buffer (which the view reparses), then the block index the host would send for it.
fn loaded_plan(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    body: &str,
    blocks: Vec<PlanBlock>,
    annotations: Vec<Annotation>,
    highlights: Vec<BlockHighlight>,
) -> TaskId {
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: body.to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.deliver(
        Message::PlanAnnotations {
            doc: doc_of(fixture.project, task),
            blocks,
            annotations,
            highlights,
            binding: Default::default(),
        },
        cx,
    );
    fixture.said();
    task
}

fn paragraph(text: &str) -> PlanBlock {
    PlanBlock {
        id: BlockId::generate(),
        kind: "paragraph".to_string(),
        text: text.to_string(),
        lineage: String::new(),
    }
}

fn thread_on(block: &PlanBlock, text: &str) -> Annotation {
    Annotation::new(
        block.id,
        None,
        CommentAuthor::User,
        text.to_string(),
        Utc::now(),
    )
}

/// The open document's view.
fn md_of(fixture: &Fixture, cx: &mut TestAppContext) -> Entity<MdView> {
    fixture.state.read_with(cx, |state, _| {
        state
            .workbench
            .plan
            .as_ref()
            .expect("a document is open")
            .md
            .clone()
    })
}

/// The threads, their marks and the highlights reach the view's margin as one `RowDecor` per row,
/// placed by the join between the host's block ids and the view's rows. Clicking a row's count
/// marker focuses its thread, and the decor says so.
#[gpui::test]
fn annotations_decorate_their_rows(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let body = "# The plan\n\nShip the thing by Friday.\n\nThen tell everyone.\n";
    let heading = PlanBlock {
        id: BlockId::generate(),
        kind: "heading:1".to_string(),
        text: "# The plan".to_string(),
        lineage: String::new(),
    };
    let step = paragraph("Ship the thing by Friday.");
    let mut open = thread_on(&step, "is that real?");
    open.marks = vec![AnnotationMark::Question];
    let mut settled = thread_on(&heading, "renamed?");
    settled.state = AnnotationState::Resolved;
    loaded_plan(
        &fixture,
        cx,
        body,
        vec![heading.clone(), step.clone()],
        vec![open.clone(), settled.clone()],
        vec![BlockHighlight {
            block_id: step.id,
            colour: HighlightColour::Yellow,
        }],
    );

    let md = md_of(&fixture, cx);
    md.read_with(cx, |view, _| {
        assert!(view.annotating(), "the dialog's view carries the layer");
        assert_eq!(
            view.decor(),
            &[
                RowDecor {
                    row: 0,
                    open: 0,
                    resolved: 1,
                    marks: Vec::new(),
                    highlight: None,
                    focused: false,
                },
                RowDecor {
                    row: 1,
                    open: 1,
                    resolved: 0,
                    marks: vec![AnnotationMark::Question],
                    highlight: Some(HighlightColour::Yellow),
                    focused: false,
                },
            ]
        );
    });

    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| view.threads_clicked(1, cx))
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.focused, Some(open.id));
        // The rail lists threads in document order, resolved ones only on request.
        assert_eq!(plan.rail_threads(), vec![open.id]);
    });
    md.read_with(cx, |view, _| {
        assert!(view.decor_at(1).is_some_and(|decor| decor.focused));
    });
}

/// `+` on a row opens the composer on that row's block. `⚑` on a row with a thread flips the mark
/// on it; on a row with none it opens the composer with the mark preset, so marking never posts an
/// empty thread.
#[gpui::test]
fn marking_a_row_marks_its_thread_or_starts_one(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let (first, second) = (paragraph("First."), paragraph("Second."));
    let mut thread = thread_on(&first, "why?");
    thread.marks = vec![AnnotationMark::Todo];
    let task = loaded_plan(
        &fixture,
        cx,
        "First.\n\nSecond.\n",
        vec![first.clone(), second.clone()],
        vec![thread.clone()],
        Vec::new(),
    );
    let md = md_of(&fixture, cx);

    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| {
            view.request_mark(0, AnnotationMark::Agent, cx)
        })
    });
    let said = fixture.said();
    assert!(matches!(
        &said[..],
        [Message::MarkAnnotation { annotation_id, mark: AnnotationMark::Agent, on: true, .. }]
            if *annotation_id == thread.id
    ));
    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| {
            view.request_mark(0, AnnotationMark::Todo, cx)
        })
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::MarkAnnotation {
            mark: AnnotationMark::Todo,
            on: false,
            ..
        }]
    ));

    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| {
            view.request_mark(1, AnnotationMark::Question, cx)
        })
    });
    assert!(
        fixture.said().is_empty(),
        "no thread is posted by the mark alone"
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.composer, Some(ComposerTarget::Block(second.id)));
        assert_eq!(plan.composer_marks, vec![AnnotationMark::Question]);
    });
    fixture.with(cx, |state, window, cx| {
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "open question".to_string();
        }
        state.submit_annotation_composer(window, cx)
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::AnnotatePlan { doc, block_id, marks, to: None, text, .. }]
            if *doc == doc_of(fixture.project, task)
                && *block_id == second.id
                && *marks == vec![AnnotationMark::Question]
                && text == "open question"
    ));

    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| view.add_thread(0, cx))
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.composer, Some(ComposerTarget::Block(first.id)));
        assert!(plan.composer_marks.is_empty());
    });

    // `✓` on the row resolves its open thread.
    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| view.request_resolve(0, cx))
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::ResolveAnnotation { annotation_id, resolved: true, .. }]
            if *annotation_id == thread.id
    ));
}

/// `●` colours every host block in the row — a list is one row and each item a block — and the
/// menu's clear sends `None`.
#[gpui::test]
fn highlighting_a_row_colours_all_its_blocks(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let (one, two) = (paragraph("one"), paragraph("two"));
    loaded_plan(
        &fixture,
        cx,
        "- one\n- two\n",
        vec![one.clone(), two.clone()],
        Vec::new(),
        Vec::new(),
    );
    let md = md_of(&fixture, cx);

    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| {
            view.request_highlight(0, Some(HighlightColour::Green), cx)
        })
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::SetBlockHighlight { block_ids, colour: Some(HighlightColour::Green), .. }]
            if *block_ids == vec![one.id, two.id]
    ));
    fixture.with(cx, |_, _, cx| {
        md.update(cx, |view, cx| view.request_highlight(0, None, cx))
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::SetBlockHighlight { colour: None, .. }]
    ));
}

/// A leading `@agent`, or the composer's "@agent" chip, addresses the post to the agent — on a
/// fresh thread and on a reply alike — and the tag itself is not posted.
#[gpui::test]
fn the_agent_toggle_addresses_the_comment(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let block_id = BlockId::generate();
    fixture.with(cx, |state, window, cx| {
        state.compose_annotation(block_id, window, cx);
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "@agent: please check this".to_string();
        }
        state.submit_annotation_composer(window, cx)
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::AnnotatePlan { to: Some(Addressee::Agent), text, .. }]
            if text == "please check this"
    ));

    let annotation_id = AnnotationId::generate();
    fixture.with(cx, |state, window, cx| {
        state.compose_reply(annotation_id, window, cx);
        state.toggle_composer_agent(cx);
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "and this".to_string();
        }
        state.submit_annotation_composer(window, cx)
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::ReplyToAnnotation { annotation_id: a, to: Some(Addressee::Agent), text, .. }]
            if *a == annotation_id && text == "and this"
    ));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.composer_to_agent, "the toggle is sticky across a post");
    });
}

/// A document open on a plan, with `annotations` and `binding` delivered by the host.
fn open_bound(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    annotations: Vec<Annotation>,
    binding: DocBinding,
) -> DocumentHandle {
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();
    let doc = doc_of(fixture.project, task);
    fixture.deliver(
        Message::PlanAnnotations {
            doc: doc.clone(),
            blocks: Vec::new(),
            annotations,
            highlights: Vec::new(),
            binding,
        },
        cx,
    );
    doc
}

/// The binding rides in on `PlanAnnotations`, and `DocAgentQueue` is kept per agent until it is
/// empty.
#[gpui::test]
fn the_binding_and_the_queue_land(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let agent = AgentId::generate();
    let doc = open_bound(
        &fixture,
        cx,
        Vec::new(),
        DocBinding {
            agent: Some(agent),
            auto_send: true,
        },
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("open");
        assert_eq!(plan.binding.agent, Some(agent));
        assert!(plan.binding.auto_send);
    });

    let queue = |pending: Vec<DocThreadRef>, state| Message::DocAgentQueue {
        project_id: fixture.project,
        agent_id: agent,
        pending,
        state,
    };
    let thread = DocThreadRef {
        doc: doc.clone(),
        annotation_id: AnnotationId::generate(),
    };
    fixture.deliver(queue(vec![thread], DocDelivery::Waiting), cx);
    fixture.state.read_with(cx, |state, _| {
        assert_eq!(state.workbench.doc_agent_queues[&agent].0.len(), 1);
    });
    fixture.deliver(queue(Vec::new(), DocDelivery::NotRunning), cx);
    fixture.state.read_with(cx, |state, _| {
        assert!(!state.workbench.doc_agent_queues.contains_key(&agent));
    });
}

/// *New agent…* parks the document; the agent's `ConversationStarted` spends it as `SetDocAgent`.
#[gpui::test]
fn a_started_agent_is_bound_to_its_document(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let doc = open_bound(&fixture, cx, Vec::new(), DocBinding::default());
    fixture.said();
    let id = AgentId::generate();
    fixture.with(cx, |state, _, _| {
        state.workbench.doc_binds.insert(id, doc.clone());
    });
    let session = ubiq_proto::ids::SessionId::generate();
    let agent = WorkAgent {
        id,
        session,
        task: None,
        parent: None,
        name: "claude".to_string(),
        summary: None,
        title: None,
        definition: None,
        activity: Activity::Thinking,
        branch: "main".to_string(),
        tokens: 0.0,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: String::new(),
        context_pct: 0,
        persistent: false,
        accept_all: false,
        debug_dump: None,
        run_dir: None,
        config_dir: None,
        thread: Vec::new(),
    };
    fixture.deliver(
        Message::ConversationStarted {
            project_id: fixture.project,
            agent: Box::new(agent),
            session: WorkSession {
                id: session,
                name: "the project".to_string(),
                branch: String::new(),
                worktree: false,
            },
            accepts_input: true,
        },
        cx,
    );
    assert!(fixture.said().iter().any(|m| matches!(
        m,
        Message::SetDocAgent { doc: d, agent_id: Some(a) } if *d == doc && *a == id
    )));
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.doc_binds.is_empty(), "spent once");
    });
}

/// Editing a user comment seeds the composer, sends `EditAnnotationComment` for changed text and
/// nothing for unchanged text.
#[gpui::test]
fn editing_a_comment_round_trips(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let annotation = Annotation::new(
        BlockId::generate(),
        None,
        CommentAuthor::User,
        "first draft".to_string(),
        Utc::now(),
    );
    let (annotation_id, comment_id) = (annotation.id, annotation.thread[0].id);
    let doc = open_bound(&fixture, cx, vec![annotation], DocBinding::default());
    fixture.said();

    fixture.with(cx, |state, window, cx| {
        state.compose_edit(annotation_id, comment_id, window, cx)
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("open");
        assert_eq!(plan.composer, Some(ComposerTarget::Edit(annotation_id, comment_id)));
        assert_eq!(plan.composer_text, "first draft");
    });

    // Unchanged: closes without a message.
    fixture.with(cx, |state, window, cx| state.submit_annotation_composer(window, cx));
    assert!(fixture.said().is_empty());

    fixture.with(cx, |state, window, cx| {
        state.compose_edit(annotation_id, comment_id, window, cx);
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = " second draft ".to_string();
        }
        state.submit_annotation_composer(window, cx)
    });
    assert!(matches!(
        &fixture.said()[..],
        [Message::EditAnnotationComment { doc: d, annotation_id: a, comment_id: c, text }]
            if *d == doc && *a == annotation_id && *c == comment_id && text == "second draft"
    ));
}
/// The `/` menu is a `CompletionProvider` against the editor's own popover: the trigger fires on
/// the typed character, and what it offers is filtered by the `/word` behind the caret. Nothing
/// here draws a menu, which is the point of doing it this way.
#[gpui::test]
fn slash_commands_answer_the_completion_provider(cx: &mut TestAppContext) {
    let provider = SlashCommands;
    cx.update(|cx| {
        assert!(provider.is_completion_trigger(0, "/", cx));
        assert!(
            provider.is_completion_trigger(0, "t", cx),
            "the letters after the slash keep filtering"
        );
        assert!(!provider.is_completion_trigger(0, " ", cx));
    });

    // The list itself: `/` offers everything, a prefix narrows it, and a caret that is not in a
    // `/word` is offered nothing at all.
    assert_eq!(slash_commands("/").len(), SLASH_COMMANDS.len());
    let narrowed = slash_commands("/ta");
    assert_eq!(narrowed.len(), 2);
    assert!(narrowed.iter().any(|command| command.label == "/task"));
    assert!(slash_prefix("a path src/state", 16).is_none());

    let source = "Steps\n\n/ta";
    let typed = slash_prefix(source, source.len()).expect("the caret is in a command");
    assert_eq!(typed, "/ta");
}

/// Every body the host states is followed by the one question the provenance layer answers, and
/// the runs that come back are decorations over the live text — at the offsets the host's line
/// numbers actually name, with the two origins told apart.
#[gpui::test]
fn changed_lines_are_decorated_by_who_wrote_them(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.said();

    let body = "# The plan\n\nShip the thing.\nThen tell everyone.\n\nA human tightened this.\n";
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: body.to_string(),
            revision: 4,
        },
        cx,
    );

    // The body arriving is what asks where it was edited — since the beginning, so a plan an
    // agent wrote from nothing reads as agent lines throughout.
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::ListPlanChanges { doc, since_revision }
            if *doc == doc_of(fixture.project, task) && since_revision.is_none()
    ));

    fixture.deliver(
        Message::PlanChanges {
            doc: doc_of(fixture.project, task),
            regions: vec![
                a_region(3, 4, SaveOrigin::Agent),
                a_region(6, 6, SaveOrigin::Human),
            ],
            stats: PlanChangeStats {
                since_revision: 0,
                revision: 4,
                lines_added: 5,
                lines_removed: 1,
                lines_modified: 2,
                blocks_touched: 2,
                human_revisions: 1,
                agent_revisions: 3,
            },
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));

    fixture.state.read_with(cx, |state, cx| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.change_stats.lines_added, 5);
        assert_eq!(plan.change_stats.blocks_touched, 2);
        assert!(plan.has_changes_by(SaveOrigin::Human));
        assert!(plan.has_changes_by(SaveOrigin::Agent));

        let ranges = state.change_ranges(cx);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].0, SaveOrigin::Agent);
        assert_eq!(
            &body[ranges[0].1.clone()],
            "Ship the thing.\nThen tell everyone."
        );
        assert_eq!(ranges[1].0, SaveOrigin::Human);
        assert_eq!(&body[ranges[1].1.clone()], "A human tightened this.");
    });

    // A plan that goes away takes its provenance with it rather than leaving marks over a body
    // that is no longer there.
    fixture.deliver(
        Message::PlanDeleted {
            doc: doc_of(fixture.project, task),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.changes.is_empty());
    });
}

/// A buffer seeded at one revision cannot silently replace a newer one. The first Save raises the
/// question and keeps every character typed; the second means it. A *further* save landing under
/// the question asks it again, because what was confirmed was confirmed about the revision before.
#[gpui::test]
fn a_stale_buffer_asks_before_it_overwrites(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "original\n".to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.with(cx, |state, window, cx| {
        let editor = state.plan_editor.clone();
        editor.update(cx, |buffer, cx| buffer.set_value("mine\n", window, cx));
        state.settle_plan_editor(window, cx);
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.revision, 1, "the buffer's watermark is what it loaded");
    });
    fixture.said();

    // An agent's `write_plan`, announced and then reloaded.
    fixture.deliver(
        Message::PlanChanged {
            doc: doc_of(fixture.project, task),
            revision: 2,
            origin: SaveOrigin::Agent,
        },
        cx,
    );
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "theirs\n".to_string(),
            revision: 2,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.said();
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.stale);
        assert_eq!(
            plan.revision, 1,
            "the buffer stayed at what it was seeded with"
        );
        assert_eq!(plan.host_revision, 2);
        assert_eq!(plan.stale_origin, Some(SaveOrigin::Agent));
        assert!(!plan.confirm_overwrite);
    });

    // The first Save writes nothing and keeps what was typed.
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    assert!(
        fixture.said().is_empty(),
        "a stale save is a question before it is a write"
    );
    fixture.state.read_with(cx, |state, cx| {
        assert_eq!(state.plan_editor.read(cx).value().to_string(), "mine\n");
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.confirm_overwrite);
    });

    // A second agent save under the question makes it a new question.
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "theirs again\n".to_string(),
            revision: 3,
        },
        cx,
    );
    fixture.said();
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(
            !plan.confirm_overwrite,
            "one confirmation covers one revision"
        );
        assert_eq!(plan.host_revision, 3);
    });

    // Ask, then mean it.
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    assert!(fixture.said().is_empty());
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::SavePlan { doc, body, expected }
            if *doc == doc_of(fixture.project, task) && body == "mine\n"
                // A confirmed overwrite names the newest revision the host has stated — the copy
                // the user was actually shown — and never the one the buffer was seeded from,
                // which nothing would accept any more. There is no force flag on the wire.
                && *expected == 3
    ));

    // And the host's answer to that save settles the surface at the new watermark.
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "mine\n".to_string(),
            revision: 4,
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(!plan.stale && !plan.dirty && !plan.confirm_overwrite);
        assert_eq!(plan.revision, 4);
        assert!(plan.stale_origin.is_none());
    });
}

/// **The guard is the host's, and this is what proves it.** A buffer that believes it is current
/// still names the revision it was seeded from, and a save that lands in between is refused rather
/// than overwritten — `PlanConflict`. The edit is kept to the character, the surface asks the
/// overwrite question against the revision the refusal named, and the press that follows names
/// *that*. There is no force flag to reach for.
#[gpui::test]
fn a_refused_save_keeps_the_edit_and_asks_again(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));
    fixture.deliver(
        Message::Plan {
            doc: doc_of(fixture.project, task),
            body: "original\n".to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.with(cx, |state, window, cx| {
        let editor = state.plan_editor.clone();
        editor.update(cx, |buffer, cx| buffer.set_value("mine\n", window, cx));
        state.settle_plan_editor(window, cx);
    });
    fixture.said();

    // The buffer believes it is current, so ⌘S writes on the first press — naming revision 1.
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::SavePlan { expected, .. } if *expected == 1
    ));

    // Somebody else's save landed first. Nothing of this window's was written.
    fixture.deliver(
        Message::PlanConflict {
            doc: doc_of(fixture.project, task),
            revision: 2,
            origin: SaveOrigin::Human,
        },
        cx,
    );
    fixture.state.read_with(cx, |state, cx| {
        assert_eq!(
            state.plan_editor.read(cx).value().to_string(),
            "mine\n",
            "a refusal is the one failure that leaves the buffer exactly as it is",
        );
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(!plan.saving, "the save is over — it did not land");
        assert!(plan.stale && plan.dirty);
        assert_eq!(
            plan.host_revision, 2,
            "the host's word about where it stands"
        );
        assert_eq!(
            plan.revision, 1,
            "the buffer stayed at what it was seeded with"
        );
        assert_eq!(plan.stale_origin, Some(SaveOrigin::Human));
        assert!(
            !plan.confirm_overwrite,
            "the question is asked about this revision, not the one already overtaken",
        );
    });

    // Ask, then mean it — and the press that means it names what the refusal reported.
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    assert!(fixture.said().is_empty());
    fixture.with(cx, |state, window, cx| state.save_document(window, cx));
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::SavePlan { body, expected, .. } if body == "mine\n" && *expected == 2
    ));
}

/// A block committed in the dialog's view is an ordinary whole-document save: the buffer holds
/// the edit, and `SavePlan` goes out with it against the revision the buffer was seeded from. The
/// host re-indexes and re-matches the blocks from there.
#[gpui::test]
fn a_block_commit_saves_the_plan(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = loaded_plan(
        &fixture,
        cx,
        "# The plan\n\nFirst.\n\nSecond.\n",
        vec![paragraph("First."), paragraph("Second.")],
        Vec::new(),
        Vec::new(),
    );
    let md = md_of(&fixture, cx);

    fixture.with(cx, |_, window, cx| {
        md.update(cx, |view, cx| {
            view.set_editable(true, window, cx);
            // The last paragraph: one under a heading is edited together with it.
            blockedit::open(view, 2, window, cx);
        });
        let edit = md
            .read(cx)
            .editing()
            .map(|edit| edit.state.clone())
            .expect("the block editor opened");
        edit.update(cx, |state, cx| {
            state.set_value("Second, revised.", window, cx)
        });
        assert!(md.update(cx, |view, cx| blockedit::commit(view, window, cx)));
    });
    cx.run_until_parked();

    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::SavePlan { doc, body, expected }
                if *doc == doc_of(fixture.project, task)
                    && body == "# The plan\n\nFirst.\n\nSecond, revised.\n"
                    && *expected == 1
        )),
        "the commit went out as a save: {said:?}",
    );
}

// ── T-124: markdown's fourth mode ────────────────────────────────────────────

/// The four positions a markdown tab offers, in the order its header draws them — Annotation last,
/// and offered by no other viewer, because no other viewer has a document handle to point the
/// surface at.
#[test]
fn markdown_offers_annotation_last_and_alone() {
    use ubiq::state::editor::{ViewLayout, ViewerKind};
    assert_eq!(
        ViewerKind::Markdown.layouts(),
        &[
            ViewLayout::Source,
            ViewLayout::Split,
            ViewLayout::Preview,
            ViewLayout::Annotation,
        ]
    );
    for kind in ViewerKind::all() {
        assert_eq!(
            kind.offers(ViewLayout::Annotation),
            kind == ViewerKind::Markdown,
            "{kind:?}",
        );
    }
    // The surface draws the document, never the buffer — the mode a thread is written in is not a
    // mode the source is edited in.
    assert!(!ViewerKind::Markdown.shows_buffer(ViewLayout::Annotation));
}

/// A markdown tab put into annotation mode opens *that file* as an annotated document — the same
/// family on the wire the plan uses, against `DocumentHandle::File` — and it is not the dialog:
/// nothing is raised over the window, and Escape has no plan to close.
#[gpui::test]
fn annotation_mode_opens_the_files_document(cx: &mut TestAppContext) {
    use ubiq::state::editor::{Subject, ViewLayout, tab_key};

    let fixture = Fixture::open(cx);
    let key = tab_key("notes.md", Subject::File);
    fixture.with(cx, |state, _, cx| {
        state.select_file("notes.md".to_string(), cx)
    });
    // The surface draws the tab's own view, which the tab builds once its bytes are here.
    let bytes = b"# Notes\n\nalpha\n";
    fixture.deliver(
        Message::ProjectFileContents {
            project_id: fixture.project,
            rel_path: "notes.md".to_string(),
            contents: ubiq_proto::files::FileContents {
                bytes: bytes.to_vec(),
                len: bytes.len() as u64,
                truncated: false,
                is_binary: false,
                version: Some(ubiq_proto::files::FileVersion {
                    len: bytes.len() as u64,
                    modified: None,
                }),
            },
        },
        cx,
    );
    fixture.said(); // drain the read the tab asked for

    fixture.with(cx, |state, _, cx| {
        state.set_view_layout(&key, ViewLayout::Annotation, cx)
    });

    let handle = DocumentHandle::File {
        project_id: fixture.project,
        rel_path: "notes.md".to_string(),
    };
    let said = fixture.said();
    assert!(
        said.iter()
            .any(|message| matches!(message, Message::LoadPlan { doc } if *doc == handle)),
        "the body was asked for: {said:?}",
    );
    assert!(
        said.iter().any(
            |message| matches!(message, Message::ListPlanAnnotations { doc, .. } if *doc == handle)
        ),
        "the threads were asked for: {said:?}",
    );
    let tab_md = fixture
        .with(cx, |state, _, cx| {
            state.file("notes.md", cx).and_then(|file| file.md.clone())
        })
        .expect("the tab has a view");
    fixture.state.read_with(cx, |state, _| {
        let doc = state.workbench.plan.as_ref().expect("a document is open");
        assert_eq!(doc.doc, handle);
        assert!(!doc.is_modal(), "a tab's document is not the dialog");
        assert!(doc.md == tab_md, "the surface is the tab's own view");
    });
    tab_md.read_with(cx, |view, _| assert!(view.annotating()));

    // Leaving the mode puts the document away: the tab is reading again, and nothing is holding a
    // surface nobody is looking at.
    fixture.with(cx, |state, _, cx| {
        state.set_view_layout(&key, ViewLayout::Preview, cx)
    });
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.plan.is_none(), "the document was put away");
    });
    tab_md.read_with(cx, |view, _| {
        assert!(!view.annotating(), "the tab's view is reading again");
        assert!(view.decor().is_empty());
    });
}
