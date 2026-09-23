//! The shell's own state: which rail mode is active, and which of the window's
//! single-open-at-a-time menus is down.
//!
//! Where the panels are is not here. The window's arrangement is the dock's own — see
//! [`super::dock`] for what a panel is and where it may sit — and asking the dock is the only way
//! to know whether a region is on screen, because the user can empty one by dragging.
//!
//! Which projects exist and which window holds which is not here — that is process-wide, and lives
//! in [`super::windows`]. What stays is what belongs to this window alone: what was typed into the
//! picker's and the explorer's search fields, and which project's close is waiting on an answer.
//!
//! Nothing about version control is here. The branch, the ahead and behind counts and the
//! working-tree totals were invented, and a fact nobody can answer for is not drawn at all.

use gpui::SharedString;
use ubiq_proto::files::RelatedFile;
use ubiq_proto::ids::{KbSourceId, PaneId, ProjectId, TaskId};
use ubiq_proto::mcp::McpInfo;
use ubiq_proto::messages::{AccountInfo, AgentTypeInfo, ProfileInfo, ShellInfo};
use ubiq_proto::tools::ListedTool;
use ubiq_proto::work::AgentId;

use crate::state::PanelKind;
use crate::state::clone::CloneState;
use crate::state::remote::RemoteConnectState;
use crate::state::remote_hosts::RemoteManagerState;
use crate::state::settings::SettingsState;
use crate::state::sink::{ColourField, DroneField, ProjectNav};
use crate::theme::ThemeId;

/// The left rail's destinations. `Control`, `Ide`, `Git`, `Agents`, `Teams`, `TeamsAll`,
/// `TeamsOld`, `Tasks` and `Sink` are built; the rest render an empty page.
///
/// `Agents` and `TeamsOld` are two screens over the same records, and the split is the point.
/// `Agents` is where the user *talks to* the agents — parallel columns, one conversation each.
/// `TeamsOld` is where the user *arranges* them — the graph of who spawned whom and which task
/// each card serves.
///
/// `Teams` is the new mode standing beside it: a clone of `TeamsOld`'s screen and state, kept
/// independent so the two can drift apart wave by wave. `TeamsOld` is not removed and not
/// redirected — it keeps drawing exactly what it always has, under its old label wrapped in
/// brackets, until the waves after this one either fold it away or replace it outright.
///
/// `Teams` and `TeamsAll` are the same screen over two spans, and which entry the rail is on is
/// the whole of what decides the span — there is no switch on the canvas. `Teams` is the active
/// project's agents and sits in the PROJECT group with the other views onto one project;
/// `TeamsAll` draws every open project's agents on one canvas, which is a fact about the window
/// rather than about any project, so it sits in the APP group beside `Control` and `Sink`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum RailMode {
    Control,
    Ide,
    Git,
    Agents,
    Teams,
    TeamsAll,
    TeamsOld,
    Kb,
    Tasks,
    /// The kitchen sink: the application's own test bench. The one mode with no project behind it
    /// at all — see [`super::sink`].
    Sink,
}

impl RailMode {
    /// Whether this mode is the IDE. The explorer, the open files and the chat belong to it.
    pub fn is_ide(self) -> bool {
        self == RailMode::Ide
    }

    /// Whether a pane started here has an edge region to land in that the user is looking at.
    ///
    /// Every mode but two is a view onto a project's work — `TeamsAll` onto several projects' at
    /// once — and the bottom region is where its terminals live. `Control` reports the running
    /// host and `Sink` is the application's own test bench —
    /// neither is about a project's folder, and a pane started from one has nowhere to be seen.
    /// So starting a runner from either moves the window to the IDE first; see
    /// `AppState::reveal_pane_region`.
    pub fn has_pane_region(self) -> bool {
        !matches!(self, RailMode::Control | RailMode::Sink)
    }

    /// The mode's own name, lowercased — what `rail.<slug>` binds a help page to.
    ///
    /// Taken from the variant rather than written out beside it, so a renamed mode renames its
    /// context key instead of quietly unbinding the page that claimed the old one. The packer
    /// reads the same variants out of this file.
    pub fn slug(self) -> String {
        format!("{self:?}").to_lowercase()
    }

    pub fn label(self) -> &'static str {
        match self {
            RailMode::Control => "Control",
            RailMode::Ide => "IDE",
            RailMode::Git => "Git",
            RailMode::Agents => "Agents",
            RailMode::Teams => "Teams",
            RailMode::TeamsAll => "All Teams",
            RailMode::TeamsOld => "[Teams]",
            RailMode::Kb => "KB",
            RailMode::Tasks => "Tasks",
            RailMode::Sink => "Sink",
        }
    }

    /// The one-line note the empty page shows for a mode that is not built yet. `Control` keeps
    /// one because the rail's tooltip prints it too, and it now says what the screen actually
    /// draws rather than what it was going to.
    pub fn note(self) -> &'static str {
        match self {
            RailMode::Control => "What this Ubiq is doing, and what its agents have spent.",
            RailMode::Ide => "",
            RailMode::Git => "What version control knows about this project.",
            RailMode::Agents => "The agents running in this project, one column each.",
            RailMode::Teams => "How the agents are arranged, and which task each serves.",
            RailMode::TeamsAll => "Every open project's agents, arranged on one canvas.",
            RailMode::TeamsOld => "How the agents are arranged, and which task each serves.",
            RailMode::Kb => "Notes and documents the agents can read.",
            RailMode::Tasks => "Work queued for the agents in this session.",
            RailMode::Sink => "The application's own test bench.",
        }
    }

    /// Every mode, in the order the rail draws them.
    pub fn every() -> impl Iterator<Item = RailMode> {
        Self::groups()
            .iter()
            .flat_map(|(_, modes)| modes.iter().copied())
    }

    /// The modes that belong to a project, in the order the rail draws them — the "PROJECT"
    /// group, as opposed to `Control`, `TeamsAll` and `Sink`, which are the "APP" group and are
    /// not about one. This is what a digit shortcut for "the current project's mode" counts:
    /// `ctrl-1` is
    /// the first of these that is enabled, not `Control`.
    pub fn project_modes() -> impl Iterator<Item = RailMode> {
        Self::groups()
            .iter()
            .find(|(label, _)| *label == "PROJECT")
            .into_iter()
            .flat_map(|(_, modes)| modes.iter().copied())
    }

    /// The rail groups, in the order they are drawn.
    pub fn groups() -> &'static [(&'static str, &'static [RailMode])] {
        &[
            (
                "APP",
                &[RailMode::Control, RailMode::TeamsAll, RailMode::Sink],
            ),
            (
                "PROJECT",
                &[
                    RailMode::Ide,
                    RailMode::Git,
                    RailMode::Agents,
                    RailMode::Teams,
                    RailMode::TeamsOld,
                    RailMode::Kb,
                    RailMode::Tasks,
                ],
            ),
        ]
    }
}

/// What one picker row has expanded into. Only one row at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowAction {
    ConfirmForget,
}

/// Why the project settings dialog is up.
#[derive(Clone, Debug)]
pub enum ProjectSettingsMode {
    /// A folder has been chosen and is not in the catalogue yet.
    Create { path: String },
    /// The project this window is showing.
    Edit { project: ProjectId },
}

/// The project settings dialog, when it is up over the workbench.
///
/// Name, description and hex live in the window's input entities and are filled on the next
/// frame — `set_value` needs a window, and the folder chooser does not come with one.
pub struct ProjectSettings {
    pub mode: ProjectSettingsMode,
    pub colour: ColourField,
    /// The Remote panel's draft, seeded from the record when the dialog opens.
    pub drone: DroneField,
    /// The dialog's own nav, starting on General. The sink page keeps its separate
    /// [`crate::state::sink`] nav — a dialog left on Tools must not reopen the sink there.
    pub nav: ProjectNav,
}

/// The "All projects" modal, while it is up.
///
/// A picker row is one line, and the picker itself only ever shows nine of history before it
/// hands off — this is where the rest of the catalogue is read, searched and acted on. Its own
/// filter rather than a share of `project_filter`: the two fields are on screen at different
/// times, but a modal raised over the picker must not go blank because the row that opened it
/// had typed something into the other one.
#[derive(Default)]
pub struct AllProjectsState {
    pub filter: String,
}

/// Which question the size-preset name prompt is asking.
///
/// Both arms are answered by `kit::prompt_modal` over the same field, which is why one state
/// carries both rather than a prompt per screen — naming is never hand-rolled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizePrompt {
    /// Save the axes as they stand under a new name. A name already in use replaces that preset
    /// rather than adding a second one under it.
    Save,
    /// Rename a saved preset. Only reachable from the Size settings section — a popover is not
    /// where a list is maintained.
    Rename { name: String },
}

/// The theme editor, while it is up: which theme is being written and which of its tokens the
/// colour picker is pointed at.
///
/// **Editing a theme wears it.** There is no draft: the editor writes straight through to the
/// theme in `custom_themes` and the window re-resolves, which is what makes the specimen strip a
/// specimen of the window rather than a second renderer's idea of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeEditor {
    pub theme: ThemeId,
    /// A key in `theme::EDITABLE_TOKENS`.
    pub token: &'static str,
}

/// Which question the theme name prompt is asking. Beside [`SizePrompt`] and for its reason:
/// naming is `kit::prompt_modal`, never a hand-rolled field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemePrompt {
    /// Name a theme about to be forked from `base`. It does not exist until this is answered.
    New { base: ThemeId },
    /// Rename one that does.
    Rename { theme: ThemeId },
}

/// Every menu in the window. Exactly one may be open, so the shell keeps a single `Option`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    Project,
    LogSubsystem,
    LogLevel,
    /// The task panel's session picker. Priority and shape are pill rows rather than menus, because
    /// three fixed values read better as the report and the control at once.
    TaskSession,
    /// The task panel's label `+`: which of the project's labels to put on, or a name that does
    /// not exist yet and the swatch to give it. A menu rather than a pill row, because unlike a
    /// kind the list is as long as the project's own vocabulary and it grows.
    TaskLabels,
    /// The task panel's parent breadcrumb: which eligible task to belong to, or none.
    TaskParent,
    /// The task panel's reference `+`: which other task to link as a reference.
    TaskReferences,
    /// One agents-screen column's `+`: which benched agent to group into it. It carries the
    /// column, because a row of columns each has one and only one may be open.
    AgentBench(usize),
    /// The clone modal's two dropdowns: whose repositories are listed, and which branch the
    /// clone checks out. Two ids rather than one carrying a discriminant, because they are two
    /// controls and only one menu in the window is open at a time anyway.
    CloneConnection,
    CloneBranch,
    /// The feedback modal's one dropdown: what the report is about.
    FeedbackKind,
    /// The style reference's demo dropdown. It picks nothing: the sink is where a control is
    /// looked at, and one menu in the window has to be openable with no project behind it.
    SinkPicker,
    /// The style reference's demo multi-select. Its own id beside `SinkPicker` because both are
    /// drawn on the same page and only one menu in the window is open at a time.
    SinkMulti,
    /// The style reference's demo heading navigator — `kit::md_navigator`'s own specimen, its id
    /// beside `SinkPicker` and `SinkMulti` for the same reason.
    SinkMdNav,
    /// The A2UI page's example picker: which surface the preview draws.
    SinkA2ui,
    /// The script page's example picker: which starter the buffers are seeded from.
    SinkScript,
    /// The teamsim page's two dropdowns: which scenario is loaded, and which arrangement placed it.
    /// Two ids rather than one carrying a discriminant, for the same reason the clone modal's are.
    SinkTeamsimPreset,
    SinkTeamsimAlgo,
    /// A dropdown on the settings page. Which one is `SinkState::settings.menu`.
    SinkSettings,
    /// The explorer's right-click menu. Which row (or the empty panel) is on `ExplorerState::menu`.
    Explorer,
    /// The KB explorer's right-click menu. Which row is on `KbState::menu`. Its own id rather than
    /// `Explorer` reused, because the two panels can both be on screen and only one menu is open.
    Kb,
    /// The status bar's size popover: the presets, the two axis sliders, and Save/Reset. An
    /// anchored panel rather than a modal — it is a menu, peeled by the same Escape and the same
    /// outside click every other menu is, and it carries no number anywhere.
    Size,
    /// A tab's right-click menu — a file, a terminal or a chat tab. Which panel it opened on, and
    /// where, is `WorkbenchState::tab_menu`.
    Tab,
    /// The new-pane control's chevron menu: which shell a pane runs, and the console. Where it
    /// opened is `WorkbenchState::new_pane_menu`.
    NewPane,
    /// The titlebar's run chevron, beside the play triangle: every runnable tool this project
    /// offers. Where it opened is `WorkbenchState::run_tool_menu`.
    RunTool,
    /// The titlebar's overflow chevron: the rarely-used commands moved off the strip to make room
    /// for it — remote connect, web export, window capture and settings. Where it opened is
    /// `WorkbenchState::overflow_menu`.
    Overflow,
    /// The titlebar's own new-project chevron, beside the `+` next to the project picker: the same
    /// three ways in the picker offers at its foot — add, clone, remote. Where it opened is
    /// `WorkbenchState::new_project_menu`.
    NewProject,
    /// The `+` menu every surface that hosts a conversation raises: *New agent*, which opens the
    /// form, and *Attach existing agent*, which lists what is already running. Where it opened,
    /// which surface asked and which of its two stages is drawn is
    /// `WorkbenchState::new_agent_menu`.
    NewAgent,
    /// The Teams toolbar's `+ Add agent`: which of the projects this window holds the agent
    /// starts in. Its own id rather than `NewAgent` reused, because it is a different question —
    /// that menu asks *what* to start, this one asks *where* — and it is the step before the form
    /// rather than a stage of it. Drawn only when the window holds more than one project: a
    /// choice of one is not a choice, and there the button raises the form outright.
    TeamsAddAgent,
    /// The Teams toolbar's states filter: which buckets the canvas draws. A `kit::MultiPicker`
    /// rather than the pill row it replaces — the buckets are the one filter on that row where
    /// several values are on at once, and four pills were four controls saying what one summary
    /// says. Its own id because the row's other two filters are not menus at all.
    TeamsBuckets,
    /// One conversation's three-dots lifecycle menu (Stop, Unload, Resume, Delete), by the agent
    /// it belongs to — several conversations can be on screen at once, each with its own. Where
    /// it opened is `WorkbenchState::conversation_menu`.
    ConversationLifecycle(AgentId),
    /// The orchestration toolbar's arrangement dropdown: which of `layout::Algo` the graph lays
    /// itself out in. No position of its own — one menu in the window is open at a time, and this
    /// one hangs off its own trigger.
    GraphLayout,
    /// The settings page's own arrangement dropdown: which of `layout::Algo` a fresh Teams graph
    /// opens in. Its own id rather than `GraphLayout` reused — that one names the toolbar's own
    /// trigger, and the two can never be open together anyway, but conflating a live pick with a
    /// stored default is the kind of thing worth a name of its own.
    TeamsDefaultAlgo,
    /// The git repository selector: which repository's git view to show when a project has multiple
    /// repositories (submodules or nested repositories).
    GitRepo,
    /// The history's branch picker: which ref the commit list is walking.
    GitBranch,
    /// A right-click on a Git screen row — a changed path, a commit or a ref. Which row, and where,
    /// is `GitView::menu`.
    Git,
    /// The status bar's file-kind readout: which viewer draws the open tab. It hangs off its own
    /// trigger and carries no position, and the override it picks lasts only as long as the tab.
    ViewerKind,
    /// The picture behind a chip on a sent turn. A menu rather than a modal because it answers
    /// nothing and blocks nothing — the same Escape and the same outside click peel it as every
    /// other anchored panel, so it needs no rung in `cancel_dialog`. What it is showing is
    /// `WorkbenchState::attachment_preview`.
    AttachmentPreview,
    /// The markdown viewer header's customisation popover (T-118, proposal §12): width preset,
    /// density, minimap visibility and side. `kit::popover`, not a modal — the same Escape and
    /// outside click every other anchored panel takes.
    MdOptions,
    /// The markdown viewer header's heading navigator (T-124) — the document's structure, offered
    /// in all four of markdown's layouts. `kit::md_navigator`'s anchored panel, on `MdOptions`'s
    /// own terms. The plan dialog's own navigator is *not* this: it is a fact about the document
    /// on screen (`DocumentEditor::nav_open`), raised inside a modal.
    MdNavigator,
}

/// The file a chip on a sent turn was clicked to look at, and what has arrived of it.
///
/// **The bytes are not held by the transcript.** A sent attachment is a path, and the picture
/// behind it is read on demand and kept only while the panel is up: a transcript that carried
/// every image it had ever mentioned would grow without bound for a panel the reader opens once.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentPreview {
    /// Which conversation's turn the chip is on, and which entry of it — what the element id the
    /// panel anchors to is built from, so two transcripts on screen cannot both claim it.
    pub agent: AgentId,
    pub attachment: u64,
    /// Which project was asked to read it, where one was — `None` for an absolute path, which is
    /// read here and asks nobody.
    ///
    /// **The path alone does not identify the answer.** `src/lib.rs` is a path two open projects
    /// can both have, and a late `ProjectFileContents` for the one this panel is not about would
    /// otherwise fill it with the wrong file's bytes.
    pub project: Option<ProjectId>,
    /// The path as the attachment holds it: project-relative, or absolute for a file that was
    /// pasted in from outside every project this window holds.
    pub path: String,
    /// How big the attachment said it was, for the note drawn when there is no picture.
    pub size: Option<u64>,
    /// What the read answered. `None` is still in flight, and is drawn as such rather than as an
    /// empty panel — an empty panel and a file that never arrived look identical.
    pub bytes: Option<Vec<u8>>,
    /// Why there is nothing to draw, where the read said so.
    pub failed: Option<String>,
}

/// One row of the new-pane control's menu, in the order it is drawn.
///
/// The rows are here rather than in the module that paints them because the pick is matched by
/// position: the menu and the action behind it read the same list, so a row that is not offered
/// cannot be picked by an index that has shifted under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewPaneRow {
    /// The heading over the detached group, drawn only when it has at least one row under it.
    /// Disabled and unclickable — the same decoration `HarnessChoice::Label` is — but still a row,
    /// because the pick that follows it is an index into this very list.
    DetachedHeading,
    /// A pane no panel currently draws, by its index into the detached list the caller passed
    /// `new_pane_rows` — that list lives on `AppState`, not here, so unlike `Shell` this index
    /// resolves against the caller's own copy rather than a field of `WorkbenchState`.
    Detached(usize),
    /// A shell, by its index in [`WorkbenchState::shells`].
    Shell(usize),
    /// The line between what starts something and what does not.
    Separator,
    /// The console, which is revealed rather than started.
    Console,
}

/// One row of the titlebar's overflow menu, in the order it is drawn.
///
/// Here rather than in the module that paints it, for the same reason [`NewPaneRow`] is: the pick
/// is matched by position, so a row that is not offered must not shift the index of the one after
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverflowRow {
    RemoteConnect,
    WebExport,
    CaptureWindow,
    Help,
    PointAtSomething,
    Settings,
}

/// In-place help while it is up: where the cursor is, and nothing else.
///
/// **The hit target is not stored.** Which name is under the cursor is a question about the frame
/// that was just laid out, answered by `ui::ident::hit_at` as the overlay is built; keeping a copy
/// here would be a second answer that goes stale the moment a panel moves under a still mouse.
/// State holds the gesture — the mode is on, the pointer is there — and the interface holds the
/// geometry, which is the same division the rest of `state/` keeps.
///
/// `cursor` is `None` until the pointer first moves inside the window, which is why the overlay
/// opens with its hint and no highlight rather than with a rectangle around whatever happens to
/// be at the origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HelpTargeting {
    /// The last position the pointer reported, in window coordinates — the same space
    /// `crate::state::ui_id::MarkRect` is recorded in.
    pub cursor: Option<(f32, f32)>,
}

/// One row of the titlebar's new-project menu, in the order it is drawn — the same three ways in
/// as the project picker's foot (`ui::project_menu`'s `add_row`, `clone_row` and `remote_row`),
/// reused rather than restated so the titlebar and the picker never drift apart on what "add a
/// project" offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewProjectRow {
    AddProject,
    CloneProject,
    RemoteProject,
}

/// One row of the harness menu, in the order it is drawn.
///
/// Here rather than in the module that paints it for the same reason [`NewPaneRow`] is: the
/// pick is matched by position, so the menu and the action behind it must read one list.
///
/// A flat list rather than a submenu, because the kit has no submenu and the pick is an index —
/// the same reason `NewPaneRow` flattens shells and harnesses into one sequence.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HarnessChoice {
    /// A harness with no identity to choose from, by its index in
    /// [`WorkbenchState::agent_types`]. What it runs as is then the library's answer — a
    /// profile, or the user's own home.
    Harness(usize),
    /// A harness and the identity to run it as: the pair the interface calls a harness.
    Pair {
        /// Index into [`WorkbenchState::agent_types`].
        harness: usize,
        /// The account id, which is what crosses the wire.
        account: String,
    },
    /// A saved setup, by its index in [`crate::state::settings::SettingsState::profiles`]. It
    /// names its own harness, identity, model and mode — everything the start needs.
    Profile(usize),
    /// A heading or a hairline: drawn, never picked. It holds an index because a menu's rows and
    /// the actions behind them are matched by position, which is what keeps `on_pick(index)`
    /// honest once the list has groups.
    Label(SharedString),
    Separator,
}

/// The question a file gesture is asking, while one is up. One at a time, the rule
/// `AccountDialog` already follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialog {
    /// Naming something new inside `parent`. Empty `parent` is the project's root. `ext` is the
    /// extension the typed name is forced to carry — set from the menu action that raised this, so
    /// a drawing made as `New Excalidraw` cannot become a text file by deleting the suffix.
    New {
        parent: String,
        dir: bool,
        ext: Option<String>,
    },
    /// Renaming `path`, seeded with its leaf name.
    ///
    /// `related` is what [`ubiq_proto::messages::Message::ProjectFileRelated`] answered for
    /// `path` — empty until that answer lands (a folder's own rename never asks; see
    /// `crate::app::AppState::ask_rename`). It only describes the checkbox; `WorkbenchState`'s own
    /// `carry_related` is what the checkbox itself holds, because it has to be toggled without
    /// waiting on a second round trip.
    Rename {
        path: String,
        related: Vec<RelatedFile>,
    },
    /// A tab's own name, typed over whatever it is currently showing — a terminal's pane title or
    /// a chat's agent name. `current` is what the field is seeded with, since neither is a fact
    /// `kind` alone can answer without a window in hand.
    RenameTab { kind: PanelKind, current: String },
    /// Removing `path`. `trash` is false when Shift was held, and the wording and the button say
    /// which one it is rather than leaving the user to know. `related` is [`FileDialog::Rename`]'s
    /// own field, on the same reasoning — empty for a folder, which never asks.
    Remove {
        path: String,
        dir: bool,
        trash: bool,
        related: Vec<RelatedFile>,
    },
    /// A drag that would move `path` into the folder `into`. Only ever raised for a folder.
    Move { path: String, into: String },
    /// An untitled buffer asking where to be saved. `key` is its tab key.
    SaveAs { key: String },
    /// The image Text tool's click, waiting on its string. `key` is its tab key; the point
    /// travels on the capture itself, set by the click that raised this.
    ImageText { key: String },
    /// A file tab holding unsaved changes, asked before its buffer is dropped. `key` is its tab
    /// key.
    DiscardChanges { key: String },
    /// A save the host refused, said out loud. Nothing to answer — the button dismisses it — but
    /// a write that did not happen is not something a dot on a tab can report, and the buffer
    /// still holds every edit. `key` is the tab's key, `reason` what the host gave.
    SaveFailed { key: String, reason: String },
    /// A save onto a path that already holds a file, which the host refused rather than
    /// performed. The only place Ubiq offers to write over one, and it offers it only because the
    /// user is standing here saying where the buffer goes. `key` is the tab's key.
    OverwriteFile { key: String },
    /// ⌘N while the clipboard holds an image: paste it as an untitled picture, or open the
    /// text buffer the keystroke has always meant. Carries no bytes — they are re-read on the
    /// answer, so a per-frame clone never carries them.
    PasteImage,
    /// The window's close, asked while any project it holds has unsaved files, running
    /// terminals or running agents. What each of them holds is counted when the dialog is drawn.
    /// `quitting` is the same question asked for the whole application — ⌘Q — which takes every
    /// window with it.
    CloseWindow { quitting: bool },
    /// Naming something new inside `parent` of a knowledge-base source. The KB's own variants
    /// rather than `New`, `Rename` and `Remove` reused, because every path in that family is
    /// relative to its source and a dialog that forgot which source it was raised on would write
    /// into whichever one happened to be first.
    KbNew {
        source: KbSourceId,
        parent: String,
        dir: bool,
    },
    /// Renaming one entry inside a source, seeded with its leaf name.
    KbRename { source: KbSourceId, path: String },
    /// Renaming the *source* — Ubiq's own label for it, committed through the whole-list
    /// `SetKbSources` write. Nothing on disk moves, which is why it is not `KbRename`.
    KbRenameSource { source: KbSourceId },
    /// Deleting one entry inside a source. There is no Trash arm: the KB family has one delete,
    /// so the confirmation says what goes rather than which of two promises is being made.
    KbRemove {
        source: KbSourceId,
        path: String,
        dir: bool,
    },
    /// One project's close, asked for the same reasons and answered in the same modal — the close
    /// in the project menu takes the window's unsaved files, running terminals and running agents
    /// just as seriously, it only has one project to say it about.
    CloseProject { project: ProjectId },
    /// Where to write a copy of the open plan into the project's working tree — an explicit,
    /// one-shot action, never a continuous mirror. Raised from the plan modal, over it, on
    /// `SaveAs`'s own terms: `rel_path` is resolved and refused the way a save-as's is.
    ExportPlan { task_id: TaskId },
}

/// The brand mark's spin, on whichever empty page is drawing the mark.
///
/// The mark is a watermark, not a control, and it rests still: a spin is an answer to something the
/// user did, or the window's own way of showing it is still awake. Only three numbers are needed to
/// say so, because the curve itself is code — see `ui/mark.rs`.
///
/// **`spinning` is what mounts the animated element at all.** GPUI plays a one-shot animation when
/// the element carrying it first appears, so an always-mounted spinning ring would spin every time
/// the user switched to an empty page. The still ring is a different element, and the spinning one
/// exists only for the length of a spin.
#[derive(Default)]
pub struct MarkState {
    /// Bumped once per spin, and part of the animated element's id, so a spin asked for while one
    /// is running replaces it from the start rather than being swallowed.
    pub spin: u64,
    /// Whether the spinning element is the one being drawn.
    pub spinning: bool,
    /// When the running spin is due to end. Moved forward by a spin asked for mid-spin, which is
    /// what the timer watching it re-reads rather than firing on the first deadline it was given.
    pub until: Option<std::time::Instant>,
    /// When the idle watch may next spin the mark, once the mark has been on screen that long.
    /// Set by the watch itself, to a fresh interval each time — see `app/mark.rs`.
    pub idle_at: Option<std::time::Instant>,
    /// Whether the idle watch is already running. One per window, started the first time the mark
    /// is drawn and never stopped.
    pub idle_watch: bool,
}

pub struct WorkbenchState {
    /// Where the host writes everything down, and whether that is the usual place. The status bar
    /// says so when it is not, because a config root you cannot see is a foot-gun.
    pub config_root: Option<String>,
    pub config_root_is_default: bool,

    pub rail_mode: RailMode,
    pub theme_id: ThemeId,
    /// The interface-scope preference keys this build does not know, carried between the blob it
    /// read and the blob it writes. `remember_interface` builds a fresh `InterfacePrefs`, so
    /// without somewhere to keep them they would be dropped on the first write — see
    /// `state/prefs.rs`'s `rest`.
    pub interface_rest: std::collections::BTreeMap<String, serde_json::Value>,
    /// What the last conversation was started on. Read back from the interface's preferences and
    /// written whenever one starts, so an empty chat tab opens on the last thing that worked
    /// rather than on whatever happens to be first in the list.
    pub last_start: Option<crate::state::prefs::LastStart>,
    /// The size presets the user saved. The built-ins are not in it — they are code, so a build
    /// that retunes one moves it for everybody. See `state/prefs.rs`.
    pub size_presets: Vec<crate::state::prefs::SizePreset>,
    /// The name a preset is being asked for, while the prompt is up. Raised from the size popover
    /// (Save preset…) and from the Size settings section (rename), which is why it is the
    /// window's rather than either screen's.
    pub size_prompt: Option<SizePrompt>,
    /// The themes the user authored. The window's copy of what `theme::set_custom_themes` was
    /// handed — this is what `remember_interface` writes and what the Themes row draws, and the
    /// thread-local beside the palette is what *resolves* one. See `state/prefs.rs`.
    pub custom_themes: Vec<crate::theme::CustomTheme>,
    /// The theme editor, while it is up. Raised from the Appearance section, and painted at the
    /// window root over it the same way every other modal the settings page raises is.
    pub theme_editor: Option<ThemeEditor>,
    /// What a theme is being named, while the prompt is up. Raised by **New theme…** and by the
    /// editor's Rename, which is why it is the window's rather than either surface's.
    pub theme_prompt: Option<ThemePrompt>,
    pub open_menu: Option<MenuId>,
    /// The panel the dock last made the displayed tab of a group. **Pushed from the dock**, where
    /// focus is decided, the way the focused pane is — it is what gives the help ladder its
    /// `panel.<name>` rung.
    pub active_panel: Option<crate::state::dock::PanelKind>,
    /// The brand mark's spin, while one is running. See [`MarkState`].
    pub mark: MarkState,

    /// What was typed into the project menu's search field.
    pub project_filter: String,
    /// A row expanded into a Forget confirmation.
    pub row_action: Option<(ProjectId, RowAction)>,
    /// Project settings, raised over the window to create a project or edit the one on screen.
    pub project_settings: Option<ProjectSettings>,
    /// The clone modal, while it is up. Beside `project_settings` rather than a mode of it: a
    /// clone has no project yet, and the two questions — "which repository" and "what is this
    /// project called here" — are asked in different places.
    pub clone_project: Option<CloneState>,
    /// The feedback modal, while it is up. Beside `clone_project` for its reason: a question
    /// raised over the window, answered once, carrying its own dropdown's open state.
    pub feedback: Option<crate::state::feedback::FeedbackForm>,
    /// Whether this build can send feedback at all, and where to. Asked once at boot and never
    /// again: the destination is compiled into the binary, so it cannot change while the process
    /// runs. Off until the host answers, which is what keeps the send button disabled in a build
    /// that has no destination.
    pub feedback_offer: ubiq_proto::feedback::FeedbackOffer,
    /// Which agent's question is on screen, while the ask dialog is up. Only the view: what has
    /// been filled in lives on the conversation's own record, which is what lets the dialog be
    /// closed and reopened with the drafts intact. See `crate::state::ask`.
    pub ask: Option<crate::state::ask::AskDialog>,
    /// The "All projects" modal, while it is up. Raised from the picker's History group when it
    /// hides more than it shows — beside `clone_project` for the same reason: a question raised
    /// over the window, answered from its own state rather than the picker's.
    pub all_projects: Option<AllProjectsState>,
    /// The annotated document on screen — today always a task's plan, raised from the task panel
    /// for a task carrying a [`ubiq_proto::work::Level`]. One at a time, like `feedback`:
    /// opening another replaces whichever was open. The field keeps its name because the plan is
    /// the only document there is; the type does not, because the surface is not. See
    /// `crate::state::document`.
    pub plan: Option<crate::state::document::DocumentEditor>,
    /// The New agent modal, while it is up. Beside `clone_project` because it is the same kind of
    /// thing: a question raised over the window, answered once, and carrying its own pickers'
    /// open state because a modal is redrawn from state on every frame.
    pub new_agent: Option<crate::state::new_agent::NewAgentForm>,
    /// The new-mission dialog, while it is up. Beside `new_agent` for the same reason: a question
    /// raised over the window, answered once, that ends in a start of its own.
    pub new_mission: Option<crate::state::new_mission::NewMissionForm>,
    /// The "Add source" modal the knowledge base settings raise, while it is up. Beside
    /// `project_settings` rather than inside it for `new_agent`'s reason: it is a question raised
    /// over that page, painted after it, and it carries its own pickers' open state.
    pub kb_source: Option<crate::state::kb::KbSourceForm>,
    /// What a start still has to say to a harness, by the conversation it was started for.
    ///
    /// A start composes a preamble — the subagent ceiling as a directive, and the opening prompt
    /// the form was given — and **does not send it**: a turn sent before the user has said
    /// anything opens the transcript on words the user never wrote. It is held here instead, and
    /// the composer's send path takes it and puts it in front of the first thing the user sends,
    /// so the harness reads it and the transcript does not show it. See
    /// `crate::state::new_agent::fold_preamble`.
    ///
    /// One entry per conversation, taken on first use and never re-added: it is a preamble, not a
    /// standing prefix.
    pub agent_preambles: std::collections::HashMap<AgentId, String>,
    /// The profile a conversation was started from, by the agent it produced — until the harness
    /// (or the user) names the conversation for itself.
    ///
    /// `ProfileInfo::id` **is** the profile's display name (`"what the user named this setup, e.g.
    /// review"`), so the id `Message::StartConversation` already carries is the whole of what a
    /// title needs — no second lookup. Nothing on `WorkAgent` remembers which profile started it
    /// (there is no such field, and none is added for this alone), so the record kept here is
    /// session-only: a reload of the window loses it exactly as it loses every other in-flight
    /// pick, and the title falls back to the harness-label default `refresh_agent_record` always
    /// gave, which is no regression.
    ///
    /// Read wherever a title is drawn, and only while `WorkAgent::summary` is still `None` — the
    /// same signal `refresh_agent_record` sets the moment the harness (or a user rename) actually
    /// names the conversation, so a profile's name never outlives the real one.
    pub agent_started_profile: std::collections::HashMap<AgentId, String>,
    /// The "Connect to a remote host" modal, while it is up. Beside `clone_project` for the same
    /// reason: raised from the titlebar rather than from settings, and answering a question that
    /// has nothing to do with any project on screen.
    pub remote_connect: Option<RemoteConnectState>,
    /// The "Remote hosts" manager panel, while it is up. The connections it lists live on the
    /// bus and in the settings record — this is only the panel's own test outcomes.
    pub remote_manager: RemoteManagerState,
    /// Application settings, raised from the titlebar's gear. Interface-wide, so it opens with
    /// no project.
    pub settings: SettingsState,
    /// The last thing the host refused to do, shown at the top of the picker until dismissed.
    pub project_error: Option<String>,
    /// The last thing the host refused to do to the work, drawn at the top of the task panel by
    /// `ui::board::form::refusal`. Its own field rather than `project_error`, because that one is
    /// drawn at the top of the project picker and a task that would not move is not a fact about
    /// the catalogue — it has to be said where the user is looking. Cleared by the next thing the
    /// host confirms.
    pub work_error: Option<String>,

    /// What was typed into the explorer's "Go to file…" field. It belongs to the window rather than
    /// to a tree, because one field filters whichever project is on screen.
    pub file_filter: String,
    /// The tab whose right-click menu is open, and where the click went down. The menu is one at a
    /// time, so this is a single `Option` like `open_menu`; the panel names the tab, the point
    /// anchors the `context_menu` over the window.
    pub tab_menu: Option<(PanelKind, (f32, f32))>,
    /// The file question that is up, if one is. One at a time, and drawn from the window's root so
    /// that both the explorer and the editor's save-as reach it.
    pub file_dialog: Option<FileDialog>,
    /// Until when a folder move skips its confirmation, from the dialog's checkbox. In memory and
    /// per window: ten minutes is not a preference, and there is nothing to migrate.
    pub move_unasked_until: Option<std::time::Instant>,
    /// The rename and delete questions' own checkbox — carry `FileDialog::Rename`'s or
    /// `FileDialog::Remove`'s `related` files along, default checked. Reset to `true` every time
    /// one of those two dialogs opens, which is what "default checked" means for a value that
    /// outlives the dialog it belongs to (a second `Rename` must not inherit the first one's
    /// answer).
    pub carry_related: bool,
    /// In-place help's targeting mode. `Some` exactly while it is up.
    ///
    /// An `Option` rather than a bool beside a hovered-target field, for the reason every other
    /// overlay on this struct is one: the mode has state of its own that means nothing when it is
    /// down, and a pair would make "mode off but a cursor still remembered" a state the type
    /// allows. See [`HelpTargeting`].
    pub help_target: Option<HelpTargeting>,
    /// Where the new-pane menu's chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::NewPane`.
    pub new_pane_menu: Option<(f32, f32)>,
    /// Where the titlebar's overflow chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::Overflow`.
    pub overflow_menu: Option<(f32, f32)>,
    /// Where the titlebar's new-project chevron was clicked, which is what anchors the menu over
    /// the window. `Some` exactly while `open_menu` is `MenuId::NewProject`.
    pub new_project_menu: Option<(f32, f32)>,
    /// Where the titlebar's run chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::RunTool`.
    pub run_tool_menu: Option<(f32, f32)>,
    /// The `+` menu, while it is down. `Some` exactly while `open_menu` is `MenuId::NewAgent`.
    pub new_agent_menu: Option<NewAgentMenu>,
    /// Where a conversation's three-dots menu was clicked. `Some` exactly while `open_menu` is
    /// `MenuId::ConversationLifecycle(_)` — the agent it belongs to is carried on that `MenuId`
    /// itself rather than duplicated here.
    pub conversation_menu: Option<(f32, f32)>,
    /// The attachment whose preview panel is up. `Some` exactly while `open_menu` is
    /// `MenuId::AttachmentPreview`.
    pub attachment_preview: Option<AttachmentPreview>,
    /// The conversation Delete asked to confirm — destructive and irreversible, so it is not fired
    /// on the click. `None` when no confirm is up.
    pub confirm_end_conversation: Option<AgentId>,
    /// The pane a tab's Close asked to end — the harness is killed and its screen goes with it,
    /// which is irreversible, so it is not fired on the click. `None` when no confirm is up.
    ///
    /// Only the *destructive* close asks. Hide takes the tab away and leaves the harness running,
    /// so there is nothing to warn about and nothing to confirm.
    pub confirm_close_pane: Option<PaneId>,
    /// The shells the host says this machine has, in the order the menu offers them. Empty until
    /// the host answers — a window asks as it attaches and again every time the menu opens, so a
    /// shell installed since is offered without a restart.
    pub shells: Vec<ShellInfo>,
    /// The agent harnesses the host says can be started here, in the order the menu offers them
    /// above the shells. Empty until the host answers — asked alongside [`Message::ListShells`]
    /// for the same reason: a harness installed since is offered without a restart.
    ///
    /// [`Message::ListShells`]: ubiq_proto::messages::Message::ListShells
    pub agent_types: Vec<AgentTypeInfo>,
    /// The MCP servers this build offers to inject into a harness, from
    /// [`Message::Mcps`]. Empty until the host answers, and every form that offers them draws
    /// nothing rather than inventing a row.
    ///
    /// **One list for the window, not one per form.** What Ubiq can inject is a property of the
    /// build the host is running, not of the harness, the account or the setup being filled in —
    /// it does not change while the window is open, and a copy on each form would be the same
    /// answer stored twice and refreshed at two different moments.
    ///
    /// [`Message::Mcps`]: ubiq_proto::messages::Message::Mcps
    pub mcps: Vec<McpInfo>,
    /// The runnable tools the host lists: the machine-wide rows first, then the current
    /// project's. The titlebar's run chevron offers the applicable ones. Empty until the host
    /// answers — asked every time that menu opens, so a tool added in the settings is offered
    /// without a restart.
    pub tools: Vec<ListedTool>,
    /// Whether the explorer's bookmarks section is open. Furniture, so it is not written down.
    pub bookmarks_open: bool,
}

/// Which surface raised the `+` menu, and so where whatever it produces lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewAgentSurface {
    /// The agents screen: a pick opens a column.
    Agents,
    /// The IDE's chat strip: a pick opens a chat tab, whichever row it was. The strip's `+` means
    /// "add a view" and keeps meaning it — the menu only answers what the view is looking at.
    Chat,
    /// The kitchen sink's bench, which reads one conversation at a time.
    Sink,
}

/// The `+` menu while it is down: where it opened, who asked, and which stage is drawn.
///
/// Two stages rather than one flat list, because the two rows ask different kinds of question —
/// *New agent* raises a form, *Attach existing agent* opens a list that can run to every
/// conversation in the project, and a list that long under a row that is not it reads as the menu
/// having only one real answer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NewAgentMenu {
    pub at: (f32, f32),
    pub surface: NewAgentSurface,
    /// Whether the second stage — the list of conversations — is what is drawn.
    pub attach: bool,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            config_root: None,
            config_root_is_default: true,
            rail_mode: RailMode::Ide,
            theme_id: ThemeId::DARK,
            interface_rest: Default::default(),
            last_start: None,
            size_presets: Vec::new(),
            size_prompt: None,
            custom_themes: Vec::new(),
            theme_editor: None,
            theme_prompt: None,
            open_menu: None,
            active_panel: None,
            mark: MarkState::default(),
            project_filter: String::new(),
            row_action: None,
            project_settings: None,
            clone_project: None,
            feedback: None,
            feedback_offer: ubiq_proto::feedback::FeedbackOffer::default(),
            ask: None,
            all_projects: None,
            plan: None,
            new_agent: None,
            new_mission: None,
            kb_source: None,
            agent_preambles: Default::default(),
            agent_started_profile: Default::default(),
            remote_connect: None,
            remote_manager: RemoteManagerState::default(),
            settings: SettingsState::default(),
            project_error: None,
            work_error: None,
            file_filter: String::new(),
            tab_menu: None,
            file_dialog: None,
            move_unasked_until: None,
            carry_related: true,
            help_target: None,
            new_pane_menu: None,
            overflow_menu: None,
            new_project_menu: None,
            run_tool_menu: None,
            new_agent_menu: None,
            conversation_menu: None,
            attachment_preview: None,
            confirm_end_conversation: None,
            confirm_close_pane: None,
            shells: Vec::new(),
            agent_types: Vec::new(),
            mcps: Vec::new(),
            tools: Vec::new(),
            bookmarks_open: false,
        }
    }
}

impl WorkbenchState {
    /// What the new-pane control's menu offers.
    ///
    /// A window with no project can start no pane — there is no folder to run one in — so it is
    /// offered the console alone rather than anything that would do nothing. Detached panes come
    /// first: reattaching one is picking up work already running, which reads before starting
    /// something new. Then the shells. **A runnable tool is no longer a row here** — it lives
    /// behind the titlebar's own play control and its chevron, which is where the whole of a
    /// project's tools are read at once; see [`Self::run_tool_rows`].
    /// Each separator is a row like any other, and there is none when there is nothing above it to
    /// separate — no detached pane degrades to exactly the menu before detaching existed.
    ///
    /// **A harness is not a row.** Starting one is what the New agent form is for, and it asks the
    /// identity, the model, the level and the mode in the same breath; this menu offers the ways
    /// of opening a terminal that are not an agent.
    ///
    /// `detached_count` is the length of the caller's own detached-pane list — that list lives on
    /// `AppState`, which `state/` holds no reference to, so it is threaded in the same way
    /// `has_project` already is rather than read from a field here.
    pub fn new_pane_rows(&self, has_project: bool, detached_count: usize) -> Vec<NewPaneRow> {
        let mut rows = Vec::new();
        if detached_count > 0 {
            rows.push(NewPaneRow::DetachedHeading);
            rows.extend((0..detached_count).map(NewPaneRow::Detached));
            rows.push(NewPaneRow::Separator);
        }
        if has_project {
            rows.extend((0..self.shells.len()).map(NewPaneRow::Shell));
            if !self.shells.is_empty() {
                rows.push(NewPaneRow::Separator);
            }
        }
        rows.push(NewPaneRow::Console);
        rows
    }

    /// What the titlebar's run menu offers: every applicable tool, by its index in
    /// [`Self::tools`].
    ///
    /// Indices rather than a row enum, because there is exactly one kind of row — a tool — and
    /// the index *is* the row. The applicable filter is the host's own stamp: a macOS-only tool
    /// is not offered on Windows, and the host is what knows which platform it answers for.
    pub fn run_tool_rows(&self) -> Vec<usize> {
        self.tools
            .iter()
            .enumerate()
            .filter(|(_, tool)| tool.applicable)
            .map(|(index, _)| index)
            .collect()
    }

    /// What the titlebar's overflow menu offers.
    ///
    /// `has_project` and `capture_offered` are asked of the caller rather than read from `self`
    /// for the reason `new_pane_rows` takes `has_project`: this is plain data, and neither the
    /// project nor the capture backend is a fact `WorkbenchState` itself can answer.
    pub fn overflow_rows(&self, has_project: bool, capture_offered: bool) -> Vec<OverflowRow> {
        let mut rows = vec![OverflowRow::RemoteConnect];
        if has_project {
            rows.push(OverflowRow::WebExport);
        }
        if has_project && capture_offered {
            rows.push(OverflowRow::CaptureWindow);
        }
        // Always offered, project or not: help is about the application, and a window with no
        // folder open is one of the places a reader most wants it.
        rows.push(OverflowRow::Help);
        // Under Help, because it is the same question asked the other way round: Help opens the
        // page for where you are standing, this one waits for you to point at something.
        rows.push(OverflowRow::PointAtSomething);
        rows.push(OverflowRow::Settings);
        rows
    }

    /// What the titlebar's new-project menu offers — always the same three rows, in the same
    /// order the project picker draws them in at its foot.
    pub fn new_project_rows(&self) -> Vec<NewProjectRow> {
        vec![
            NewProjectRow::AddProject,
            NewProjectRow::CloneProject,
            NewProjectRow::RemoteProject,
        ]
    }

    /// What a harness menu offers: one row per identity signed into a harness, and one per saved
    /// setup — grouped so both are legible, read by every surface that offers a list of them and
    /// by the pick behind it.
    ///
    /// **A bare harness is no longer a row.** Starting one with nothing else answered is what the
    /// New agent form is for, and it asks the identity, the model, the level and the mode in the
    /// same breath; a row that started a harness on whatever the library happened to resolve was
    /// the same launch with every question skipped. So the `Default` group — every
    /// [`HarnessChoice::Harness`] row, its heading and its separator — is gone, and what is left
    /// is the two groups that name something the user set up.
    ///
    /// Unavailable harnesses are still what a row draws disabled over, so a list says a tool is
    /// missing rather than silently omitting it.
    ///
    /// **A harness that cannot converse is omitted entirely**, unavailable ones notwithstanding:
    /// the two absences say different things. "Not installed" is worth drawing disabled, because
    /// installing it is the fix; "has no structured bridge" is not something the reader can act
    /// on, and the harness is not missing — it still runs perfectly well in a pane. Indices stay
    /// indices into `agent_types`, gap and all.
    pub fn harness_choices(
        &self,
        accounts: &[AccountInfo],
        profiles: &[ProfileInfo],
    ) -> Vec<HarnessChoice> {
        let conversable: Vec<usize> = self
            .agent_types
            .iter()
            .enumerate()
            .filter(|(_, harness)| harness.chat)
            .map(|(index, _)| index)
            .collect();

        let pairs: Vec<HarnessChoice> = conversable
            .iter()
            .map(|&index| (index, &self.agent_types[index]))
            .flat_map(|(index, harness)| {
                accounts
                    .iter()
                    .filter(move |account| account.logged_in.contains(&harness.id))
                    .map(move |account| HarnessChoice::Pair {
                        harness: index,
                        account: account.id.clone(),
                    })
            })
            .collect();

        let mut rows: Vec<HarnessChoice> = Vec::new();
        if !pairs.is_empty() {
            rows.push(HarnessChoice::Label("Configured".into()));
            rows.extend(pairs);
        }
        if !profiles.is_empty() {
            if !rows.is_empty() {
                rows.push(HarnessChoice::Separator);
            }
            rows.push(HarnessChoice::Label("Defined".into()));
            rows.extend((0..profiles.len()).map(HarnessChoice::Profile));
        }
        rows
    }

    /// One harness by the library's id, where the host still lists it. `None` for a harness that
    /// has been uninstalled, or that this build has simply not been told about yet.
    pub fn agent_type(&self, id: &str) -> Option<&AgentTypeInfo> {
        self.agent_types.iter().find(|info| info.id == id)
    }

    /// One harness by its **display label**, which is all a `WorkAgent` carries: the host mints the
    /// label from the library and hands it to the work record, so a surface reading a record rather
    /// than a form has nothing else to match on. The inverse of what `ui::settings::harness_label`
    /// does, and here rather than beside it because two surfaces want it.
    pub fn agent_type_by_label(&self, label: &str) -> Option<&AgentTypeInfo> {
        self.agent_types.iter().find(|info| info.label == label)
    }

    /// Whether the explorer and the chat are on screen at all. They are IDE furniture and leave
    /// together — every other panel outlives a rail-mode switch, and the centre panel is what the
    /// mode actually selects between.
    pub fn is_ide(&self) -> bool {
        self.rail_mode.is_ide()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::quota::QuotaSource;

    fn harness(id: &str, available: bool) -> AgentTypeInfo {
        AgentTypeInfo {
            id: id.to_string(),
            label: id.to_string(),
            command: id.to_string(),
            available,
            chat: true,
            acp: false,
            modes: Vec::new(),
            unattended_mode: None,
            keeps_sessions: true,
            quota: QuotaSource::default(),
        }
    }

    fn account(id: &str, logged_in: &[&str]) -> AccountInfo {
        AccountInfo {
            id: id.to_string(),
            logged_in: logged_in.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn profile(id: &str, agent_type: &str) -> ProfileInfo {
        ProfileInfo {
            id: id.to_string(),
            agent_type: agent_type.to_string(),
            account: None,
            model: None,
            mode: None,
            thinking: None,
            max_subagents: None,
            prompt: None,
            mcps: Vec::new(),
            mission_assistant: None,
            project: None,
        }
    }

    fn with(agent_types: Vec<AgentTypeInfo>) -> WorkbenchState {
        WorkbenchState {
            agent_types,
            ..Default::default()
        }
    }

    /// With nothing signed in and nothing saved there is no list at all. A bare harness is not a
    /// row any more — the New agent form is what starts one — so a machine that has configured
    /// nothing has nothing to offer here rather than a group of unanswered launches.
    #[test]
    fn nothing_configured_offers_nothing() {
        let state = with(vec![harness("claude-code", true), harness("codex", true)]);

        assert_eq!(state.harness_choices(&[], &[]), Vec::new());
    }

    /// Signing in is what puts rows on the list: one `Configured` group, its heading a decoration
    /// at a position the pick must skip, and no `Default` group over it.
    #[test]
    fn accounts_are_the_configured_group() {
        let state = with(vec![harness("claude-code", true)]);
        let accounts = [
            account("mdn", &["claude-code"]),
            account("syn", &["claude-code"]),
        ];

        assert_eq!(
            state.harness_choices(&accounts, &[]),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "mdn".to_string()
                },
                HarnessChoice::Pair {
                    harness: 0,
                    account: "syn".to_string()
                },
            ]
        );
    }

    /// An account is only offered for the harnesses it actually has a login for. One account
    /// serving two harnesses is normal, and an account that serves neither offers nothing.
    #[test]
    fn an_account_is_only_offered_where_it_is_signed_in() {
        let state = with(vec![
            harness("claude-code", true),
            harness("codex", true),
            harness("copilot", true),
        ]);
        let accounts = [
            account("both", &["claude-code", "codex"]),
            account("byenv", &[]),
        ];

        assert_eq!(
            state.harness_choices(&accounts, &[]),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "both".to_string()
                },
                HarnessChoice::Pair {
                    harness: 1,
                    account: "both".to_string()
                },
            ]
        );
    }

    /// A saved setup adds a second, "Defined" group — and it appears with no account signed in at
    /// all, since a profile carries its own identity. `Configured` stays absent in that case:
    /// an empty heading is worse than none, which is the rule both groups follow.
    #[test]
    fn profiles_add_a_defined_group_of_their_own() {
        let state = with(vec![harness("codex", true)]);
        let profiles = [profile("reviewer", "codex"), profile("writer", "codex")];

        assert_eq!(
            state.harness_choices(&[], &profiles),
            vec![
                HarnessChoice::Label("Defined".into()),
                HarnessChoice::Profile(0),
                HarnessChoice::Profile(1),
            ]
        );
    }

    /// The whole point of matching by position: once the decorations are counted in, a `Pair`'s
    /// index in the full list still names the same `(harness, account)` the row shows.
    #[test]
    fn a_pairs_index_in_the_full_list_still_resolves_to_it() {
        let state = with(vec![harness("claude-code", true), harness("codex", true)]);
        let accounts = [account("mdn", &["codex"])];

        let rows = state.harness_choices(&accounts, &[]);
        assert_eq!(
            rows[1],
            HarnessChoice::Pair {
                harness: 1,
                account: "mdn".to_string()
            }
        );
    }

    /// A harness with no structured bridge is not offered as a conversation — it would be started
    /// and then never say anything. It is not *missing*, though: it keeps its place in
    /// `agent_types` and its row in the new-pane menu, where a terminal is exactly what it wants.
    #[test]
    fn a_harness_that_cannot_converse_is_not_offered_as_one() {
        let mut grok = harness("grok", true);
        grok.chat = false;
        let state = with(vec![
            harness("claude-code", true),
            grok,
            harness("codex", true),
        ]);
        let accounts = [account("mdn", &["claude-code", "grok", "codex"])];

        assert_eq!(
            state.harness_choices(&accounts, &[]),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "mdn".to_string()
                },
                HarnessChoice::Pair {
                    harness: 2,
                    account: "mdn".to_string()
                },
            ],
            "the indices are still positions in `agent_types`, gap and all"
        );
    }

    /// A logged-in identity for a harness that cannot converse adds no row either: the pair would
    /// name a conversation that cannot happen.
    #[test]
    fn an_account_on_a_non_chat_harness_adds_no_pair() {
        let mut grok = harness("grok", true);
        grok.chat = false;
        let state = with(vec![grok]);
        let accounts = [account("mdn", &["grok"])];

        assert_eq!(state.harness_choices(&accounts, &[]), Vec::new());
    }
}
