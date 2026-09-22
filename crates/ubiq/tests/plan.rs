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
    AnnotationsBody, ComposerTarget, DocumentBody, Notice, SLASH_COMMANDS, slash_commands,
    slash_prefix,
};
use ubiq::ui::editor::SlashCommands;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{
    Annotation, AnnotationState, PlanBlock, PlanChangeStats, PlanChangedRegion, SaveOrigin,
};
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{CommentAuthor, Level, Priority, Status, TaskRecord};

const PATIENCE: Duration = Duration::from_millis(500);

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
        Message::LoadPlan { project_id, task_id }
            if *project_id == fixture.project && *task_id == task
    )));
    assert!(said.iter().any(|message| matches!(
        message,
        Message::ListPlanAnnotations { project_id, task_id }
            if *project_id == fixture.project && *task_id == task
    )));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("the modal is open");
        assert_eq!(plan.task_id(), Some(task));
        assert!(matches!(plan.body, DocumentBody::Loading));
        assert!(matches!(plan.annotations, AnnotationsBody::Loading));
    });

    fixture.deliver(
        Message::Plan {
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
            revision: 2,
            origin: SaveOrigin::Agent,
        },
        cx,
    );

    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::LoadPlan { project_id, task_id }
            if *project_id == fixture.project && *task_id == task
    ));

    // A `PlanChanged` for a task whose plan is not open asks for nothing: nobody is waiting on
    // it.
    fixture.with(cx, |state, _, cx| state.close_plan(cx));
    fixture.deliver(
        Message::PlanChanged {
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
            body: "# The plan".to_string(),
            revision: 1,
        },
        cx,
    );

    fixture.deliver(
        Message::PlanDeleted {
            project_id: fixture.project,
            task_id: task,
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
            task_id: Some(task),
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
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
            blocks: Vec::new(),
            annotations: Vec::new(),
        },
        cx,
    );
    fixture.deliver(
        Message::PlanError {
            project_id: fixture.project,
            task_id: Some(task),
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
            project_id: fixture.project,
            task_id: task,
            blocks: vec![block.clone()],
            annotations: vec![annotation.clone()],
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        match &plan.annotations {
            AnnotationsBody::Loaded {
                blocks,
                annotations,
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
            project_id: fixture.project,
            task_id: task,
        },
        cx,
    );
    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::ListPlanAnnotations { project_id, task_id }
            if *project_id == fixture.project && *task_id == task
    ));
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(matches!(plan.annotations, AnnotationsBody::Loading));
    });

    fixture.with(cx, |state, _, cx| state.close_plan(cx));
    fixture.deliver(
        Message::PlanAnnotationsChanged {
            project_id: fixture.project,
            task_id: task,
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
        Message::AnnotatePlan { project_id, task_id: t, block_id: b, quote: None, text }
            if *project_id == fixture.project && *t == task && *b == block_id && text == "Why here?"
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
        Message::ReplyToAnnotation { project_id, task_id: t, annotation_id: a, text }
            if *project_id == fixture.project && *t == task && *a == annotation_id && text == "Agreed."
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
        Message::ResolveAnnotation { project_id, task_id: t, annotation_id: a, resolved: true }
            if *project_id == fixture.project && *t == task && *a == annotation_id
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
            project_id: fixture.project,
            task_id: task,
            // The block itself is gone from the index — that absence is what "orphaned" means —
            // but the flag on the annotation is what says so, not a lookup miss the interface
            // would have to reconstruct.
            blocks: Vec::new(),
            annotations: vec![annotation.clone()],
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
            project_id: fixture.project,
            task_id: task,
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
        Message::SavePlan { project_id, task_id: t, body }
            if *project_id == fixture.project
                && *t == task
                && body == "# The plan\n\nFirst step.\n"
    ));

    fixture.deliver(
        Message::Plan {
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
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

/// The annotated passages are decorations over the live text, at the offsets the host's blocks
/// are actually at — the join between block ids, which are the host's, and buffer offsets, which
/// are the window's. A quoted passage narrows the range to the quote.
#[gpui::test]
fn annotations_decorate_their_passages(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));

    let body = "# The plan\n\nShip the thing by Friday.\n\nThen tell everyone.\n";
    let heading = PlanBlock {
        id: BlockId::generate(),
        kind: "heading:1".to_string(),
        text: "# The plan".to_string(),
    };
    let step = PlanBlock {
        id: BlockId::generate(),
        kind: "paragraph".to_string(),
        text: "Ship the thing by Friday.".to_string(),
    };
    let quoted = Annotation::new(
        step.id,
        Some("by Friday".to_string()),
        CommentAuthor::User,
        "is that real?".to_string(),
        Utc::now(),
    );
    let whole = Annotation::new(
        heading.id,
        None,
        CommentAuthor::Agent,
        "renamed?".to_string(),
        Utc::now(),
    );

    fixture.deliver(
        Message::Plan {
            project_id: fixture.project,
            task_id: task,
            body: body.to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.deliver(
        Message::PlanAnnotations {
            project_id: fixture.project,
            task_id: task,
            blocks: vec![heading.clone(), step.clone()],
            annotations: vec![quoted.clone(), whole.clone()],
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));

    fixture.state.read_with(cx, |state, cx| {
        let ranges = state.annotation_ranges(cx);
        assert_eq!(ranges.len(), 2);
        let for_quote = ranges
            .iter()
            .find(|(id, _)| *id == quoted.id)
            .map(|(_, range)| range.clone())
            .expect("the quoted thread has a range");
        assert_eq!(&body[for_quote], "by Friday");
        let for_block = ranges
            .iter()
            .find(|(id, _)| *id == whole.id)
            .map(|(_, range)| range.clone())
            .expect("the whole-block thread has a range");
        assert_eq!(&body[for_block], "# The plan");
    });

    // Clicking inside a decorated passage is what opens its thread over that passage.
    fixture.with(cx, |state, _, cx| {
        let editor = state.plan_editor.clone();
        let at = body.find("by Friday").expect("the passage is in the body");
        editor.update(cx, |buffer, cx| buffer.set_selected_range(at..at, cx));
        state.document_clicked(cx);
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert_eq!(plan.thread, Some(quoted.id));
    });

    // A click in unannotated text closes it again rather than leaving a popover behind.
    fixture.with(cx, |state, _, cx| {
        let editor = state.plan_editor.clone();
        let at = body.find("Then tell").expect("the tail is in the body");
        editor.update(cx, |buffer, cx| buffer.set_selected_range(at..at, cx));
        state.document_clicked(cx);
    });
    fixture.state.read_with(cx, |state, _| {
        let plan = state.workbench.plan.as_ref().expect("still open");
        assert!(plan.thread.is_none());
    });
}

/// Annotating a selection anchors on the block the selection is in and quotes the passage — the
/// character-range selection slice 4 could not offer. A selection in text the host has no block
/// for is refused where the user is looking rather than posted as a thread that would be
/// orphaned on arrival.
#[gpui::test]
fn annotating_a_selection_quotes_the_passage(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let task = fixture.seed_task(Some(Level::Mission), cx);
    fixture.said();
    fixture.with(cx, |state, _, cx| state.open_plan(task, cx));

    let body = "Ship the thing by Friday.\n";
    let block = PlanBlock {
        id: BlockId::generate(),
        kind: "paragraph".to_string(),
        text: "Ship the thing by Friday.".to_string(),
    };
    fixture.deliver(
        Message::Plan {
            project_id: fixture.project,
            task_id: task,
            body: body.to_string(),
            revision: 1,
        },
        cx,
    );
    fixture.deliver(
        Message::PlanAnnotations {
            project_id: fixture.project,
            task_id: task,
            blocks: vec![block.clone()],
            annotations: Vec::new(),
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| state.settle_plan_editor(window, cx));
    fixture.said();

    let at = body.find("by Friday").expect("the passage is in the body");
    fixture.with(cx, |state, window, cx| {
        let editor = state.plan_editor.clone();
        editor.update(cx, |buffer, cx| {
            buffer.set_selected_range(at..at + "by Friday".len(), cx)
        });
        state.annotate_selection(window, cx);
    });
    fixture.with(cx, |state, _, _| {
        if let Some(plan) = state.workbench.plan.as_mut() {
            plan.composer_text = "is that real?".to_string();
        }
    });
    fixture.with(cx, |state, window, cx| {
        state.submit_annotation_composer(window, cx)
    });

    let said = fixture.said();
    assert_eq!(said.len(), 1);
    assert!(matches!(
        &said[0],
        Message::AnnotatePlan { block_id, quote: Some(quote), text, .. }
            if *block_id == block.id && quote == "by Friday" && text == "is that real?"
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
            project_id: fixture.project,
            task_id: task,
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
        Message::ListPlanChanges { project_id, task_id, since_revision }
            if *project_id == fixture.project
                && *task_id == task
                && since_revision.is_none()
    ));

    fixture.deliver(
        Message::PlanChanges {
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
            revision: 2,
            origin: SaveOrigin::Agent,
        },
        cx,
    );
    fixture.deliver(
        Message::Plan {
            project_id: fixture.project,
            task_id: task,
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
            project_id: fixture.project,
            task_id: task,
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
        Message::SavePlan { project_id, task_id: t, body }
            if *project_id == fixture.project && *t == task && body == "mine\n"
    ));

    // And the host's answer to that save settles the surface at the new watermark.
    fixture.deliver(
        Message::Plan {
            project_id: fixture.project,
            task_id: task,
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
