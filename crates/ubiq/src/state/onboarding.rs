//! Onboarding: a wizard of consecutive flows, each a list of steps.
//!
//! Data and small mutators only. A [`Flow`] is one named walk — the welcome, the tour of the main
//! screen, the first visit to the IDE — and a [`Trigger`] says when it starts on its own. A flow
//! may name another to follow it (`then`), and a [`OnboardingRun`] is a flow plus everything it
//! chains, concatenated: the progress dots span the whole run, not one flow of it.
//!
//! A step is informative (markdown), a form the interface draws, or a **highlight**: the dialog
//! steps aside, a place on the screen is outlined, and a small card beside it carries the same
//! navigation. A **sub-flow** step stands for a whole other flow — the welcome's tour — in one
//! step and one dot: launched from it, that flow runs as a sub-run with its own steps and progress
//! and hands the window back to the step when it ends. What a step does on arrival ([`StepAction`]) and when its buttons are live
//! ([`Gate`]) are named here and answered in `app/onboarding.rs`, so nothing in this module needs
//! `AppState`.
//!
//! Which flows are done is `WorkbenchState::onboarding_done`, mirrored to the interface's
//! preferences; the registry of flows is `ui::onboarding::flows`.

use gpui::SharedString;

use super::RailMode;
use super::ui_id::UiId;

/// A flow's stable name — what `onboarding_done` records.
pub type FlowId = &'static str;

/// The first-run welcome.
pub const WELCOME: FlowId = "welcome";
/// The tour of the main screen, a sub-flow of the welcome.
pub const TOUR: FlowId = "tour";
/// The first visit to the IDE with a project open.
pub const IDE: FlowId = "ide";

/// When a flow starts by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// At launch, while it is not done.
    Start,
    /// The first time the window is in this mode (with a project, if the mode needs one).
    EnterMode(RailMode),
    /// Never by itself: opened by hand, reached through another flow's `then`, or launched from a
    /// sub-flow step.
    Manual,
}

/// A form the dialog draws in place of a markdown body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    /// Pick the palette the window wears.
    Palette,
}

/// What a step shows.
#[derive(Clone, Debug, PartialEq)]
pub enum StepBody {
    /// The dialog, with this markdown as its body.
    Markdown(SharedString),
    /// The dialog, with a form.
    Form(FormKind),
    /// No dialog: `target` is outlined and `text` sits in a card beside it.
    Highlight { target: UiId, text: SharedString },
    /// The dialog, with this markdown and a `launch` button that opens `flow` as a sub-run. The
    /// whole sub-flow is this one step: Next passes it without running it.
    SubFlow {
        flow: FlowId,
        source: SharedString,
        launch: SharedString,
    },
}

/// Whether a navigation button is live on a step. Named conditions rather than closures, so the
/// answer can need the whole window and this module none of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Gate {
    #[default]
    On,
    Off,
    /// Live while the window has a project open.
    ProjectOpen,
}

/// Something a step does to the window when it is arrived at, by Next or by Back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepAction {
    /// Move the window to this rail mode.
    EnterMode(RailMode),
}

/// One page of a flow.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub title: SharedString,
    pub body: StepBody,
    pub back: Gate,
    pub next: Gate,
    pub on_enter: Option<StepAction>,
}

impl Step {
    pub fn markdown(title: impl Into<SharedString>, source: impl Into<SharedString>) -> Self {
        Self::with(title, StepBody::Markdown(source.into()))
    }

    pub fn form(title: impl Into<SharedString>, form: FormKind) -> Self {
        Self::with(title, StepBody::Form(form))
    }

    pub fn highlight(
        title: impl Into<SharedString>,
        target: UiId,
        text: impl Into<SharedString>,
    ) -> Self {
        Self::with(
            title,
            StepBody::Highlight {
                target,
                text: text.into(),
            },
        )
    }

    pub fn sub_flow(
        title: impl Into<SharedString>,
        flow: FlowId,
        source: impl Into<SharedString>,
        launch: impl Into<SharedString>,
    ) -> Self {
        Self::with(
            title,
            StepBody::SubFlow {
                flow,
                source: source.into(),
                launch: launch.into(),
            },
        )
    }

    pub fn entering(mut self, action: StepAction) -> Self {
        self.on_enter = Some(action);
        self
    }

    fn with(title: impl Into<SharedString>, body: StepBody) -> Self {
        Self {
            title: title.into(),
            body,
            back: Gate::On,
            next: Gate::On,
            on_enter: None,
        }
    }

    /// Whether the dialog steps aside for this step.
    pub fn hides_dialog(&self) -> bool {
        matches!(self.body, StepBody::Highlight { .. })
    }
}

/// One named walk.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub id: FlowId,
    pub title: SharedString,
    pub trigger: Trigger,
    /// The flow that follows this one in the same run.
    pub then: Option<FlowId>,
    pub steps: Vec<Step>,
}

/// A flow and the flows it chains, open on one step.
///
/// A run opened from a [`StepBody::SubFlow`] step is a **sub-run**: it holds the run it was
/// launched from as `parent`, with its own steps and its own progress, and leaving it — past its
/// last step, by its Skip, or Back from its first — puts the parent back on screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnboardingRun {
    /// Every flow in the run, in order.
    pub flows: Vec<FlowId>,
    pub steps: Vec<(FlowId, Step)>,
    pub index: usize,
    /// The run this one was launched from, when it is a sub-run.
    pub parent: Option<Box<OnboardingRun>>,
}

impl OnboardingRun {
    /// The run starting at `start`, following `then` through `registry`. `None` when `start` is
    /// not registered or the run would have no steps. A chain that loops stops at the repeat.
    pub fn chain(start: FlowId, registry: &[Flow]) -> Option<Self> {
        let mut flows = Vec::new();
        let mut steps = Vec::new();
        let mut next = Some(start);
        while let Some(id) = next {
            if flows.contains(&id) {
                break;
            }
            let Some(flow) = registry.iter().find(|flow| flow.id == id) else {
                break;
            };
            flows.push(id);
            steps.extend(flow.steps.iter().cloned().map(|step| (id, step)));
            next = flow.then;
        }
        (!steps.is_empty()).then_some(Self {
            flows,
            steps,
            index: 0,
            parent: None,
        })
    }

    /// Open `sub` over this run, which it keeps as its parent.
    pub fn enter_sub(&mut self, mut sub: OnboardingRun) {
        let parent = std::mem::replace(self, OnboardingRun::default());
        sub.parent = Some(Box::new(parent));
        *self = sub;
    }

    /// Put the parent back, on the step that launched this sub-run. `false` for a top-level run.
    pub fn leave_sub(&mut self) -> bool {
        let Some(parent) = self.parent.take() else {
            return false;
        };
        *self = *parent;
        true
    }

    pub fn is_sub(&self) -> bool {
        self.parent.is_some()
    }

    pub fn current(&self) -> &Step {
        &self.steps[self.index].1
    }

    pub fn current_flow(&self) -> FlowId {
        self.steps[self.index].0
    }

    /// Whether the dialog is stepped aside for the current step.
    pub fn hidden(&self) -> bool {
        self.current().hides_dialog()
    }

    pub fn is_first(&self) -> bool {
        self.index == 0
    }

    pub fn is_last(&self) -> bool {
        self.index + 1 >= self.steps.len()
    }

    /// Whether the current step is the last of its own flow — passing it completes that flow.
    pub fn ends_flow(&self) -> bool {
        self.steps
            .get(self.index + 1)
            .is_none_or(|(flow, _)| *flow != self.current_flow())
    }

    /// `(index, len)` across the whole run.
    pub fn progress(&self) -> (usize, usize) {
        (self.index, self.steps.len())
    }

    /// One step on. `false` at the last step, which the caller treats as finishing.
    pub fn forward(&mut self) -> bool {
        if self.is_last() {
            return false;
        }
        self.index += 1;
        true
    }

    /// One step back. `false` at the first.
    pub fn back(&mut self) -> bool {
        if self.is_first() {
            return false;
        }
        self.index -= 1;
        true
    }
}
