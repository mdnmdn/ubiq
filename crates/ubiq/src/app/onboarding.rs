//! The onboarding wizard as the window acts on it: starting a run, moving through it, and the
//! check that decides whether a flow is due.
//!
//! The data is `crate::state::onboarding` and the flows themselves are
//! `crate::ui::onboarding::flows`. What is answered here is everything a step names but cannot
//! do: whether a [`Gate`] is open, what a [`StepAction`] does to the window, and which flows a
//! finished or skipped run marks done in the interface's preferences.
//!
//! **Only the first window auto-starts a launch flow.** A process opening several windows would
//! otherwise greet the user once per window; [`AUTO_STARTED`] is the process-wide answer. Mode
//! flows are per window and need no such guard — the done set is.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, Context};

use super::AppState;
use crate::state::RailMode;
use crate::state::onboarding::{FlowId, Gate, OnboardingRun, StepAction, Trigger};

/// Whether this process has already auto-started a [`Trigger::Start`] flow in some window.
static AUTO_STARTED: AtomicBool = AtomicBool::new(false);

impl AppState {
    /// Start a run at `flow`, done or not — the overflow menu's **Show onboarding**, and the
    /// automatic starts below.
    pub fn open_onboarding(&mut self, flow: FlowId, cx: &mut Context<Self>) {
        let registry = crate::ui::onboarding::flows::all(self, cx);
        let Some(run) = OnboardingRun::chain(flow, &registry) else {
            return;
        };
        self.workbench.open_menu = None;
        self.workbench.onboarding = Some(run);
        self.enter_onboarding_step(cx);
        cx.notify();
    }

    /// Whether the current step's Back is live.
    pub fn onboarding_back_open(&self, cx: &App) -> bool {
        self.workbench
            .onboarding
            .as_ref()
            .is_some_and(|run| !run.is_first() && self.gate_open(run.current().back, cx))
    }

    /// Whether the current step's Next (or Finish) is live.
    pub fn onboarding_next_open(&self, cx: &App) -> bool {
        self.workbench
            .onboarding
            .as_ref()
            .is_some_and(|run| self.gate_open(run.current().next, cx))
    }

    /// Next: a flow whose last step is passed is done; passing the run's last step closes it.
    pub fn onboarding_next(&mut self, cx: &mut Context<Self>) {
        if !self.onboarding_next_open(cx) {
            return;
        }
        let Some(run) = self.workbench.onboarding.as_mut() else {
            return;
        };
        if run.ends_flow() {
            let flow = run.current_flow();
            self.workbench.onboarding_done.insert(flow.to_string());
            self.remember_interface();
        }
        let Some(run) = self.workbench.onboarding.as_mut() else {
            return;
        };
        if run.forward() {
            self.enter_onboarding_step(cx);
            cx.notify();
        } else {
            self.finish_onboarding(cx);
        }
    }

    pub fn onboarding_back(&mut self, cx: &mut Context<Self>) {
        if !self.onboarding_back_open(cx) {
            return;
        }
        if self
            .workbench
            .onboarding
            .as_mut()
            .is_some_and(|run| run.back())
        {
            self.enter_onboarding_step(cx);
            cx.notify();
        }
    }

    /// Skip: the way out that is always offered. Every flow of the run counts as done, so none of
    /// them comes back by itself.
    pub fn skip_onboarding(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.workbench.onboarding.as_ref() else {
            return;
        };
        let flows: Vec<String> = run.flows.iter().map(|flow| flow.to_string()).collect();
        self.workbench.onboarding_done.extend(flows);
        self.remember_interface();
        self.finish_onboarding(cx);
    }

    /// Close the run, and see whether another flow is now due — the IDE's, after the welcome.
    fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        self.workbench.onboarding = None;
        self.check_onboarding(cx);
        cx.notify();
    }

    /// Start whichever flow is due, if any.
    ///
    /// Nothing while a run is open or before the interface's preferences have answered. A launch
    /// flow not yet done comes first, once per process; once every launch flow is done, the flow
    /// of the mode the window is in — if that mode needs a project, only once one is loaded.
    pub fn check_onboarding(&mut self, cx: &mut Context<Self>) {
        if self.workbench.onboarding.is_some() || !self.workbench.prefs_loaded {
            return;
        }
        let registry = crate::ui::onboarding::flows::all(self, cx);
        let done = &self.workbench.onboarding_done;
        let pending_start = registry
            .iter()
            .find(|flow| flow.trigger == Trigger::Start && !done.contains(flow.id))
            .map(|flow| flow.id);
        if let Some(flow) = pending_start {
            if !AUTO_STARTED.swap(true, Ordering::SeqCst) {
                self.open_onboarding(flow, cx);
            }
            return;
        }
        let mode = self.workbench.rail_mode;
        let due = registry
            .iter()
            .find(|flow| flow.trigger == Trigger::EnterMode(mode) && !done.contains(flow.id))
            .map(|flow| flow.id);
        let Some(flow) = due else {
            return;
        };
        let needs_project = mode.spec().is_some_and(|spec| spec.needs_project);
        if needs_project && !self.project_loaded(cx) {
            return;
        }
        self.open_onboarding(flow, cx);
    }

    /// Whether the window has a project open *and* that project's furniture has arrived — before
    /// it has, the mode the window reports is the default rather than the project's own.
    fn project_loaded(&self, cx: &App) -> bool {
        self.project(cx)
            .and_then(|id| self.projects.get(&id))
            .is_some_and(|open| open.restored)
    }

    fn gate_open(&self, gate: Gate, cx: &App) -> bool {
        match gate {
            Gate::On => true,
            Gate::Off => false,
            Gate::ProjectOpen => self.project(cx).is_some(),
        }
    }

    /// Run the current step's arrival action, by Next and by Back alike.
    fn enter_onboarding_step(&mut self, cx: &mut Context<Self>) {
        let action = self
            .workbench
            .onboarding
            .as_ref()
            .and_then(|run| run.current().on_enter);
        if let Some(StepAction::EnterMode(mode)) = action {
            self.onboarding_enter_mode(mode, cx);
        }
    }

    /// A mode switch made on the user's behalf: only to a mode the rail is drawing.
    fn onboarding_enter_mode(&mut self, mode: RailMode, cx: &mut Context<Self>) {
        if self.mode_enabled(mode, cx) {
            self.enter_rail_mode(mode, false, cx);
        }
    }
}
