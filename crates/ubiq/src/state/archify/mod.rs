//! The Archify viewer's interface state: one `Global`, never a field on `AppState`.
//!
//! The file is the IDE's (D4 = A, the base viewer seam D204): its tab reads, watches and saves it,
//! and the contributed viewer is handed the tab's own buffer to draw from. So nothing here holds a
//! document. What lives here is what the viewer keeps *about* a tab, keyed by the tab's key
//! (`OpenFile::key()`): the latest compile and its picture, whether the diagnostics panel is open,
//! the agent's last tool call, and (in [`Ui::views`], like the viewport) what the user is doing to
//! the picture.
//!
//! A compile is scheduled from the viewer's draw ([`Ui::want_compile`]) when the buffer's text
//! hashes differently from the one last scheduled, so an edit, a reload from disk and a changed
//! quality override all arrive the same way.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use gpui::{App, FocusHandle, Global};
use ubiq_archify::scene::{Bounds, GroupId, Scene};

use crate::state::archify::compiled::{Due, Outcome};
use crate::state::archify::focus::Reach;
use crate::ui::archify::theme::ChartTheme;
use crate::ui::archify::agent::{self, AgentCall, Ask, Notice};

pub mod animate;
pub mod compiled;
pub mod focus;
pub mod motion;

/// What the viewer keeps about one tab.
pub struct Tab {
    /// The hash of the text the latest compile was scheduled for; `None` asks for a compile of
    /// whatever the buffer holds (a new tab, or a changed override).
    requested: Option<u64>,
    /// Sequence number of the latest schedule; an older answer is dropped.
    seq: u64,
    /// The latest `compile` result that was not superseded (`None` until the first lands).
    pub compiled: Option<Arc<Outcome>>,
    /// The last scene a compile produced. It outlives a compile that fails, so a typo in the
    /// source leaves the picture up while the panel says what is wrong.
    pub scene: Option<Arc<Scene>>,
    /// The diagnostics panel is open.
    pub panel_open: bool,
    /// The latest MCP tool call the host reported against this file (the status chip).
    pub agent: Option<AgentCall>,
    /// Whether the document passed the last time an agent looked at it: set by each
    /// `archify_validate` and, when Ask agent is chosen, by what the document was then. A pass
    /// after a `Some(false)` is the notification.
    validated: Option<bool>,
}

impl Default for Tab {
    fn default() -> Self {
        Tab {
            requested: None,
            seq: 0,
            compiled: None,
            scene: None,
            panel_open: true,
            agent: None,
            validated: None,
        }
    }
}

/// What [`Ui::want_compile`] hands to `compiled::drive`: when to run, and what the run needs that
/// is not the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub due: Due,
    pub seq: u64,
    /// The settings' quality override at the moment of scheduling (`Opts.quality`, D15).
    pub quality: Option<String>,
    /// The settings' palette at the moment of scheduling: a preset name, or `document` (P9.1).
    pub palette: String,
}

/// The viewport key of a tab's picture, and the key of its [`Interaction`], camera and motion.
pub fn picture_key(tab_key: &str) -> String {
    format!("archify.{tab_key}")
}

/// The tab key a picture key was made from (`picture_key` undone).
pub fn tab_of(picture_key: &str) -> &str {
    picture_key
        .strip_prefix("archify.")
        .unwrap_or(picture_key)
}

/// The hash a tab's text is compared by.
pub fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// The tooltip's subject: the group under the pointer and where the pointer was when it got there,
/// in pixels from the picture's top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hover {
    pub group: GroupId,
    pub x: f32,
    pub y: f32,
}

/// What the user is doing to one picture (keyed like its viewport, `archify.<tab key>`): the
/// focused node, how far the light reaches from it, the tooltip, and where a click began. Lives in
/// the `Global`, so it survives a tab switch; it is not part of the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interaction {
    /// The focused node's id (`Group::node_id`). Stale after an edit that removes the node: the
    /// painter then lights nothing and dims nothing.
    pub focus: Option<String>,
    pub reach: Reach,
    pub hover: Option<Hover>,
    /// Window position of a left press, until its release decides whether it was a click.
    pub down: Option<[f32; 2]>,
    /// A box (viewBox units) a diagnostic click wants in view. Kept until the picture is measured,
    /// then `agent::drive_reveal` pans to it and clears it.
    pub reveal: Option<Bounds>,
    /// The node finder is open (`finder.rs`): its list hangs over the picture and the filter field
    /// holds the keys until it closes.
    pub finder: bool,
}

/// The viewer's preferences: `UiSettings.archify`, persisted by `SetSettings`. A blob written
/// before this existed reads as the default.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ArchifySettings {
    /// The quality-profile override: `advisory`, `standard` or `showcase`; `None` follows each
    /// document's `meta.quality_profile` (D15).
    pub quality: Option<String>,
    /// The visual preset: `document`, or `classic`, `signal-flow`, `blueprint`, `editorial`.
    pub palette: String,
    /// Whether ambient motion plays; off is the reduced-motion state.
    pub motion: bool,
    /// The chart's own light/dark choice (`auto`, `dark`, `light`); `None` is `auto`.
    pub chart_theme: Option<String>,
}

impl Default for ArchifySettings {
    fn default() -> Self {
        ArchifySettings {
            quality: None,
            palette: "document".into(),
            motion: true,
            chart_theme: None,
        }
    }
}

#[derive(Default)]
pub struct Ui {
    /// Per tab, keyed by `OpenFile::key()`.
    pub tabs: HashMap<String, Tab>,
    /// Per-picture focus, reach and hover, keyed like the viewport.
    views: HashMap<String, Interaction>,
    /// Takes the keys while a picture has focus (`key_context("Archify")`). One per picture:
    /// two tabs can be on screen at once in a split dock.
    pub picture_focus: HashMap<String, FocusHandle>,
    /// The preferences.
    pub settings: ArchifySettings,
    /// The newest tool call that named no tab (`archify_guide`, an inline document): every tab
    /// shows it until a call of its own is newer.
    unbound_call: Option<AgentCall>,
    /// A redraw ticker for the chip's "ago" is running (`agent::keep_fresh`).
    pub ticking: bool,
    /// The camera moves in flight, per picture (`motion::drive`).
    pub cameras: crate::state::archify::motion::Cameras,
    /// One motion state per picture: overlays, fades and the frame requests (`animate::advance`).
    pub motions: crate::state::archify::animate::Motions,
    /// Host paths an export is in flight to (`SaveHostFileAs` sent, no answer yet).
    pub exports: std::collections::HashSet<String>,
    /// Enter in the finder's filter field is wired (`finder::open`, once).
    pub finder_bound: bool,
}

impl Global for Ui {}

pub fn ui(cx: &mut App) -> &mut Ui {
    cx.default_global::<Ui>()
}

/// Read-only access, for a menu builder that has only `&App`.
pub fn peek(cx: &App) -> Option<&Ui> {
    cx.try_global::<Ui>()
}

impl Ui {
    /// The interaction of the picture keyed `key`, created on first use.
    pub fn view(&mut self, key: &str) -> &mut Interaction {
        self.views.entry(key.to_string()).or_default()
    }

    /// A copy of the interaction of `key`, for a frame to read.
    pub fn peek_view(&self, key: &str) -> Interaction {
        self.views.get(key).cloned().unwrap_or_default()
    }

    /// The picture whose node finder is open, if any.
    pub fn open_finder(&self) -> Option<String> {
        self.views
            .iter()
            .find_map(|(key, v)| v.finder.then(|| key.clone()))
    }

    /// The box a diagnostic click left on `key`'s picture and nothing has revealed yet.
    pub fn reveal_pending(&self, key: &str) -> Option<Bounds> {
        self.views.get(key).and_then(|v| v.reveal)
    }

    /// The tab's state, created on first use.
    pub fn tab(&mut self, key: &str) -> &mut Tab {
        self.tabs.entry(key.to_string()).or_default()
    }

    /// The status chip's call for `key`: its own latest, or the unbound one when that is newer
    /// (the agent reading the guide, a validation of an inline document).
    pub fn call_for(&self, key: &str) -> Option<AgentCall> {
        let own = self.tabs.get(key).and_then(|t| t.agent.clone());
        match (own, &self.unbound_call) {
            (Some(own), Some(other)) if other.at > own.at => Some(other.clone()),
            (Some(own), _) => Some(own),
            (None, other) => other.clone(),
        }
    }

    /// What Ask agent starts on for the tab `key` (the file at `rel`, of `diagram_type`): the
    /// agent is told what is wrong with it now. Asking also notes whether it passes now, so the
    /// agent's first passing validation is a news item only if it was failing.
    pub fn ask_for(&mut self, key: &str, rel: &str, diagram_type: Option<&str>) -> Ask {
        let tab = self.tab(key);
        tab.validated = tab.compiled.as_ref().map(|o| o.ok);
        Ask {
            prompt: agent::prompt_for_doc(rel, diagram_type, tab.compiled.as_deref()),
            autostart: true,
        }
    }

    /// The host reported an MCP tool call. Files it under the tab it named (else as unbound) and
    /// returns a notice when it was a validation that turned the document from failing to ok.
    pub fn agent_called(
        &mut self,
        key: Option<String>,
        tool: String,
        ok: bool,
        at: Instant,
    ) -> Option<Notice> {
        let call = AgentCall {
            tool: tool.clone(),
            ok,
            at,
        };
        let Some((key, tab)) = key.and_then(|k| self.tabs.get_mut(&k).map(|t| (k, t))) else {
            self.unbound_call = Some(call);
            return None;
        };
        tab.agent = Some(call);
        if tool != agent::VALIDATE {
            return None;
        }
        let before = tab.validated.replace(ok);
        let name = key.rsplit('/').next().unwrap_or(&key);
        agent::turned_ok(before, ok).then(|| Notice {
            text: format!("{name} now validates (archify_validate passed)."),
        })
    }

    /// Change the preferences; the caller keeps them (`AppState::update_archify_settings`).
    pub fn update_settings(&mut self, change: impl FnOnce(&mut ArchifySettings)) {
        let quality = self.settings.quality.clone();
        let palette = self.settings.palette.clone();
        change(&mut self.settings);
        if self.settings.quality != quality || self.settings.palette != palette {
            self.recompile_all();
        }
    }

    /// The stored preferences arrived (or changed under another window): take them, and the chart
    /// theme with them.
    pub fn load_settings(&mut self, settings: ArchifySettings) {
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        let theme = self.settings.chart_theme.as_deref().map_or(ChartTheme::Auto, ChartTheme::parse);
        crate::ui::archify::theme::set_chart_theme(theme);
        self.recompile_all();
    }

    /// Every tab compiles again the next time it is drawn: the quality override changed.
    fn recompile_all(&mut self) {
        for tab in self.tabs.values_mut() {
            tab.requested = None;
        }
    }

    /// The viewer drew tab `key` holding `text`: a [`Job`] when it has to be compiled, `None` when
    /// a compile of this very text is already scheduled or done. A tab with no result yet runs at
    /// once; an edit waits for a quieter moment.
    pub fn want_compile(&mut self, key: &str, text: &str) -> Option<Job> {
        let hash = hash_of(text);
        let quality = self.settings.quality.clone();
        let palette = self.settings.palette.clone();
        let tab = self.tab(key);
        if tab.requested == Some(hash) {
            return None;
        }
        let due = match (tab.requested, &tab.compiled) {
            (Some(_), Some(_)) => Due::Debounce,
            _ => Due::Now,
        };
        tab.requested = Some(hash);
        tab.seq += 1;
        Some(Job {
            due,
            seq: tab.seq,
            quality,
            palette,
        })
    }

    /// A job's delay is over: still the newest for its tab?
    pub fn is_current(&self, key: &str, seq: u64) -> bool {
        self.tabs.get(key).is_some_and(|t| t.seq == seq)
    }

    /// A run finished. Dropped (`false`) when a newer one has been scheduled since.
    pub fn finish_compile(&mut self, key: &str, seq: u64, outcome: Outcome) -> bool {
        let Some(tab) = self.tabs.get_mut(key) else {
            return false;
        };
        if tab.seq != seq {
            return false;
        }
        if let Some(scene) = &outcome.scene {
            tab.scene = Some(scene.clone());
        }
        tab.compiled = Some(Arc::new(outcome));
        true
    }

    /// The diagnostics panel's toggle.
    pub fn toggle_panel(&mut self, key: &str) {
        let tab = self.tab(key);
        tab.panel_open = !tab.panel_open;
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "a/x.archify";

    fn outcome(scene: Option<Arc<Scene>>) -> Outcome {
        Outcome {
            ok: scene.is_some(),
            profile: "standard",
            diagnostics: Vec::new(),
            scene,
            trace: false,
        }
    }

    #[test]
    fn a_new_tab_compiles_at_once_and_an_edit_waits() {
        let mut ui = Ui::default();
        let first = ui.want_compile(KEY, "a").unwrap();
        assert_eq!(first.due, Due::Now);
        assert!(ui.want_compile(KEY, "a").is_none(), "the same text is not scheduled twice");

        // Nothing has landed yet, so an edit still does not wait.
        assert_eq!(ui.want_compile(KEY, "ab").unwrap().due, Due::Now);

        let job = ui.want_compile(KEY, "abc").unwrap();
        ui.finish_compile(KEY, job.seq, outcome(None));
        assert_eq!(ui.want_compile(KEY, "abcd").unwrap().due, Due::Debounce);
    }

    #[test]
    fn a_job_that_was_edited_again_is_replaced_and_a_stale_answer_dropped() {
        let mut ui = Ui::default();
        let first = ui.want_compile(KEY, "a").unwrap();
        let second = ui.want_compile(KEY, "ab").unwrap();
        assert!(!ui.is_current(KEY, first.seq), "replaced by the second");
        assert!(ui.is_current(KEY, second.seq));
        assert!(!ui.finish_compile(KEY, first.seq, outcome(None)));
        assert!(ui.tabs[KEY].compiled.is_none());
        assert!(ui.finish_compile(KEY, second.seq, outcome(None)));
        assert!(ui.tabs[KEY].compiled.is_some());
        // An answer for a tab nothing has scheduled has nowhere to go.
        assert!(!ui.finish_compile("other", 1, outcome(None)));
    }

    #[test]
    fn the_quality_override_reaches_the_job_and_a_change_compiles_again() {
        let mut ui = Ui::default();
        let job = ui.want_compile(KEY, "a").unwrap();
        ui.finish_compile(KEY, job.seq, outcome(None));
        assert!(ui.want_compile(KEY, "a").is_none());

        ui.update_settings(|s| s.quality = Some("showcase".into()));
        let job = ui.want_compile(KEY, "a").expect("the override changed");
        assert_eq!(job.due, Due::Now);
        assert_eq!(job.quality.as_deref(), Some("showcase"));

        // Other preferences leave the compile alone.
        ui.update_settings(|s| s.motion = !s.motion);
        assert!(ui.want_compile(KEY, "a").is_none());
    }

    #[test]
    fn a_failed_compile_keeps_the_last_picture() {
        let mut ui = Ui::default();
        let scene = Arc::new(ubiq_archify::scene::SceneBuilder::new(10.0, 10.0).build());
        let job = ui.want_compile(KEY, "a").unwrap();
        ui.finish_compile(KEY, job.seq, outcome(Some(scene.clone())));
        let job = ui.want_compile(KEY, "b").unwrap();
        ui.finish_compile(KEY, job.seq, outcome(None));
        assert!(Arc::ptr_eq(ui.tabs[KEY].scene.as_ref().unwrap(), &scene));
        assert!(!ui.tabs[KEY].compiled.as_ref().unwrap().ok);
    }

    #[test]
    fn settings_changes_stick() {
        let mut ui = Ui::default();
        assert_eq!(ui.settings, ArchifySettings::default());
        ui.update_settings(|s| s.quality = Some("standard".into()));
        ui.update_settings(|s| s.motion = false);
        assert_eq!(ui.settings.quality.as_deref(), Some("standard"));
        assert!(!ui.settings.motion);
    }

    #[test]
    fn a_tool_call_is_filed_under_its_tab_or_as_unbound() {
        let mut ui = Ui::default();
        ui.tab(KEY);
        let t0 = Instant::now();
        assert!(ui.agent_called(None, "archify_guide".into(), true, t0).is_none());
        // Unbound: every tab shows it until the tab has a newer call of its own.
        assert_eq!(ui.call_for(KEY).unwrap().tool, "archify_guide");
        let later = t0 + std::time::Duration::from_secs(2);
        ui.agent_called(Some(KEY.into()), "archify_validate".into(), false, later);
        assert_eq!(ui.call_for(KEY).unwrap().tool, "archify_validate");
        assert!(!ui.call_for(KEY).unwrap().ok);
        // A call naming a tab nothing drew is unbound.
        ui.agent_called(Some("gone.archify".into()), "archify_validate".into(), true, later);
        assert_eq!(ui.unbound_call.as_ref().unwrap().tool, "archify_validate");
    }

    #[test]
    fn a_call_is_filed_under_the_tab_of_its_relative_path() {
        let mut ui = Ui::default();
        ui.tab(KEY);
        ui.agent_called(Some(KEY.into()), "archify_render".into(), true, Instant::now());
        assert_eq!(ui.tabs[KEY].agent.as_ref().unwrap().tool, "archify_render");
    }

    #[test]
    fn an_agent_validation_that_turns_a_failing_doc_ok_is_a_notice_once() {
        let mut ui = Ui::default();
        ui.tab(KEY);
        let t = Instant::now();
        let call = |ui: &mut Ui, tool: &str, ok: bool| {
            ui.agent_called(Some(KEY.into()), tool.into(), ok, t)
        };
        // No baseline: the first pass is not news.
        assert!(call(&mut ui, "archify_validate", true).is_none());
        assert!(call(&mut ui, "archify_validate", false).is_none());
        let notice = call(&mut ui, "archify_validate", true).expect("failing then ok");
        assert!(notice.text.contains("x.archify"));
        assert!(call(&mut ui, "archify_validate", true).is_none());
        // Other tools never notify, whatever they report.
        call(&mut ui, "archify_validate", false);
        assert!(call(&mut ui, "archify_guide", true).is_none());
        assert_eq!(ui.tabs[KEY].validated, Some(false));
    }

    #[test]
    fn asking_about_a_failing_doc_sets_the_baseline_and_autostarts() {
        let mut ui = Ui::default();
        let job = ui.want_compile(KEY, "a").unwrap();
        let d = ubiq_archify::diag::Diagnostic::error("schema/required", "title is required")
            .with_subject(ubiq_archify::diag::Subject::of("dataflow").with_path("/meta/title"));
        let mut found = outcome(None);
        found.diagnostics.push(d);
        ui.finish_compile(KEY, job.seq, found);

        let ask = ui.ask_for(KEY, "a/x.archify", Some("dataflow"));
        assert!(ask.autostart);
        assert!(ask.prompt.contains("a/x.archify") && ask.prompt.contains("title is required"));
        let t = Instant::now();
        assert!(
            ui.agent_called(Some(KEY.into()), "archify_validate".into(), true, t)
                .is_some(),
            "it was failing when the agent was asked"
        );
    }

    #[test]
    fn each_picture_keeps_its_own_interaction() {
        let mut ui = Ui::default();
        ui.view("a").focus = Some("n1".into());
        ui.view("b").reach = Reach::Up;
        assert_eq!(ui.peek_view("a").focus.as_deref(), Some("n1"));
        assert_eq!(ui.peek_view("a").reach, Reach::Near);
        assert_eq!(ui.peek_view("b").reach, Reach::Up);
        assert_eq!(ui.peek_view("c"), Interaction::default());
    }

    #[test]
    fn the_panel_is_open_until_toggled() {
        let mut ui = Ui::default();
        assert!(ui.tab(KEY).panel_open);
        ui.toggle_panel(KEY);
        assert!(!ui.tabs[KEY].panel_open);
    }
}
