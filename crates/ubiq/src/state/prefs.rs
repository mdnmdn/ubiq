//! What the interface remembers between runs, and the schema it owns.
//!
//! The host stores this as an opaque string it never parses, so **the interface owns the schema
//! and the interface versions it**. A blob that fails to parse, or that carries a schema this
//! build has no arm for, is discarded and the window opens on defaults — the host could not
//! validate it, and most of what is here is furniture a default rebuilds.
//!
//! **A schema step that moves a value the user set is upgraded instead**, in [`decode`]: throwing
//! away an appearance the user chose is not a default the window can rebuild. `4 → 5` is the first
//! such arm — see [`upgrade_four_to_five`].

use serde::{Serialize, de::DeserializeOwned};

use ubiq_proto::work::Status;

use crate::state::RailMode;
use crate::theme::{AccentId, ThemeId};

/// The shape this build writes and understands. Bump it and older blobs are discarded.
///
/// It moved to `2` when the files a project remembers became **tab keys** rather than paths, and
/// the dock's saved arrangement gained one panel per open file. Neither is a field a default could
/// rescue: a path read as a key opens the wrong tab, and an arrangement written before file panels
/// existed has a centre panel where the files belong. `LAYOUT_VERSION` follows this number, so the
/// blob and the arrangement inside it are discarded together rather than one half at a time.
///
/// It moved to `3` when the graph screen became `RailMode::Orchestration` and `Agents` became the
/// column screen. `rail_mode: "Agents"` is still a name this build reads, and it now names a
/// different screen — the one case a default cannot rescue, because nothing is missing: the value
/// changed meaning. An older blob would open the wrong mode with the wrong arrangement under it.
///
/// It moved to `4` when `RailMode::Orchestration` was renamed to `RailMode::TeamsOld` to make room
/// for the new `Teams` mode beside it. Same screen, same arrangement, but the serialised tag
/// changed — `rail_mode: "Orchestration"` names nothing this build reads, so a blob written
/// before this change is discarded rather than opening on defaults with the wrong mode recorded.
///
/// It moved to `5` when sizing became two axes (`D151`): `density`, `chrome_font_size` and
/// `conversation_font_size` are gone, and `ViewPrefs::content_font_size` stopped being the
/// project's. Existing fields changing meaning is exactly the case a default cannot rescue — and
/// it is also the first schema step **upgraded rather than discarded**, because throwing this blob
/// away would silently reset every user's appearance. See [`upgrade_four_to_five`].
pub const SCHEMA: u32 = 5;

/// One rail mode's arrangement of one project's window: which edge regions were on screen, and the
/// dock blob that restores it.
///
/// The blob carries the whole arrangement — the tree, the axes, the sizes, which tab of each group
/// was displayed, and whether each region was open. The region flags are written beside it for a
/// `settle` that has the flags and cannot read the blob; the blob is what a restore uses.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct ModeLayout {
    pub show_left: bool,
    pub show_bottom: bool,
    pub show_right: bool,
    #[serde(default)]
    pub layout: Option<serde_json::Value>,
}

impl ModeLayout {
    /// The arrangement a mode opens on when it has never been arranged.
    ///
    /// Most modes open on the centre alone: no region is furniture, and each comes back the
    /// moment it is asked for. Git is the exception — its refs explorer and its changes panel
    /// *are* the screen, so the left and right regions open with it (`D119`). The knowledge base
    /// is the same claim over one side: its explorer is how a document is reached at all, and its
    /// right is the centre's to use.
    pub fn default_for(mode: RailMode) -> Self {
        let (show_left, show_right) = match mode {
            RailMode::Git => (true, true),
            RailMode::Kb | RailMode::Agents => (true, false),
            RailMode::Tasks => (false, true),
            _ => (false, false),
        };
        Self {
            show_left,
            show_bottom: false,
            show_right,
            layout: None,
        }
    }
}

/// The last thing a chat tab was started on: the harness, the identity it ran as, and the two
/// answers the New agent form cannot recover from anywhere else.
///
/// Interface scope rather than a project's, because which harnesses this machine has and which
/// account is signed into them is a fact about the machine — a second project on the same laptop
/// should open offering what the first one used, not start from nothing again.
///
/// The model and the reasoning level are deliberately **not** here: the host already remembers
/// those per harness and identity, and hands them back with the catalogue. What it does not know
/// about is the permission mode a start asked for and the subagent ceiling it was given, so those
/// two are written here and read back as the form's preselection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct LastStart {
    pub agent_type: String,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    /// What the last start was allowed to do without asking. `None` is "the harness's own", which
    /// is a real answer as well as what a blob written before this field carried.
    #[serde(default)]
    pub mode: Option<String>,
    /// The subagent ceiling the last start asked for. `None` says nothing about it at all.
    #[serde(default)]
    pub max_subagents: Option<u8>,
}

/// A named point on the two size axes, and nothing else.
///
/// **A preset is a name and the two numbers.** Not a palette, not an accent, not a trim: letting
/// it carry those would make the size popover two ideas, and a user who wants an appearance saved
/// whole is asking for a workspace, which is a different feature (`D151`).
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct SizePreset {
    pub name: String,
    pub ui_scale: f32,
    pub text_ratio: f32,
}

/// The presets every build ships, in the order they are offered.
///
/// The three names `Density` was retired into are here — Compact, Regular, Comfortable — so
/// nothing disappeared from the user's vocabulary when the enum went (`D151`), plus the Large the
/// old five-constant factor could never reach. **`Compact` is `0.90`, the value
/// `Density::Compact` migrates to**, so a blob upgraded from schema 4 lands *on* a preset rather
/// than between two of them; every value here is a multiple of the slider's `0.05` step for the
/// same reason.
pub const BUILT_IN_SIZE_PRESETS: &[(&str, f32, f32)] = &[
    ("Compact", 0.90, 1.0),
    ("Regular", 1.0, 1.0),
    ("Comfortable", 1.15, 1.0),
    ("Large", 1.30, 1.0),
];

/// Whether a name is one of the built-ins. A built-in cannot be deleted, and a saved preset that
/// shadows one replaces it in place rather than sitting beside it.
pub fn is_built_in_preset(name: &str) -> bool {
    BUILT_IN_SIZE_PRESETS
        .iter()
        .any(|(built_in, _, _)| built_in.eq_ignore_ascii_case(name))
}

/// Every preset on offer: the built-ins in their own order, then whatever the user saved.
///
/// A saved preset whose name matches a built-in **takes that built-in's place and its position**,
/// rather than appearing twice under one name — the same rule saving follows, and the reason
/// `save_size_preset` replaces by name.
pub fn all_size_presets(saved: &[SizePreset]) -> Vec<SizePreset> {
    let mut all: Vec<SizePreset> = BUILT_IN_SIZE_PRESETS
        .iter()
        .map(|&(name, ui_scale, text_ratio)| {
            saved
                .iter()
                .find(|preset| preset.name.eq_ignore_ascii_case(name))
                .cloned()
                .unwrap_or(SizePreset {
                    name: name.to_string(),
                    ui_scale,
                    text_ratio,
                })
        })
        .collect();
    all.extend(
        saved
            .iter()
            .filter(|preset| !is_built_in_preset(&preset.name))
            .cloned(),
    );
    all
}

/// What belongs to the whole interface rather than to any one project.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct InterfacePrefs {
    pub schema: u32,
    pub theme: ThemeId,
    /// The accent the palette is dressed in. `None` — and a blob written before this field
    /// existed — is the palette's own seed, so no schema bump: `default` like every field added
    /// after the first release.
    #[serde(default)]
    pub accent: Option<AccentId>,
    /// The size axis — [`crate::theme::Metrics`], written out flat.
    ///
    /// `ui_scale` moves every dimension in the window and `text_ratio` moves type within it; the
    /// three trims nudge one family against the others. All five default to `1.0`, which is the
    /// window every constant declares. **All five are interface-scoped**, the content trim
    /// included: appearance is a property of the person, not of the folder they opened (`D151`).
    #[serde(default = "one")]
    pub ui_scale: f32,
    #[serde(default = "one")]
    pub text_ratio: f32,
    #[serde(default = "one")]
    pub content_trim: f32,
    #[serde(default = "one")]
    pub chrome_trim: f32,
    #[serde(default = "one")]
    pub conversation_trim: f32,
    /// The size presets the user saved, in the order they were made. The built-ins are code
    /// ([`BUILT_IN_SIZE_PRESETS`]) rather than seeded rows, so a build that retunes one moves it
    /// for everybody; an entry here whose name matches a built-in replaces it.
    ///
    /// **No schema bump for it** — `#[serde(default)]` like every field added after the first
    /// release, and `rest` carries it back out for a build that does not name it.
    #[serde(default)]
    pub size_presets: Vec<SizePreset>,
    /// The themes the user authored, in the order they were made. Each is a fork of a built-in
    /// palette plus a sparse override map (`D152`), so a theme travels in `preferences.toml` with
    /// everything else the interface remembers and needs no new store, no new message and no host
    /// change.
    ///
    /// **No schema bump for it**, for the same reason `size_presets` needed none:
    /// `#[serde(default)]`, and `rest` carries it back out for a build that does not name it.
    #[serde(default)]
    pub custom_themes: Vec<crate::theme::CustomTheme>,
    /// What the last conversation was started on, so the next empty tab opens on it. `default`
    /// like every field added after the first release — see [`ViewPrefs`].
    #[serde(default)]
    pub last_start: Option<LastStart>,
    /// Every key in the blob this build does not know, kept as it was found and written back out.
    ///
    /// Serde drops what a struct does not name, so without this a blob carrying more than this
    /// build understands — one written by a newer build at the same schema, or by an edition
    /// keeping a namespaced key of its own — loses those keys the first time this build writes
    /// the blob back. The doc above promises the forward direction; this is the other one.
    #[serde(flatten, default)]
    pub rest: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for InterfacePrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            theme: ThemeId::DARK,
            accent: None,
            ui_scale: 1.0,
            text_ratio: 1.0,
            content_trim: 1.0,
            chrome_trim: 1.0,
            conversation_trim: 1.0,
            size_presets: Vec::new(),
            custom_themes: Vec::new(),
            last_start: None,
            rest: Default::default(),
        }
    }
}

/// The default every size axis takes: the window every constant declares.
fn one() -> f32 {
    1.0
}

/// An untitled tab's text, kept by name rather than by path: nothing on disk is what makes it
/// untitled in the first place, so the key `open_files` uses for every other tab names nothing
/// here to reopen.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Scratch {
    pub name: String,
    pub text: String,
}

/// What belongs to one project: how its window was arranged, and what it was looking at.
///
/// Every field added after the first release is `#[serde(default)]`, so a blob written by an
/// older build at the same schema opens with the new fields empty rather than being discarded.
/// That is what keeps the schema still: it moves only when a field a build already writes changes
/// meaning, which a default cannot rescue.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct ViewPrefs {
    pub schema: u32,
    /// The rail mode the window was left in. The mode's own arrangement is one `modes` entry below.
    pub rail_mode: RailMode,
    /// The window's arrangement, remembered **per rail mode**. Each mode keeps its own picture of
    /// which regions were on screen and a dock blob for the whole tree, because the IDE's side
    /// panels are not the sink's firewalls and arriving in one mode must not undo the other.
    ///
    /// A mode with no entry has never been arranged; opening it falls to
    /// [`ModeLayout::default_for`]. Entries are written when the window leaves a mode (the blob is
    /// the whole arrangement, read off the dock) and read back into the window when the mode
    /// returns.
    ///
    /// The blob is stored by the host as an opaque value it never parses, like everything else
    /// here, so the schema stays the interface's own. It carries a version of its own inside, and
    /// one written for another is discarded for the default arrangement rather than half-applied.
    /// Terminal panels are in it and are dropped on load: layout persists, harnesses do not.
    #[serde(default)]
    pub modes: std::collections::HashMap<RailMode, ModeLayout>,
    /// The tabs open in the centre, in tab order, as `state/editor.rs`'s tab keys.
    ///
    /// A key rather than a path, because a file and its diff are two tabs on one path and a path
    /// names both. An unprefixed key *is* the path, which is what the file itself opens under.
    #[serde(default)]
    pub open_files: Vec<String>,
    /// Untitled buffers, kept by name and text rather than as `open_files` keys: an untitled tab
    /// names nothing on disk, so a real project file's unsaved edits still close for good — only
    /// what has no path to reread from is worth carrying past a close.
    #[serde(default)]
    pub scratch: Vec<Scratch>,
    /// The open files protected from close, as the same tab keys `open_files` uses. A tab key is
    /// the one tab identity a restart can act on — a pane dies with its process, so only a file's
    /// pin is worth writing down; see `OpenFile::pinned`.
    #[serde(default)]
    pub pinned_files: Vec<String>,
    /// The chat tabs that were open, in tab order, as the agent each was attached to. A tab
    /// attached to nothing is not written down: there is nothing to bring back, and a fresh tab
    /// is what an empty list already produces.
    ///
    /// **The tab's own id is not what is remembered.** A `ChatId` is a process-local counter and
    /// means nothing in the next run; the attachment is what does, now that a conversation can be
    /// marked persistent and the host keeps its run directory across a restart. A restore mints a
    /// fresh id per tab and asks the host to revive the agent named here.
    ///
    /// Stored as text rather than as an `AgentId`, the rule `bookmarks` follows: an id this build
    /// can no longer read costs one tab on restore rather than the whole blob.
    ///
    /// **No schema bump for it** — it is `#[serde(default)]` like every field added after the
    /// first release, and moving the schema would throw away every user's whole layout for the
    /// sake of a convenience.
    #[serde(default)]
    pub chats: Vec<String>,
    /// Which of `open_files` or `scratch` was in front. A key rather than an index, because a
    /// file that fails to open must not shift what "active" meant.
    #[serde(default)]
    pub active_file: Option<String>,
    /// The folders the explorer had open, so a tree comes back as it was left rather than shut.
    #[serde(default)]
    pub expanded: Vec<String>,
    /// The row the explorer had selected, open or not.
    #[serde(default)]
    pub selected: Option<String>,
    /// The text in the explorer's "Go to file…" field, kept per project so a switch back does not
    /// have to be re-typed. Absent means the field was empty.
    #[serde(default)]
    pub file_filter: String,
    /// **Parsed and ignored.** The base point size the content family used to be drawn at, when
    /// that size was the project's rather than the interface's. It is now
    /// `InterfacePrefs::content_trim` and one setting for all of Ubiq (`D151`); what is left here
    /// is read once, by [`crate::app::AppState::apply_preferences`], to seed that trim from the
    /// most recently opened project — and then written back out untouched, so a downgrade still
    /// finds the zoom it left. It goes at the next schema step.
    ///
    /// Written as `ui_font_size` before the three families were named, which is the alias.
    #[serde(default, alias = "ui_font_size")]
    pub content_font_size: Option<f32>,
    /// Whether every file editor in this project soft-wraps long lines. `None` is the editor's own
    /// default.
    #[serde(default)]
    pub editor_wrap: Option<bool>,
    /// The rail modes this project hides. Empty is every mode on screen; the last enabled mode
    /// cannot be turned off, so the rail is never empty.
    #[serde(default)]
    pub hidden_modes: Vec<RailMode>,
    /// The places written down in this project. Each destination is stored as its `ubiq://` text,
    /// and one that no longer parses is dropped rather than costing the blob — see
    /// [`crate::state::nav::kept_bookmarks`].
    #[serde(default, deserialize_with = "crate::state::nav::kept_bookmarks")]
    pub bookmarks: Vec<crate::state::nav::Bookmark>,
    /// The places most recently arrived at, newest first, as `ubiq://` text. Only destinations
    /// that survive a restart are in it.
    #[serde(default)]
    pub recents: Vec<String>,
    /// The tasks board's columns folded to a strip — see `state::board::BoardState::shut`. `default`
    /// like every field added after the first release, so no schema bump.
    #[serde(default)]
    pub board_shut: Vec<Status>,
    /// Whether the tasks board opens a task as a centred modal instead of the side panel — see
    /// `state::board::BoardState::popup`.
    #[serde(default)]
    pub board_popup: bool,
    /// Whether the Teams canvas stops drawing the delegates that have finished — see
    /// `state::teams::TeamsView::hide_done`.
    ///
    /// Remembered rather than transient: it is a reading preference about a canvas the user comes
    /// back to, and a long-running project is exactly the one where it was turned on. The other
    /// two Teams filters — the session and the buckets — are not written down, because narrowing
    /// to one session is a thing a reader does *now*.
    #[serde(default)]
    pub teams_hide_done: bool,
    /// Every key in the blob this build does not know, kept as it was found and written back out.
    ///
    /// Serde drops what a struct does not name, so without this a blob carrying more than this
    /// build understands — one written by a newer build at the same schema, or by an edition
    /// keeping a namespaced key of its own — loses those keys the first time this build writes
    /// the blob back. The doc above promises the forward direction; this is the other one.
    #[serde(flatten, default)]
    pub rest: std::collections::BTreeMap<String, serde_json::Value>,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            rail_mode: RailMode::Ide,
            modes: std::collections::HashMap::new(),
            open_files: Vec::new(),
            scratch: Vec::new(),
            pinned_files: Vec::new(),
            chats: Vec::new(),
            active_file: None,
            expanded: Vec::new(),
            selected: None,
            file_filter: String::new(),
            content_font_size: None,
            editor_wrap: None,
            hidden_modes: Vec::new(),
            bookmarks: Vec::new(),
            recents: Vec::new(),
            board_shut: Vec::new(),
            board_popup: false,
            teams_hide_done: false,
            rest: Default::default(),
        }
    }
}

/// Read a blob back, or nothing at all.
///
/// The schema is probed before anything else is trusted, so a blob from a newer build is discarded
/// whole rather than half-applied. **An older one is upgraded rather than discarded** where an arm
/// exists: a schema step that moves fields a user has set — which `4 → 5` is — would otherwise
/// reset every user's appearance silently.
pub fn decode<T: DeserializeOwned>(blob: &str) -> Option<T> {
    decode_upgraded(blob).map(|(value, _)| value)
}

/// [`decode`], and whether the blob had to be upgraded on the way in.
///
/// The one caller that needs the second half is the interface scope: an upgraded blob is what says
/// the content size has still to be taken off a project's `view.toml` and folded into the
/// interface's trim.
pub fn decode_upgraded<T: DeserializeOwned>(blob: &str) -> Option<(T, bool)> {
    let mut value: serde_json::Value = serde_json::from_str(blob)
        .inspect_err(|error| tracing::debug!("discarding unreadable view state: {error}"))
        .ok()?;

    let schema = value.get("schema").and_then(serde_json::Value::as_u64);
    let upgraded = match schema {
        Some(schema) if schema as u32 == SCHEMA => false,
        Some(4) => {
            upgrade_four_to_five(&mut value);
            true
        }
        Some(schema) => {
            tracing::debug!("discarding view state written for schema {schema}");
            return None;
        }
        None => {
            tracing::debug!("discarding view state with no schema");
            return None;
        }
    };

    serde_json::from_value(value)
        .inspect_err(|error| tracing::debug!("discarding view state: {error}"))
        .ok()
        .map(|value| (value, upgraded))
}

/// The one migration arm: three appearance fields become the two size axes and a trim.
///
/// It rewrites the JSON before serde sees it, and **removes** what it consumed — a key left behind
/// would be swept into `rest` and written back out forever. A `ViewPrefs` blob at schema 4 carries
/// none of these keys and falls through untouched, which is what makes one arm serve both scopes.
fn upgrade_four_to_five(value: &mut serde_json::Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    map.insert("schema".into(), SCHEMA.into());

    // `Density::Compact | Regular | Comfortable` were 0.9 / 1.0 / 1.15 over five constants; they
    // are the same three numbers over every dimension now.
    if let Some(density) = map.remove("density") {
        let ui_scale = match density.as_str() {
            Some("Compact") => 0.9,
            Some("Comfortable") => 1.15,
            _ => 1.0,
        };
        map.insert("ui_scale".into(), ui_scale.into());
    }

    // The chrome base *was* the text axis for the whole window, so it is the text ratio: what the
    // user chose, over what the chrome family would draw at ratio 1.
    if let Some(size) = map.remove("chrome_font_size").and_then(|v| v.as_f64()) {
        let ratio = size / (crate::theme::TEXT_BASE * crate::theme::Family::Chrome.ratio()) as f64;
        map.insert("text_ratio".into(), ratio.into());
    }

    // The conversation base was its own, so it folds into its own family's trim — over the size
    // that family now draws at, the text ratio above included.
    let text_ratio = map
        .get("text_ratio")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(1.0);
    if let Some(size) = map
        .remove("conversation_font_size")
        .and_then(|v| v.as_f64())
    {
        let base = (crate::theme::TEXT_BASE * crate::theme::Family::Conversation.ratio()) as f64
            * text_ratio;
        map.insert("conversation_trim".into(), (size / base).into());
    }
}

/// Write one out. Infallible in practice; an unserialisable value becomes an empty blob, which
/// decodes to nothing and opens on defaults.
pub fn encode<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema-4 interface blob is **upgraded, not discarded**: the three appearance fields it
    /// carries become the two size axes and a trim, the keys they were written under are gone so
    /// `rest` cannot resurrect them, and everything else the blob said survives.
    #[test]
    fn a_schema_four_interface_blob_is_upgraded() {
        let blob = r#"{
            "schema": 4,
            "theme": "ember-dark",
            "accent": "blue",
            "density": "Comfortable",
            "chrome_font_size": 15.0,
            "conversation_font_size": 15.0,
            "something_newer": 7
        }"#;

        let (prefs, upgraded) =
            decode_upgraded::<InterfacePrefs>(blob).expect("a schema-4 blob upgrades");
        assert!(upgraded, "the caller is told a migration ran");
        assert_eq!(prefs.schema, SCHEMA);
        assert_eq!(prefs.theme, crate::theme::ThemeId("ember-dark"));
        assert_eq!(prefs.ui_scale, 1.15, "Comfortable is 1.15");

        // 15px of chrome, where the chrome family draws at 13 × 0.96.
        let want = 15.0 / (crate::theme::TEXT_BASE * crate::theme::Family::Chrome.ratio());
        assert!((prefs.text_ratio - want).abs() < 1e-5);
        // The conversation was at the same size as the chrome, so its own trim is 1.
        assert!((prefs.conversation_trim - 1.0).abs() < 1e-5);
        assert_eq!(
            prefs.content_trim, 1.0,
            "no content size in an interface blob"
        );

        // The keys the migration consumed do not come back through `rest`.
        assert!(!prefs.rest.contains_key("density"));
        assert!(!prefs.rest.contains_key("chrome_font_size"));
        assert!(
            prefs.rest.contains_key("something_newer"),
            "the rest is kept"
        );
    }

    /// A schema-4 *project* blob carries none of those keys and falls through the same arm
    /// untouched — which is what lets one arm serve both scopes. Its content size survives for
    /// `apply_preferences` to fold into the interface's trim.
    #[test]
    fn a_schema_four_view_blob_keeps_its_content_size() {
        let blob = r#"{"schema": 4, "rail_mode": "Ide", "content_font_size": 16.0}"#;
        let (view, upgraded) = decode_upgraded::<ViewPrefs>(blob).expect("upgrades");
        assert!(upgraded);
        assert_eq!(view.schema, SCHEMA);
        assert_eq!(view.content_font_size, Some(16.0));
    }

    /// A saved preset replaces the built-in it is named after, in that built-in's place, and one
    /// with a name of its own is offered after all four.
    #[test]
    fn a_saved_preset_shadows_the_built_in_it_names() {
        let saved = vec![
            SizePreset {
                name: "Reading".into(),
                ui_scale: 1.2,
                text_ratio: 1.15,
            },
            SizePreset {
                name: "Compact".into(),
                ui_scale: 0.85,
                text_ratio: 0.9,
            },
        ];
        let all = all_size_presets(&saved);
        let names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Compact", "Regular", "Comfortable", "Large", "Reading"]
        );
        assert_eq!(all[0].ui_scale, 0.85, "the saved one wins its own name");
        assert_eq!(all[1].ui_scale, 1.0);
    }

    /// The built-in Compact and the `4 → 5` migration's `Density::Compact` are the same number, so
    /// an upgrading user lands *on* a preset rather than between two stops.
    #[test]
    fn compact_agrees_with_the_migration() {
        let blob = r#"{"schema": 4, "theme": "dark", "density": "Compact"}"#;
        let prefs = decode::<InterfacePrefs>(blob).expect("upgrades");
        let compact = all_size_presets(&[])
            .into_iter()
            .find(|p| p.name == "Compact")
            .expect("built in");
        assert_eq!(prefs.ui_scale, compact.ui_scale);
    }

    /// A blob from a schema this build knows nothing about is still discarded whole.
    #[test]
    fn an_unknown_schema_is_discarded() {
        assert!(decode::<InterfacePrefs>(r#"{"schema": 99}"#).is_none());
        assert!(decode::<InterfacePrefs>("not json").is_none());
        assert!(decode::<InterfacePrefs>("{}").is_none());
    }

    /// A blob this build wrote round-trips with no migration.
    #[test]
    fn a_current_blob_round_trips() {
        let prefs = InterfacePrefs {
            ui_scale: 1.15,
            text_ratio: 0.9,
            content_trim: 1.2,
            ..Default::default()
        };
        let (back, upgraded) =
            decode_upgraded::<InterfacePrefs>(&encode(&prefs)).expect("round-trips");
        assert!(!upgraded);
        assert_eq!(back, prefs);
    }
}
