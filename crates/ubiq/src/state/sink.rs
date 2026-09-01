//! The kitchen sink: the application's own test bench, and the fixtures it draws.
//!
//! **Nothing here belongs to a project.** Every other screen in the window is about a folder the
//! host opened; this one is about Ubiq itself — the editor with no file behind it, each special
//! viewer against a document written here rather than read from disk, and one page holding every
//! primitive the kit and the theme offer. It is where a control is looked at before a screen is
//! built out of it, and where a palette change is checked against everything at once.
//!
//! So the documents are `&'static str` and the state is a handful of demo fields. A fixture that
//! came from the host would make this a project screen; a fixture that came from a file would make
//! it a file screen. The sink is neither, which is the whole reason it can be opened with no
//! project at all.
//!
//! **Nothing here draws and nothing here holds a buffer.** Which layout each document is in lives
//! here; the buffer it is edited in is the window's, beside the other component-library state on
//! [`crate::app::AppState`], because a fixture's buffer is one of the window's fields and not one
//! of a project's files.

use std::collections::HashMap;

use crate::state::editor::{FileLanguage, ViewLayout, ViewerKind};
use crate::state::file_picker::{
    Commit, PickKind, PickerCount, PickerNode, PickerOwner, PickerRequest, PickerView,
};

/// One page of the sink. The first four are one document each, drawn by the viewer its name
/// implies; the last two are drawn rather than parsed — the style reference, and the file picker
/// raised against a fixture tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SinkSection {
    Editor,
    Markdown,
    Mermaid,
    Excalidraw,
    Style,
    Files,
}

impl SinkSection {
    /// The six, in the order the strip draws them.
    pub fn all() -> &'static [SinkSection] {
        &[
            SinkSection::Editor,
            SinkSection::Markdown,
            SinkSection::Mermaid,
            SinkSection::Excalidraw,
            SinkSection::Style,
            SinkSection::Files,
        ]
    }

    /// What the strip's tab says.
    pub fn label(self) -> &'static str {
        match self {
            SinkSection::Editor => "Editor",
            SinkSection::Markdown => "Markdown",
            SinkSection::Mermaid => "Mermaid",
            SinkSection::Excalidraw => "Excalidraw",
            SinkSection::Style => "Style",
            SinkSection::Files => "Files",
        }
    }

    /// The one line under the page's title: what this page is for testing.
    pub fn note(self) -> &'static str {
        match self {
            SinkSection::Editor => "The plain buffer: highlighting, line numbers, folding.",
            SinkSection::Markdown => "A document with a diagram fence of each kind inside it.",
            SinkSection::Mermaid => "A diagram the renderer drew, cached by its content.",
            SinkSection::Excalidraw => "A scene painted from its own JSON.",
            SinkSection::Style => "Every token, surface, control and field, on one page.",
            SinkSection::Files => "The file picker, raised every way a screen can ask for it.",
        }
    }

    /// The document this page draws, or nothing for the page that draws no document.
    pub fn doc(self) -> Option<&'static SinkDoc> {
        docs().iter().find(|doc| doc.section == self)
    }
}

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
        match self {
            SinkModal::Confirm => "Close this pane?",
            SinkModal::Form => "Name this session",
            SinkModal::Danger => "Forget this project?",
        }
    }

    /// What the body says. One paragraph, because a modal that needs two is a panel.
    pub fn note(self) -> &'static str {
        match self {
            SinkModal::Confirm => {
                "The harness in it is still running. Closing the pane ends it, and its scrollback \
                 goes with it."
            }
            SinkModal::Form => {
                "A session is a named piece of work with a folder. The name is what the rail, the \
                 graph and the board call it."
            }
            SinkModal::Danger => {
                "Ubiq forgets the folder, its arrangement and everything it remembered about it. \
                 Nothing on disk is touched."
            }
        }
    }

    /// What the confirming button says. Never "OK": a button says what it does.
    pub fn confirm(self) -> &'static str {
        match self {
            SinkModal::Confirm => "Close the pane",
            SinkModal::Form => "Create",
            SinkModal::Danger => "Forget it",
        }
    }
}

/// The independent facets the style reference's toggle row draws. Four, because that is what the
/// graph's bucket row has and the row has to be tested at the width it is used at.
pub const FACETS: [&str; 4] = ["running", "waiting", "ended", "error"];

/// The values its choice row picks between.
pub const CHOICES: [&str; 3] = ["source", "preview", "split"];

/// The rows its demo menu offers.
pub const MENU_ITEMS: [&str; 4] = ["Claude Code", "Codex", "Gemini CLI", "opencode"];

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
    /// The file picker page: how the next dialog is asked for, and what the last one answered.
    pub picker: PickerDemo,
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
            picker: PickerDemo::default(),
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
        if doc.viewer().has_preview() {
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

    /// An exited harness leaves its pane. Nothing disappears from under the user.
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
