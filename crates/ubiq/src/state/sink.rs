//! The kitchen sink: the application's own test bench, and the fixtures it draws.
//!
//! **Nothing here belongs to a project.** Every other screen in the window is about a folder the
//! host opened; this one is about Ubiq itself — the editor with no file behind it, each special
//! viewer against a document written here rather than read from disk, and one page holding every
//! primitive the kit and the theme offer. It is where a control is looked at before a screen is
//! built out of it, and where a palette change is checked against everything at once.
//!
//! So the documents are `&'static str` and the state is a handful of demo fields — including the
//! two settings layouts, which hold nav, harness cards and a project colour rather than a
//! document. A fixture that came from the host would make this a project screen; a fixture that
//! came from a file would make it a file screen. The sink is neither, which is the whole reason
//! it can be opened with no project at all.
//!
//! **Nothing here draws and nothing here holds a buffer.** Which layout each document is in lives
//! here; the buffer it is edited in is the window's, beside the other component-library state on
//! [`crate::app::AppState`], because a fixture's buffer is one of the window's fields and not one
//! of a project's files.

use std::collections::HashMap;

use gpui::ScrollHandle;
use ubiq_proto::ids::SshProfileId;
use ubiq_proto::projects::{DroneChange, DroneOrigin};
use ubiq_proto::settings::DronePreset;
use ubiq_proto::work::AgentId;

use crate::state::editor::{FileLanguage, ViewLayout, ViewerKind};
use crate::state::file_picker::{
    Commit, PickKind, PickerCount, PickerNode, PickerOwner, PickerRequest, PickerView,
};

/// One page of the sink. The first four are one document each, drawn by the viewer its name
/// implies; the rest are drawn rather than parsed — the style reference, the file picker raised
/// against a fixture tree, and the two settings layouts composed from the kit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SinkSection {
    Editor,
    Markdown,
    Mermaid,
    Excalidraw,
    Style,
    Files,
    Settings,
    Project,
    Messages,
    A2ui,
    Script,
    Teamsim,
}

impl SinkSection {
    /// The twelve, in the order the strip draws them.
    pub fn all() -> &'static [SinkSection] {
        &[
            SinkSection::Editor,
            SinkSection::Markdown,
            SinkSection::Mermaid,
            SinkSection::Excalidraw,
            SinkSection::Style,
            SinkSection::Files,
            SinkSection::Settings,
            SinkSection::Project,
            SinkSection::Messages,
            SinkSection::A2ui,
            SinkSection::Script,
            SinkSection::Teamsim,
        ]
    }

    /// What the strip's tab says.
    pub fn label(self) -> &'static str {
        SECTION_COPY[self as usize].0
    }

    /// The one line under the page's title: what this page is for testing.
    pub fn note(self) -> &'static str {
        SECTION_COPY[self as usize].1
    }

    /// The document this page draws, or nothing for the page that draws no document.
    pub fn doc(self) -> Option<&'static SinkDoc> {
        docs().iter().find(|doc| doc.section == self)
    }
}

/// The tab and the line under the title, one row per [`SinkSection`], in variant order.
const SECTION_COPY: [(&str, &str); 12] = [
    (
        "Editor",
        "The plain buffer: highlighting, line numbers, folding.",
    ),
    (
        "Markdown",
        "A document with a diagram fence of each kind inside it.",
    ),
    (
        "Mermaid",
        "A diagram the renderer drew, cached by its content.",
    ),
    ("Excalidraw", "A scene painted from its own JSON."),
    (
        "Style",
        "Every token, surface, control and field, on one page.",
    ),
    (
        "Files",
        "The file picker, raised every way a screen can ask for it.",
    ),
    (
        "Settings",
        "Application settings: the nav, the rows, the harness accordion.",
    ),
    (
        "Project",
        "Project settings: the dialog with a nav, a form and a footer.",
    ),
    (
        "Messages",
        "One live conversation, beside the bus traffic behind it.",
    ),
    (
        "A2UI",
        "A surface an agent could send, drawn from the JSON beside it.",
    ),
    (
        "Script",
        "A JavaScript scratchpad, run by the embedded interpreter.",
    ),
    (
        "Teamsim",
        "The teams graph's arrangements, against a scenario you can edit.",
    ),
];

/// One fixture document: what it is called, which page draws it, and its text.
///
/// The name carries the extension, so the viewer and the highlighter are chosen by exactly the
/// rule a real file goes through — [`ViewerKind::of`] and [`FileLanguage::of`] — rather than by a
/// second table that could disagree with it.
pub struct SinkDoc {
    /// What identifies it: the element ids it builds, the buffer the window holds for it, and the
    /// layout it is remembered in.
    pub key: &'static str,
    /// What the header calls it. Written as a file name, because that is what picks its viewer.
    pub name: &'static str,
    pub section: SinkSection,
    pub source: &'static str,
}

impl SinkDoc {
    /// What draws it, by the same rule a file on disk goes through.
    pub fn viewer(&self) -> ViewerKind {
        ViewerKind::of(self.name)
    }

    /// What highlights its source, by the same rule.
    pub fn language(&self) -> FileLanguage {
        FileLanguage::of(self.name)
    }
}

/// The four fixtures, in page order.
pub fn docs() -> &'static [SinkDoc] {
    &[
        SinkDoc {
            key: "sink-editor",
            name: "coordinator.rs",
            section: SinkSection::Editor,
            source: RUST,
        },
        SinkDoc {
            key: "sink-markdown",
            name: "notes.md",
            section: SinkSection::Markdown,
            source: MARKDOWN,
        },
        SinkDoc {
            key: "sink-mermaid",
            name: "flow.mmd",
            section: SinkSection::Mermaid,
            source: MERMAID,
        },
        SinkDoc {
            key: "sink-scene",
            name: "shapes.excalidraw",
            section: SinkSection::Excalidraw,
            source: SCENE,
        },
    ]
}

/// Which modal the style reference has up. Three shapes rather than one, because the differences
/// are the point: a question, a form, and something that will not come back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SinkModal {
    Confirm,
    Form,
    Danger,
}

impl SinkModal {
    pub fn title(self) -> &'static str {
        MODAL_COPY[self as usize].0
    }

    /// What the body says. One paragraph, because a modal that needs two is a panel.
    pub fn note(self) -> &'static str {
        MODAL_COPY[self as usize].1
    }

    /// What the confirming button says. Never "OK": a button says what it does.
    pub fn confirm(self) -> &'static str {
        MODAL_COPY[self as usize].2
    }
}

/// Title, body and confirming button, one row per [`SinkModal`], in variant order.
const MODAL_COPY: [(&str, &str, &str); 3] = [
    (
        "Close this pane?",
        "The harness in it is still running. Closing the pane ends it, and its scrollback goes \
         with it.",
        "Close the pane",
    ),
    (
        "Name this session",
        "A session is a named piece of work with a folder. The name is what the rail, the graph \
         and the board call it.",
        "Create",
    ),
    (
        "Forget this project?",
        "Ubiq forgets the folder, its arrangement and everything it remembered about it. Nothing \
         on disk is touched.",
        "Forget it",
    ),
];

/// The independent facets the style reference's toggle row draws. Four, because that is what the
/// graph's bucket row has and the row has to be tested at the width it is used at.
pub const FACETS: [&str; 4] = ["running", "waiting", "ended", "error"];

/// The values its choice row picks between.
pub const CHOICES: [&str; 3] = ["source", "preview", "split"];

/// The rows its demo menu offers.
pub const MENU_ITEMS: [&str; 4] = ["Claude Code", "Codex", "Gemini CLI", "opencode"];

// ── Application settings ────────────────────────────────────────────

/// The left nav of the application settings page, in the order the mock draws it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsNav {
    Appearance,
    Harnesses,
    AgentDefaults,
    Sessions,
    Keyboard,
    Privacy,
}

impl SettingsNav {
    pub fn all() -> &'static [SettingsNav] {
        &[
            SettingsNav::Appearance,
            SettingsNav::Harnesses,
            SettingsNav::AgentDefaults,
            SettingsNav::Sessions,
            SettingsNav::Keyboard,
            SettingsNav::Privacy,
        ]
    }

    pub fn label(self) -> &'static str {
        SETTINGS_NAV_LABELS[self as usize]
    }
}

/// One label per [`SettingsNav`], in variant order.
const SETTINGS_NAV_LABELS: [&str; 6] = [
    "Appearance",
    "Harnesses",
    "Agent defaults",
    "Sessions & worktrees",
    "Keyboard",
    "Privacy & data",
];

/// Which dropdown on the settings page is the one `MenuId::SinkSettings` is holding open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsMenu {
    Auth,
    Model,
    Thinking,
    Mode,
}

/// One registered harness on the settings page. Copy is a fixture; whether it is on and whether
/// it is open live on [`SettingsDemo`].
pub struct HarnessFixture {
    pub name: &'static str,
    pub kind: &'static str,
    pub auth: &'static str,
    pub account: &'static str,
    pub model: &'static str,
    pub connected: bool,
}

pub const HARNESS_FIXTURES: [HarnessFixture; 5] = [
    HarnessFixture {
        name: "Claude Code — work",
        kind: "Claude Code",
        auth: "OAuth account",
        account: "marco@studio.dev",
        model: "Opus 4.6",
        connected: true,
    },
    HarnessFixture {
        name: "Claude Code — personal",
        kind: "Claude Code",
        auth: "API key",
        account: "sk-ant-\u{2026}4f2c",
        model: "Sonnet 4.5",
        connected: true,
    },
    HarnessFixture {
        name: "Codex — work",
        kind: "Codex",
        auth: "ChatGPT",
        account: "marco@studio.dev",
        model: "gpt-5",
        connected: true,
    },
    HarnessFixture {
        name: "Gemini CLI",
        kind: "Gemini CLI",
        auth: "OAuth account",
        account: "not signed in",
        model: "gemini-3-pro",
        connected: false,
    },
    HarnessFixture {
        name: "opencode",
        kind: "opencode",
        auth: "API key",
        account: "local",
        model: "qwen2.5",
        connected: true,
    },
];

pub const THEME_CHOICES: [&str; 3] = ["system", "dark", "light"];
pub const DENSITY_CHOICES: [&str; 2] = ["comfortable", "compact"];
pub const AUTH_CHOICES: [&str; 3] = ["OAuth account", "API key", "None"];
pub const MODEL_CHOICES: [&str; 3] = ["Opus 4.6", "Sonnet 4.5", "Haiku 4.5"];
pub const THINKING_CHOICES: [&str; 3] = ["Think", "Standard", "Fast"];
pub const MODE_CHOICES: [&str; 3] = ["Plan", "Edit", "Auto"];

/// Permission modes on Agent defaults: the label, and the one line that says what it allows.
pub const PERMISSIONS: [(&str, &str); 3] = [
    (
        "Plan",
        "Reads and searches only. Writes nothing without approval.",
    ),
    (
        "Edit",
        "May edit files in its own session. Asks before running commands.",
    ),
    (
        "Auto",
        "Edits and runs commands without prompting. Use inside a worktree.",
    ),
];

/// What the application settings page holds between frames.
pub struct SettingsDemo {
    pub nav: SettingsNav,
    pub theme: usize,
    pub accent_follows: bool,
    pub density: usize,
    pub font_size: u8,
    pub reduce_motion: bool,
    pub permission: usize,
    pub max_agents: u8,
    pub context_warn: u8,
    pub retry: bool,
    pub idle: u8,
    /// Which harness card is open. Independent of whether it is enabled.
    pub open_harness: Option<usize>,
    pub harness_on: [bool; HARNESS_FIXTURES.len()],
    pub auth: usize,
    pub model: usize,
    pub thinking: usize,
    pub mode: usize,
    pub env: Vec<String>,
    /// Which of the page's pickers `MenuId::SinkSettings` is holding open.
    pub menu: Option<SettingsMenu>,
}

impl Default for SettingsDemo {
    fn default() -> Self {
        Self {
            nav: SettingsNav::Appearance,
            theme: 0,
            accent_follows: true,
            density: 1,
            font_size: 12,
            reduce_motion: false,
            permission: 0,
            max_agents: 6,
            context_warn: 85,
            retry: true,
            idle: 30,
            open_harness: Some(0),
            harness_on: [true, true, true, true, false],
            auth: 0,
            model: 0,
            thinking: 0,
            mode: 0,
            env: vec!["NODE_ENV=development".into()],
            menu: None,
        }
    }
}

impl SettingsDemo {
    pub fn enabled_count(&self) -> usize {
        self.harness_on.iter().filter(|on| **on).count()
    }

    pub fn toggle_harness(&mut self, index: usize) {
        if let Some(on) = self.harness_on.get_mut(index) {
            *on = !*on;
        }
    }

    /// Open this card, or shut it if it is already the open one.
    pub fn toggle_open(&mut self, index: usize) {
        self.open_harness = if self.open_harness == Some(index) {
            None
        } else {
            Some(index)
        };
    }

    pub fn nudge_font(&mut self, delta: i32) {
        self.font_size = (self.font_size as i32 + delta).clamp(8, 24) as u8;
    }

    pub fn nudge_agents(&mut self, delta: i32) {
        self.max_agents = (self.max_agents as i32 + delta).clamp(1, 16) as u8;
    }

    pub fn nudge_warn(&mut self, delta: i32) {
        self.context_warn = (self.context_warn as i32 + delta).clamp(50, 100) as u8;
    }

    pub fn nudge_idle(&mut self, delta: i32) {
        self.idle = (self.idle as i32 + delta).clamp(5, 120) as u8;
    }

    pub fn add_env(&mut self, pair: String) {
        let pair = pair.trim().to_string();
        if pair.is_empty()
            || !pair.contains('=')
            || self.env.iter().any(|existing| existing == &pair)
        {
            return;
        }
        self.env.push(pair);
    }

    pub fn remove_env(&mut self, index: usize) {
        if index < self.env.len() {
            self.env.remove(index);
        }
    }
}

// ── Project settings ────────────────────────────────────────────────

/// The left nav of the project settings dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ProjectNav {
    #[default]
    General,
    Tools,
    /// Which lanes this project's task board draws, and which of them shut themselves when empty.
    Tasks,
    /// Where the project's folder actually is: here, or behind a drone on another machine.
    Remote,
    /// Which folders and repositories the project's documents come from.
    Kb,
    Documentation,
    Integrations,
}

impl ProjectNav {
    pub fn all() -> &'static [ProjectNav] {
        &[
            ProjectNav::General,
            ProjectNav::Tools,
            ProjectNav::Tasks,
            ProjectNav::Remote,
            ProjectNav::Kb,
            ProjectNav::Documentation,
            ProjectNav::Integrations,
        ]
    }

    pub fn label(self) -> &'static str {
        PROJECT_NAV_COPY[self as usize].0
    }

    /// The count the nav prints, when the item has one.
    pub fn count(self) -> Option<u32> {
        PROJECT_NAV_COPY[self as usize].1
    }
}

/// Label and the count beside it, one row per [`ProjectNav`], in variant order.
const PROJECT_NAV_COPY: [(&str, Option<u32>); 7] = [
    ("General", None),
    ("Tools", None),
    ("Tasks", None),
    ("Remote", None),
    // The fixture's root count. The live dialog prints the project's own instead, because the
    // number beside this row is what the section is a list of.
    ("Knowledge base", Some(2)),
    ("Documentation", Some(4)),
    ("Integrations", Some(1)),
];

/// The project the dialog is about. A fixture: the sink has no project behind it.
pub const PROJECT_NAME: &str = "agent-manager";
pub const PROJECT_PATH: &str = "~/dev/agent-manager";
pub const PROJECT_BRANCH: &str = "main";
pub const PROJECT_MARK: &str = "am";
pub const PROJECT_ABOUT: &str = "Tauri + React desktop app that multiplexes coding agents. Rust backend in src-tauri, panels in src/panels.";
pub const PROJECT_ABOUT_LIMIT: usize = 280;
pub const PROJECT_COLOUR: usize = 0;

/// The colour half of a project form: which swatch is picked, the free colour that took over from
/// it, and the picker's own state. One shape, because the sink's dialog and the live one are the
/// same form and the maths behind them has to be the same maths.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColourField {
    /// Index into the project swatches, used while [`Self::custom`] is empty.
    pub swatch: usize,
    /// A free colour the picker committed, as `0xRRGGBB`. Takes over from the swatch.
    pub custom: Option<u32>,
    pub picker_open: bool,
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
}

impl Default for ColourField {
    fn default() -> Self {
        Self {
            swatch: PROJECT_COLOUR,
            custom: None,
            picker_open: false,
            hue: 0.6,
            sat: 0.6,
            val: 0.95,
        }
    }
}

impl ColourField {
    /// What a form with no colour behind it reads as: the picker is shut and nothing is picked.
    pub const NONE: Self = Self {
        swatch: 0,
        custom: None,
        picker_open: false,
        hue: 0.0,
        sat: 0.0,
        val: 0.0,
    };

    /// The colour on screen: the free one if there is one, else the swatch's.
    pub fn rgb(&self) -> u32 {
        self.custom.unwrap_or_else(|| swatch_rgb(self.swatch))
    }

    pub fn set_hsv(&mut self, hue: f32, sat: f32, val: f32) {
        self.hue = hue.clamp(0.0, 1.0);
        self.sat = sat.clamp(0.0, 1.0);
        self.val = val.clamp(0.0, 1.0);
        self.custom = Some(hsv_to_rgb(self.hue, self.sat, self.val));
    }

    pub fn set_rgb(&mut self, rgb: u32) {
        let (hue, sat, val) = rgb_to_hsv(rgb);
        self.hue = hue;
        self.sat = sat;
        self.val = val;
        self.custom = Some(rgb & 0x00ff_ffff);
    }

    pub fn set_swatch(&mut self, index: usize) {
        self.swatch = index;
        self.custom = None;
        self.picker_open = false;
    }

    /// Open the picker on the colour that is showing, or shut it.
    pub fn toggle_picker(&mut self) {
        let open = !self.picker_open;
        if open {
            self.seed_hsv();
        }
        self.picker_open = open;
    }

    /// Point the picker's three sliders at the colour that is showing.
    pub fn seed_hsv(&mut self) {
        let (hue, sat, val) = rgb_to_hsv(self.rgb());
        self.hue = hue;
        self.sat = sat;
        self.val = val;
    }

    /// Take a colour typed into the hex field. Nothing happens when it is the colour already
    /// showing — a field syncing itself must not turn a swatch into a custom colour.
    pub fn apply_hex(&mut self, rgb: u32) -> bool {
        if self.custom == Some(rgb) || (self.custom.is_none() && rgb == swatch_rgb(self.swatch)) {
            return false;
        }
        self.set_rgb(rgb);
        true
    }
}

/// One project swatch as `0xRRGGBB`: the theme's colour, seen the way the picker works in.
pub fn swatch_rgb(index: usize) -> u32 {
    let colour = crate::theme::project_colour(index);
    rgb_from_channels(colour.r, colour.g, colour.b)
}

/// The Remote panel's draft, held between frames until Save sends it.
///
/// One shape for both copies of the form, on [`ColourField`]'s reasoning: the sink page and the
/// live dialog are the same form, so the maths behind them has to be the same maths. The folder is
/// not a field here but an `InputState` on `AppState` — `set_value` needs a window, and this
/// struct is built without one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DroneField {
    /// Whether the project is pinned to a drone. A flag of its own rather than
    /// `profile.is_some()`, so switching back to "runs here" leaves the profile and the folder on
    /// screen to switch back to.
    pub on_drone: bool,
    pub profile: Option<SshProfileId>,
    pub preset: DronePreset,
}

impl DroneField {
    /// Seed the draft from a record's origin, so the form opens on what the project says.
    pub fn from_origin(origin: Option<&DroneOrigin>) -> Self {
        match origin {
            Some(origin) => Self {
                on_drone: true,
                profile: Some(origin.profile),
                preset: origin.preset,
            },
            None => Self::default(),
        }
    }

    /// What Save has to send, given the folder typed beside it and the record as it stands.
    ///
    /// Three answers, which are the three `UpdateProject` can carry: `Set` for a project that
    /// names a drone, `Local` for one brought home, and `None` — nothing said — when the form
    /// agrees with the record, or when it says "on a drone" without yet naming which.
    ///
    /// The linger override rides through untouched: the panel picks a lifetime preset, and a
    /// per-project override in seconds is a value only a record carries.
    pub fn change(&self, root: &str, current: Option<&DroneOrigin>) -> Option<DroneChange> {
        let wanted = match (self.on_drone, self.profile) {
            (true, Some(profile)) => Some(DroneOrigin {
                profile,
                root: root.trim().to_string(),
                preset: self.preset,
                linger_secs: current.and_then(|origin| origin.linger_secs),
            }),
            (true, None) => return None,
            (false, _) => None,
        };
        if wanted.as_ref() == current {
            return None;
        }
        Some(match wanted {
            Some(origin) => DroneChange::Set(origin),
            None => DroneChange::Local,
        })
    }
}

/// What the project settings dialog holds between frames.
#[derive(Default)]
pub struct ProjectDemo {
    pub nav: ProjectNav,
    pub colour: ColourField,
    pub drone: DroneField,
}

impl ProjectDemo {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Parses `#rgb` or `#rrggbb`, with or without the `#`.
pub fn parse_hex(text: &str) -> Option<u32> {
    let text = text.trim();
    let text = text.strip_prefix('#').unwrap_or(text);
    if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match text.len() {
        3 => {
            let n = u32::from_str_radix(text, 16).ok()?;
            let r = (n >> 8) & 0xf;
            let g = (n >> 4) & 0xf;
            let b = n & 0xf;
            Some(((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11))
        }
        6 => u32::from_str_radix(text, 16).ok(),
        _ => None,
    }
}

pub fn hex_string(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0x00ff_ffff)
}

pub fn rgb_from_channels(r: f32, g: f32, b: f32) -> u32 {
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// The HSV conversions live in [`crate::theme`], beside `rgba_of`: `kit::colour_picker` needs the
/// same maths this form does, and the kit names no module under `state/`.
pub use crate::theme::{hsv_to_rgb, rgb_to_hsv};

// ── The file picker's page ──────────────────────────────────────────

/// The folders the picker page can be rooted at: what the picker's `root` does, seen from outside.
pub const PICKER_ROOTS: [(&str, &str); 3] = [
    ("project", ""),
    ("docs", "docs"),
    ("src-tauri", "src-tauri"),
];

/// The prefilters it can be raised with. The first is no prefilter at all, which is what most
/// callers ask for.
pub const PICKER_PATTERNS: [(&str, Option<&str>); 4] = [
    ("everything", None),
    ("*.md", Some("*.md")),
    ("*.rs", Some("*.rs")),
    ("*.json", Some("*.json")),
];

/// What the page's controls hold, and what the last picker handed back.
///
/// Every field is one line of a [`crate::state::file_picker::PickerRequest`], because the page is
/// the request made adjustable: a picker is looked at here in each of the shapes a screen can ask
/// for one, and the readout under the button is the answer that came back.
pub struct PickerDemo {
    pub kind: PickKind,
    pub count: PickerCount,
    pub commit: Commit,
    pub modal: bool,
    /// Which view the dialog opens in. The user may change it once it is up — that is the point of
    /// the toggle — and this is only where it starts.
    pub view: PickerView,
    /// Indices into [`PICKER_ROOTS`] and [`PICKER_PATTERNS`].
    pub root: usize,
    pub pattern: usize,
    /// What came back, and whether the last dialog was dismissed instead of confirmed. `None` is
    /// "nothing has been asked yet", which is not the same as a picker that answered with nothing.
    pub result: Option<Vec<String>>,
    pub dismissed: bool,
}

impl Default for PickerDemo {
    fn default() -> Self {
        Self {
            kind: PickKind::Files,
            count: PickerCount::Multiple,
            commit: Commit::OnButton,
            modal: true,
            view: PickerView::Tree,
            root: 0,
            pattern: 0,
            result: None,
            dismissed: false,
        }
    }
}

impl PickerDemo {
    /// The request the page's controls add up to.
    pub fn request(&self) -> PickerRequest {
        let title = match self.kind {
            PickKind::Files => "Select documentation files",
            PickKind::Folders => "Select a folder",
            PickKind::Either => "Select files or folders",
        };
        PickerRequest::new(PickerOwner::Sink, title)
            .root(PICKER_ROOTS[self.root.min(PICKER_ROOTS.len() - 1)].1)
            .pattern(PICKER_PATTERNS[self.pattern.min(PICKER_PATTERNS.len() - 1)].1)
            .kind(self.kind)
            .count(self.count)
            .commit(self.commit)
            .modal(self.modal)
    }
}

/// The tree the picker page raises a dialog over.
///
/// It is this repository's own shape, written down rather than read: the sink has no project
/// behind it, and a picker with nothing in it demonstrates nothing. Paths are project-relative and
/// the project itself carries the empty one, exactly as a listing from the host would.
pub fn picker_tree() -> Vec<PickerNode> {
    vec![PickerNode::dir(
        "agent-manager",
        "",
        vec![
            PickerNode::dir(
                "docs",
                "docs",
                vec![
                    PickerNode::file("architecture.md", "docs/architecture.md", 12_400),
                    PickerNode::file("conventions.md", "docs/conventions.md", 6_100),
                    PickerNode::file("harnesses.md", "docs/harnesses.md", 9_200),
                    PickerNode::dir(
                        "adr",
                        "docs/adr",
                        vec![
                            PickerNode::file(
                                "0001-worktrees.md",
                                "docs/adr/0001-worktrees.md",
                                3_300,
                            ),
                            PickerNode::file(
                                "0002-session-store.md",
                                "docs/adr/0002-session-store.md",
                                4_700,
                            ),
                            PickerNode::file(
                                "0003-harness-registry.md",
                                "docs/adr/0003-harness-registry.md",
                                5_500,
                            ),
                        ],
                    ),
                ],
            ),
            PickerNode::dir(
                "src",
                "src",
                vec![
                    PickerNode::file("main.rs", "src/main.rs", 1_800),
                    PickerNode::file(
                        "a-name-long-enough-to-need-eliding-in-any-column.rs",
                        "src/a-name-long-enough-to-need-eliding-in-any-column.rs",
                        2_600,
                    ),
                ],
            ),
            PickerNode::dir(
                "src-tauri",
                "src-tauri",
                vec![
                    PickerNode::file("tauri.conf.json", "src-tauri/tauri.conf.json", 2_100),
                    PickerNode::dir(
                        "src",
                        "src-tauri/src",
                        vec![PickerNode::file("lib.rs", "src-tauri/src/lib.rs", 7_400)],
                    ),
                ],
            ),
            PickerNode::file("README.md", "README.md", 4_000),
            PickerNode::file("package.json", "package.json", 2_000),
        ],
    )]
}

/// What the sink remembers between frames: which page is open, which layout each document is in,
/// which modal is up, and the state the style reference's own controls carry.
///
/// The demo fields are here rather than in the drawing code for the reason every other screen's
/// The A2UI page: the example on show, and the surface being used.
///
/// A drawn surface is not a picture of a payload any more — it holds a data model its own inputs
/// write into, one buffer per text field, and a record of what has been sent. All of that is
/// [`crate::state::a2ui::live::Live`], rebuilt whenever the payload is re-parsed, because an id
/// from one payload means nothing in the next.
#[derive(Default)]
pub struct A2uiDemo {
    /// Which example of [`crate::state::a2ui::EXAMPLES`] the picker last chose.
    pub example: usize,
    /// Which half of the page's lower pane is forward: the data model, or what has been sent.
    pub pane: A2uiPane,
    /// The surface, its data model, its fields and its action log.
    pub live: crate::state::a2ui::live::Live,
}

/// The script page: what the last run said, and how it runs.
///
/// The two buffers are the window's, like every other editor state, so what is kept here is only
/// the answer — and nothing at all until the reader presses Run. A page that evaluated on every
/// keystroke would run half-typed programs, which is why this one runs on the button.
///
/// A run is asynchronous: the interpreter answers on a thread of its own, so [`Self::running`] says
/// one is in flight, [`Self::seq`] is bumped on each Run so the answer of an older run is discarded
/// against a newer one, and the panel the script declared through `ubiq.sink.ui` is drawn into a
/// [`crate::state::a2ui::live::Live`] under [`Self::a2ui`].
#[derive(Default)]
pub struct ScriptDemo {
    /// What the last evaluation printed, returned, and how long it took. `None` until Run.
    pub outcome: Option<crate::state::script::ScriptOutcome>,
    /// Whether a run is in flight. Set on Run, cleared when its answer lands.
    pub running: bool,
    /// Bumped on every Run. The answer of an older run is discarded when it lands behind a newer
    /// one, so pressing Run twice does not let the first result overwrite the second.
    pub seq: u64,
    /// Which dialect the Run and Validate buttons use.
    pub dialect: crate::state::script::Dialect,
    /// Which [`SCRIPT_EXAMPLES`] the picker last chose.
    pub example: usize,
    /// The last Validate answer. `None` until Validate is pressed.
    pub report: Option<crate::state::script::SyntaxReport>,
    /// How oxc is pointed at both buffers: what the settings panel writes.
    pub options: crate::state::script::OxcOptions,
    /// Whether that settings panel is disclosed.
    pub settings_open: bool,
    /// Which of the two things the right-hand panel shows: the reference, or the script's panel.
    pub pane: ScriptPane,
    /// The panel the last run declared, kept so a click on it can be replayed against the same
    /// declaration the reader is looking at.
    pub ui: Option<crate::state::script::ScriptUi>,
    /// That panel, live: its data model, the buffer behind each of its text fields, and what it
    /// has sent.
    pub a2ui: crate::state::a2ui::live::Live,
}

/// What the script page's right-hand panel shows.
///
/// The reference and the script's own panel occupy the same half because they are alternatives,
/// not neighbours: a reader is either learning the surface or driving the thing they built with
/// it, and splitting the half again would leave neither enough room to read.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum ScriptPane {
    /// `script::API_DOC`, rendered as Markdown.
    #[default]
    Docs,
    /// The A2UI surface the script declared through `ubiq.sink.ui`.
    Panel,
}

impl ScriptPane {
    pub fn label(self) -> &'static str {
        match self {
            ScriptPane::Docs => "Docs",
            ScriptPane::Panel => "Panel",
        }
    }

    pub fn all() -> &'static [ScriptPane] {
        &[ScriptPane::Docs, ScriptPane::Panel]
    }
}

/// One script starter: what the picker calls it, which dialect it runs as, and the two buffers it
/// seeds.
///
/// The provenance is on the page because it is load-bearing: each example exists to demonstrate a
/// different mode of the page — compiled TypeScript, a non-parsing program, a returned payload —
/// and a reader deciding which one a starter is for needs to know which is which.
pub struct ScriptExample {
    pub name: &'static str,
    pub dialect: crate::state::script::Dialect,
    pub origin: &'static str,
    pub prelude: &'static str,
    pub source: &'static str,
}

/// The starters the picker offers, in the order it lists them: the JavaScript ones first, then the
/// TypeScript ones, each group from the plainest to the one that exercises the most.
///
/// Between them they cover every mode the page has that a button alone would not demonstrate —
/// both dialects, each host namespace, a declared panel and its replayed click, a program Validate
/// refuses in each dialect, and the two limits that stop a run.
pub const SCRIPT_EXAMPLES: &[ScriptExample] = &[
    ScriptExample {
        name: "JS — hello",
        dialect: crate::state::script::Dialect::Js,
        origin: "the console, the completion value, and the prelude in front of both",
        prelude: SCRIPT_JS_PRELUDE,
        source: SCRIPT_JS_SOURCE,
    },
    ScriptExample {
        name: "JS — project & tasks",
        dialect: crate::state::script::Dialect::Js,
        origin: "ubiq.project and ubiq.tasks: a snapshot to read, an intent to record",
        prelude: "",
        source: SCRIPT_FACTS_SOURCE,
    },
    ScriptExample {
        name: "JS — ai & search",
        dialect: crate::state::script::Dialect::Js,
        origin: "ubiq.ai and ubiq.search: every call that would act is recorded, not performed",
        prelude: "",
        source: SCRIPT_EFFECTS_SOURCE,
    },
    ScriptExample {
        name: "JS — a panel with a handler",
        dialect: crate::state::script::Dialect::Js,
        origin: "ubiq.sink.ui: switch the right pane to Panel, press the button, watch the replay",
        prelude: "",
        source: SCRIPT_A2UI_SOURCE,
    },
    ScriptExample {
        name: "JS — caught by Validate",
        dialect: crate::state::script::Dialect::Js,
        origin: "a syntax error: Validate names the buffer, the line and the column",
        prelude: "",
        source: SCRIPT_BROKEN_SOURCE,
    },
    ScriptExample {
        name: "JS — stopped by the limit",
        dialect: crate::state::script::Dialect::Js,
        origin: "an endless loop: the interrupt handler stops it and the page keeps drawing",
        prelude: "",
        source: SCRIPT_RUNAWAY_SOURCE,
    },
    ScriptExample {
        name: "TS — compiled, enums and all",
        dialect: crate::state::script::Dialect::Ts,
        origin: "TypeScript compiled to JavaScript before the run, enums lowered rather than erased",
        prelude: SCRIPT_TS_PRELUDE,
        source: SCRIPT_TS_SOURCE,
    },
    ScriptExample {
        name: "TS — generics & narrowing",
        dialect: crate::state::script::Dialect::Ts,
        origin: "type-only syntax that has to disappear: generics, unions, satisfies, as const",
        prelude: SCRIPT_TS_GENERIC_PRELUDE,
        source: SCRIPT_TS_GENERIC_SOURCE,
    },
    ScriptExample {
        name: "TS — a typed panel",
        dialect: crate::state::script::Dialect::Ts,
        origin: "ubiq.sink.ui from TypeScript: the payload typed, the handler typed",
        prelude: SCRIPT_TS_A2UI_PRELUDE,
        source: SCRIPT_TS_A2UI_SOURCE,
    },
    ScriptExample {
        name: "TS — caught by Validate",
        dialect: crate::state::script::Dialect::Ts,
        origin: "TypeScript that does not parse: the same report, in the other dialect",
        prelude: "",
        source: SCRIPT_TS_BROKEN_SOURCE,
    },
];

/// The plainest starter's prelude: one function the program below leans on.
const SCRIPT_JS_PRELUDE: &str = r#"// The prelude runs before the script, in the same context.
function greet(who) {
  return `hello, ${who}`;
}
"#;

/// The plainest starter: the four console levels, a value, and the completion value.
const SCRIPT_JS_SOURCE: &str = r#"console.log(greet("Ubiq"), "on", ubiq.platform(), "at", ubiq.now());
console.info("version:", ubiq.version());
console.warn("a warning reads like this");
console.error("and an error like this");

const rows = [1, 2, 3].map((n) => ({ n, square: n * n }));
console.log(rows);

rows.length;
"#;

/// The read-only namespaces: what the interface serialised before the run started.
const SCRIPT_FACTS_SOURCE: &str = r#"const project = ubiq.project.info();
console.log(project ? `${project.name} at ${project.root}` : "no project open");
console.log("catalogue:", ubiq.project.list().map((p) => p.name));

const tasks = ubiq.tasks.list();
console.log(`${tasks.length} task(s) in flight`);
for (const task of tasks.slice(0, 5)) {
  console.info(`  ${task.status}  ${task.title}`);
}

// An intent: recorded and drawn in the console, performed by nobody.
ubiq.tasks.create({ title: "tidy the docs", project: project?.id });

tasks.length;
"#;

/// The two namespaces whose every useful call is an intent.
const SCRIPT_EFFECTS_SOURCE: &str = r#"console.log("inference reachable:", ubiq.ai.available());
console.log("models:", ubiq.ai.models().map((m) => m.id));

// Recorded, never run — the interpreter performs no effect.
ubiq.ai.ask("summarise the working tree", { model: "local/qwen", maxTokens: 256 });

const last = ubiq.search.last();
console.log("last query:", last?.query ?? "(none)");
console.log("rust hits:", ubiq.search.files(".rs").length);

ubiq.search.text("TODO", { caseSensitive: false });

"see the intents under the console";
"#;

/// The Validate starter: parses at a glance but does not parse. The unbalanced paren is the point.
const SCRIPT_BROKEN_SOURCE: &str = r#"function twice(n) {
  return n * 2
}

twice(2
"#;

/// The limit starter. The interrupt handler stops it, and the page draws the reason.
const SCRIPT_RUNAWAY_SOURCE: &str = r#"// The interrupt handler stops this after the block limit, and the page says so.
// The interface keeps drawing throughout: the interpreter is on a thread of its own.
let n = 0;
while (true) {
  n += 1;
}
"#;

/// The TypeScript starter's prelude: types the script below leans on, compiled before the run.
const SCRIPT_TS_PRELUDE: &str = r#"interface Greeting { who: string }
function greet(g: Greeting): string {
  return `hello, ${g.who}`;
}
"#;

/// The TypeScript starter: type syntax in the program as well as the prelude, and an enum — a
/// runtime construct oxc lowers rather than erases.
const SCRIPT_TS_SOURCE: &str = r#"console.log(greet({ who: "Ubiq" }));

let n: number = 41;
n += 1;

enum Colour { Red, Green }
console.log(Colour[Colour.Green]);
console.log(`compiled: ${n}`);

n;
"#;

/// The generics starter's prelude: a generic function and a discriminated union, both of which
/// have to leave no trace in the JavaScript QuickJS runs.
const SCRIPT_TS_GENERIC_PRELUDE: &str = r#"type Shape =
  | { kind: "circle"; r: number }
  | { kind: "square"; side: number };

function area(shape: Shape): number {
  switch (shape.kind) {
    case "circle": return Math.PI * shape.r ** 2;
    case "square": return shape.side ** 2;
  }
}

function first<T>(items: readonly T[]): T | undefined {
  return items[0];
}
"#;

/// The generics starter: `satisfies`, `as const`, a non-null assertion and a parameter property —
/// four things a stripper gets wrong and a transformer does not.
const SCRIPT_TS_GENERIC_SOURCE: &str = r#"const shapes = [
  { kind: "circle", r: 2 },
  { kind: "square", side: 3 },
] as const satisfies readonly Shape[];

class Point {
  constructor(public readonly x: number, public readonly y: number) {}
  get length(): number { return Math.hypot(this.x, this.y); }
}

console.log(shapes.map((s) => area(s).toFixed(2)));
console.log("first:", first(shapes)!.kind);
console.log("length:", new Point(3, 4).length);

shapes.length;
"#;

/// The typed-panel starter's prelude: the shapes the declaration below is written against.
const SCRIPT_TS_A2UI_PRELUDE: &str = r#"interface SinkEvent {
  name: string;
  surfaceId: string;
  componentId: string;
}

function stamp(): string {
  return ubiq.now().slice(11, 19);
}
"#;

/// The typed-panel starter: the same declaration as the JavaScript one, written in TypeScript.
const SCRIPT_TS_A2UI_SOURCE: &str = r#"const event: SinkEvent | null = ubiq.sink.event();
const clicks: number = event ? 1 : 0;

ubiq.sink.ui(
  {
    createSurface: {
      surfaceId: "typed-panel",
      components: [
        { id: "root", component: "Card", child: "col" },
        { id: "col", component: "Column", children: ["msg", "field", "go"] },
        { id: "msg", component: "Text", text: { path: "/msg" } },
        {
          id: "field",
          component: "TextField",
          label: "your name",
          value: { path: "/name" }
        },
        { id: "label", component: "Text", text: "say hello" },
        {
          id: "go",
          component: "Button",
          child: "label",
          variant: "primary",
          action: { event: { name: "hello" } }
        }
      ],
      dataModel: {
        msg: event ? `${event.name} at ${stamp()}` : "press the button",
        name: "Ubiq"
      },
      sendDataModel: true
    }
  },
  (e: SinkEvent) => console.info("handler saw", e.name, "from", e.componentId)
);

clicks;
"#;

/// The TypeScript Validate starter: a type annotation with nothing after it.
const SCRIPT_TS_BROKEN_SOURCE: &str = r#"interface Row {
  n: number;
  label: string
}

const rows: Row[] = [{ n: 1, label: }];
"#;

/// The panel starter: the script declares a surface and a handler, and a click on it re-runs the
/// script with the event in hand. The counter proves the replay — it is read out of the surface
/// the last run left behind, because nothing else survives one.
const SCRIPT_A2UI_SOURCE: &str = r#"const event = ubiq.sink.event();

ubiq.sink.ui({
  createSurface: {
    surfaceId: "script-panel",
    components: [
      { id: "root", component: "Card", child: "body" },
      { id: "body", component: "Column", children: ["status", "greeting_field", "save_button"] },
      { id: "status", component: "Text", text: { path: "/status" } },
      {
        id: "greeting_field",
        component: "TextField",
        label: "greeting",
        value: { path: "/greeting" }
      },
      { id: "save_label", component: "Text", text: "log it" },
      {
        id: "save_button",
        component: "Button",
        child: "save_label",
        variant: "primary",
        action: { event: { name: "logGreeting" } }
      }
    ],
    dataModel: {
      status: event
        ? `${event.name} fired on ${event.componentId}`
        : "nothing pressed yet — switch the pane to Panel",
      greeting: "bound from the script"
    },
    sendDataModel: true
  }
}, (e) => console.info("handler saw", e.name, "from", e.componentId));

event ? `replayed for ${event.name}` : "declared";
"#;

/// What the A2UI page shows under its payload: the values, or the messages.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum A2uiPane {
    /// The live data model — what a keystroke in the preview just changed.
    #[default]
    Model,
    /// The envelopes the surface has sent, newest first.
    Actions,
}

/// are: a control that cannot hold a value is not being tested, and a value read out of the
/// element tree is not state.
pub struct SinkState {
    pub section: SinkSection,
    /// Per document, by key. A document not in the map is in its viewer's default layout.
    layouts: HashMap<&'static str, ViewLayout>,
    pub modal: Option<SinkModal>,
    /// The toggle row: four independent facets.
    pub facets: [bool; FACETS.len()],
    /// The choice row: exactly one of [`CHOICES`].
    pub choice: usize,
    /// What the stepper holds, and what the meter and the ring report — one value driving three
    /// controls, so a nudge is visible in all of them at once.
    pub level: u8,
    pub disclosed: bool,
    /// Which row of the demo menu was picked.
    pub picked: usize,
    /// Which rows of the demo multi-select are ticked. Indices into [`MENU_ITEMS`], and the
    /// specimen's preselection: the page opens with two of them already on, because a control
    /// that only ever starts empty never shows the state a form loads it in.
    pub multi: Vec<usize>,
    /// The style reference's file-list toggle: tree when true, list when false.
    pub files_tree: bool,
    /// The file picker page: how the next dialog is asked for, and what the last one answered.
    pub picker: PickerDemo,
    /// Application settings: the nav, the rows, the harness accordion.
    pub settings: SettingsDemo,
    /// Project settings: the dialog's nav and the colour swatch.
    pub project: ProjectDemo,
    /// The messages page: which conversation is on the left, and what the tape on the right is
    /// showing.
    pub messages: MessagesDemo,
    /// The A2UI page: which example the picker last chose, and the live surface drawn from it —
    /// its data model, the buffer behind each of its text fields, and what it has sent.
    pub a2ui: A2uiDemo,
    /// The script page: what the last run of the embedded interpreter said.
    pub script: ScriptDemo,
    /// The teamsim page: the loaded scenario, where the arrangement put it, and what the page
    /// remembers about looking at it. The type is
    /// [`crate::state::teamsim::TeamsimDemo`] — the geometry lives beside the scenario it is
    /// derived from, so this module never grows a second layout engine.
    pub teamsim: crate::state::teamsim::TeamsimDemo,
}

impl Default for SinkState {
    fn default() -> Self {
        Self {
            section: SinkSection::Editor,
            layouts: HashMap::new(),
            modal: None,
            facets: [true, true, false, true],
            choice: 1,
            level: 60,
            disclosed: true,
            picked: 0,
            multi: vec![0, 2],
            files_tree: true,
            picker: PickerDemo::default(),
            settings: SettingsDemo::default(),
            project: ProjectDemo::default(),
            messages: MessagesDemo::default(),
            a2ui: A2uiDemo::default(),
            script: ScriptDemo::default(),
            teamsim: crate::state::teamsim::TeamsimDemo::default(),
        }
    }
}

impl SinkState {
    /// Which layout a document is in. A viewer with no preview is only ever its source, which is
    /// the same rule [`crate::state::editor::OpenFile`] follows.
    pub fn layout(&self, doc: &SinkDoc) -> ViewLayout {
        if !doc.viewer().has_preview() {
            return ViewLayout::Source;
        }
        self.layouts
            .get(doc.key)
            .copied()
            .unwrap_or(ViewLayout::default())
    }

    /// Put a document into one of its viewer's layouts. A viewer with no preview keeps its source.
    pub fn set_layout(&mut self, doc: &SinkDoc, layout: ViewLayout) {
        // The fixture page hosts no web session, so `Edit` is not one of its positions however a
        // caller arrives at it.
        if doc.viewer().has_preview() && layout != ViewLayout::Edit {
            self.layouts.insert(doc.key, layout);
        }
    }

    /// The stepper's step, clamped to the range the meter and the ring can report.
    pub fn nudge(&mut self, delta: i32) {
        self.level = (self.level as i32 + delta).clamp(0, 100) as u8;
    }

    /// What the meter draws.
    pub fn fraction(&self) -> f32 {
        self.level as f32 / 100.0
    }
}

/// The messages page's own state: one live conversation beside the bus tape.
///
/// The sink has no project, so the conversation is picked out of whatever the window happens to
/// hold rather than looked up in one. The tape itself is process-wide and lives in
/// [`ubiq_proto::bus`]; nothing of it is copied here — only what is being looked at.
pub struct MessagesDemo {
    /// Which conversation the left half draws. `None` is "the first one there is", so a window
    /// that starts an agent while this page is open fills in without a click.
    pub agent: Option<AgentId>,
    /// What the viewer under the list shows: the harness's own original line when true, the bus
    /// message when false. Side by side they do not fit, so it is a toggle rather than a column.
    pub original: bool,
    /// The entry the viewer is reading, by sequence number. The list picks it; nothing expands in
    /// place, because a formatted message is taller than any row a list can keep.
    pub selected: Option<u64>,
    /// Whether the tail stays in view as entries arrive.
    pub follow: bool,
    pub scroll: ScrollHandle,
    /// The viewer's own scroll, kept apart from the list's: reading down a long message must not
    /// move the row that chose it.
    pub body_scroll: ScrollHandle,
    /// Whether the next conversation the New agent menu starts is this page's to read. Set by the
    /// bench's own *New chat*, taken in `pick_new_agent_menu`.
    pub pending_attach: bool,
    /// Where the last dump landed, until the next one. A path rather than a note: it is what the
    /// reader takes to a shell.
    pub dumped: Option<String>,
    /// Whether the tape's nudges are already being listened to. The subscription is lazy — the
    /// page is a debug bench and most windows never open it — and started exactly once.
    pub subscribed: bool,
}

impl Default for MessagesDemo {
    fn default() -> Self {
        Self {
            agent: None,
            original: false,
            selected: None,
            follow: true,
            scroll: ScrollHandle::new(),
            body_scroll: ScrollHandle::new(),
            pending_attach: false,
            dumped: None,
            subscribed: false,
        }
    }
}

// ── The fixtures ────────────────────────────────────────────────────

/// The plain buffer's document. Rust, because that is what this repository is, and long enough to
/// have something to fold and something to scroll.
const RUST: &str = r#"//! One pane's pseudo-terminal, and the reader that never blocks on the UI.

use std::collections::HashMap;

use crate::pty::Pty;
use ubiq_proto::ids::PaneId;

/// How many bytes one read hands the bus at a time. A slow interface must never
/// stall the harness, so the reader owns the buffer and the bus takes a copy.
const CHUNK: usize = 8 * 1024;

pub struct Coordinator {
    panes: HashMap<PaneId, Pane>,
    chunk: usize,
}

struct Pane {
    pty: Pty,
    cols: u16,
    rows: u16,
    running: bool,
}

impl Coordinator {
    pub fn new() -> Self {
        Self {
            panes: HashMap::new(),
            chunk: CHUNK,
        }
    }

    /// Geometry has to reach the kernel, not just the interface: a pane that
    /// resized while its harness believes the old size is the corruption bug.
    pub fn resize(&mut self, pane: PaneId, cols: u16, rows: u16) -> bool {
        let Some(pane) = self.panes.get_mut(&pane) else {
            return false;
        };
        if (pane.cols, pane.rows) == (cols, rows) {
            return false;
        }
        pane.cols = cols;
        pane.rows = rows;
        pane.pty.resize(cols, rows);
        true
    }

    /// The UI closes the tab on `PaneExited`; the mock only records that the harness stopped.
    pub fn exited(&mut self, pane: PaneId, code: i32) {
        if let Some(pane) = self.panes.get_mut(&pane) {
            pane.running = false;
            tracing::info!(?code, "harness exited");
        }
    }
}
"#;

/// The Markdown document. It carries a fence of each diagram kind, because the fence renderers are
/// the part of the Markdown viewer with somewhere to go wrong.
const MARKDOWN: &str = r##"# Kitchen sink

A document with **bold**, *italic*, `inline code`, a [link](https://example.invalid)
and a footnote-free paragraph long enough to wrap at the width the panel gives it.

> A blockquote, for the one place a document quotes something.

## A list, and a list that is checked

- The rail selects what the middle of the window is for
- A pane is a terminal, not a text buffer
- Exactly one pane holds focus

1. Ask the host
2. Wait for the answer
3. Draw what came back

- [x] Tokens have a value in both palettes
- [ ] Focus across split panes
- [ ] The console's level floor

## A table

| Token group | Accessors | For |
|---|---|---|
| Surface | `app_bg`, `pane_bg`, `surface` | The stack of backgrounds |
| Text | `text`, `text_muted`, `text_faint` | Copy, at three tiers |
| Status | `danger`, `success`, `warning`, `info` | What something is doing |

## A code fence

```rust
pub fn is_drawn(&self, at: Visibility) -> bool {
    match self {
        PanelKind::Explorer => at.is_ide,
        PanelKind::Logs => true,
        _ => false,
    }
}
```

## A Mermaid fence

```mermaid
flowchart LR
  U[UI] -->|PaneInput| C(Coordinator)
  C -->|PaneOutput| U
  C --> P[[pseudo-terminal]]
  P --> H{{harness}}
```

## An Excalidraw fence

```excalidraw
{"type":"excalidraw","version":2,"elements":[
{"id":"a","type":"rectangle","x":0,"y":0,"width":150,"height":60,
 "strokeColor":"#1971c2","backgroundColor":"#a5d8ff","fillStyle":"solid"},
{"id":"b","type":"text","x":18,"y":20,"width":120,"height":24,
 "text":"a fenced scene","fontSize":18,"strokeColor":"#1e1e1e"},
{"id":"c","type":"arrow","x":160,"y":30,"width":80,"height":0,
 "points":[[0,0],[80,0]],"strokeColor":"#1e1e1e"},
{"id":"d","type":"ellipse","x":250,"y":0,"width":90,"height":60,
 "strokeColor":"#2f9e44","backgroundColor":"#b2f2bb","fillStyle":"solid"}
]}
```
"##;

/// The Mermaid document. A flowchart rather than a sequence, because a flowchart exercises the
/// layout engine and is the shape the architecture is drawn in.
const MERMAID: &str = r#"flowchart TD
  Bin[ubiq-app] --> Host[coordinator]
  Bin --> Ui[window]
  Ui -->|PaneInput, Resize| Bus{{the bus}}
  Bus -->|PaneOutput, Exited| Ui
  Bus --> Host
  Host --> Pty[pseudo-terminal]
  Pty --> Harness[Claude Code]
  Pty --> Harness2[Codex]
  Host --> Catalog[(project catalogue)]
"#;

/// The scene. One element of every kind the parser draws, so paint order, fills, dashes, arrowheads
/// and text alignment are all on screen at once.
const SCENE: &str = r##"{
  "type": "excalidraw",
  "version": 2,
  "source": "ubiq kitchen sink",
  "appState": { "viewBackgroundColor": "transparent" },
  "elements": [
    {
      "id": "frame", "type": "frame", "name": "the window",
      "x": 0, "y": 0, "width": 520, "height": 300,
      "strokeColor": "#868e96", "backgroundColor": "transparent"
    },
    {
      "id": "rect", "type": "rectangle",
      "x": 40, "y": 60, "width": 160, "height": 80,
      "strokeColor": "#1971c2", "backgroundColor": "#a5d8ff",
      "fillStyle": "solid", "strokeWidth": 2
    },
    {
      "id": "rect-label", "type": "text",
      "x": 40, "y": 90, "width": 160, "height": 25,
      "text": "rectangle", "fontSize": 18, "textAlign": "center",
      "strokeColor": "#1971c2"
    },
    {
      "id": "diamond", "type": "diamond",
      "x": 260, "y": 40, "width": 120, "height": 120,
      "strokeColor": "#e8590c", "backgroundColor": "#ffd8a8",
      "fillStyle": "solid", "strokeStyle": "dashed"
    },
    {
      "id": "diamond-label", "type": "text",
      "x": 260, "y": 88, "width": 120, "height": 25,
      "text": "diamond", "fontSize": 18, "textAlign": "center",
      "strokeColor": "#e8590c"
    },
    {
      "id": "ellipse", "type": "ellipse",
      "x": 410, "y": 60, "width": 80, "height": 80,
      "strokeColor": "#2f9e44", "backgroundColor": "#b2f2bb",
      "fillStyle": "solid", "opacity": 70
    },
    {
      "id": "arrow-1", "type": "arrow",
      "x": 200, "y": 100, "width": 60, "height": 0,
      "points": [[0, 0], [60, 0]],
      "strokeColor": "#1e1e1e", "endArrowhead": "arrow"
    },
    {
      "id": "arrow-2", "type": "arrow",
      "x": 380, "y": 100, "width": 30, "height": 0,
      "points": [[0, 0], [30, 0]],
      "strokeColor": "#1e1e1e",
      "startArrowhead": "arrow", "endArrowhead": "arrow"
    },
    {
      "id": "line", "type": "line",
      "x": 40, "y": 200, "width": 450, "height": 40,
      "points": [[0, 0], [150, 40], [300, 0], [450, 40]],
      "strokeColor": "#7048e8", "strokeWidth": 2, "strokeStyle": "dotted"
    },
    {
      "id": "caption", "type": "text",
      "x": 40, "y": 256, "width": 450, "height": 20,
      "text": "frame, shapes, connectors, then text — paint order is by kind",
      "fontSize": 14, "fontFamily": 3, "strokeColor": "#868e96"
    }
  ]
}
"##;
