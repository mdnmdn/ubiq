//! `AppState`: everything the window knows, and the root of its element tree.
//!
//! It owns the panes, the focused pane and the layout mode, plus the workbench's own state — which
//! rail mode is active, which panels are open, what the explorer, the editor and the chat are
//! showing. No process handle and no pseudo-terminal reaches this far: a pane is an ID and a title.
//!
//! Every mutator ends in `cx.notify()`. One that forgets is a panel that stops updating.

use gpui::{
    App, Bounds, Context, Entity, IntoElement, Render, ScrollHandle, Subscription, Window,
    WindowBounds, WindowOptions, point, prelude::*, px, size,
};
use gpui_component::input::{EditorState, InputEvent, InputState, TabSize, TextareaState};
use uuid::Uuid;

use crate::state::{
    ChatState, EditorPaneState, ExplorerState, MenuId, RailMode, WorkbenchState, sample,
};
use crate::theme::{self, ThemeId};
use crate::ui;

/// Single agent harness pane state.
#[derive(Clone)]
pub struct PaneState {
    pub id: Uuid,
    pub harness: String,
    pub args: Vec<String>,
    pub rows: u16,
    pub cols: u16,
    pub title: String,
    /// Whether the harness behind the pane is still running. An exited pane keeps its last screen.
    pub running: bool,
}

/// Layout mode for pane arrangement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    /// Single pane fills the dock.
    Single,
    /// Side-by-side vertical split.
    Vsplit,
    /// Top-bottom horizontal split.
    Hsplit,
    /// Grid layout (future).
    Grid,
}

pub struct AppState {
    /// Active panes, in the order the dock's tabs show them.
    panes: Vec<PaneState>,
    /// Currently focused pane.
    focused_pane: Option<Uuid>,
    /// How the dock arranges its panes.
    layout_mode: LayoutMode,

    pub workbench: WorkbenchState,
    pub explorer: ExplorerState,
    pub editor: EditorPaneState,
    pub chat: ChatState,

    /// The component library's own state entities.
    pub editor_state: Entity<EditorState>,
    pub chat_input: Entity<TextareaState>,
    pub file_filter: Entity<InputState>,
    /// The titlebar's command field: shortcuts and search, in the middle of the window.
    pub command_input: Entity<InputState>,
    /// The project menu's own search field.
    pub project_search: Entity<InputState>,
    pub chat_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl AppState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::for_project(0, window, cx)
    }

    /// Build a window pointed at one project. A second window is the same view with a different
    /// project selected — see [`open_project_window`].
    pub fn for_project(project: usize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = sample::editor();
        let source = editor
            .active_file()
            .map(|f| f.source.clone())
            .unwrap_or_default();
        let language = editor
            .active_file()
            .map(|f| ui::editor::highlighter_language(f.language))
            .unwrap_or(gpui_component::highlighter::Language::Plain);

        let editor_state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .folding(true)
                .show_whitespaces(false)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .default_value(source)
        });

        let chat_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Reply, or describe the next change\u{2026}")
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });

        let file_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Go to file\u{2026}"));

        let command_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search files, or run a command\u{2026}")
        });

        let project_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a project\u{2026}"));

        let mut subscriptions = Vec::new();

        subscriptions.push(cx.subscribe_in(
            &chat_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.chat.draft = input.read(cx).value().to_string();
                    cx.notify();
                }
                // The textarea submits on a bare Enter; Shift+Enter still inserts a newline and
                // must not send.
                InputEvent::PressEnter { shift: false, .. } => this.send_chat(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &file_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.explorer.filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &project_search,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.workbench.project_filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));

        let mut this = Self {
            panes: Vec::new(),
            focused_pane: None,
            layout_mode: LayoutMode::Single,
            workbench: sample::workbench(project),
            explorer: sample::explorer(),
            editor,
            chat: sample::chat(),
            editor_state,
            chat_input,
            file_filter,
            command_input,
            project_search,
            chat_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        };

        for title in sample::pane_titles() {
            this.push_pane(title);
        }
        this.focused_pane = this.panes.first().map(|p| p.id);
        this
    }

    // ── Panes ───────────────────────────────────────────────────────

    pub fn panes(&self) -> &[PaneState] {
        &self.panes
    }

    pub fn focused_pane(&self) -> Option<&PaneState> {
        self.focused_pane
            .and_then(|id| self.panes.iter().find(|p| p.id == id))
    }

    pub fn focused_pane_index(&self) -> usize {
        self.focused_pane
            .and_then(|id| self.panes.iter().position(|p| p.id == id))
            .unwrap_or(0)
    }

    pub fn layout_mode(&self) -> LayoutMode {
        self.layout_mode
    }

    fn push_pane(&mut self, harness: &str) -> Uuid {
        let id = Uuid::new_v4();
        self.panes.push(PaneState {
            id,
            harness: harness.to_string(),
            args: Vec::new(),
            rows: 24,
            cols: 80,
            title: harness.to_string(),
            running: true,
        });
        id
    }

    pub fn spawn_pane(
        &mut self,
        harness: String,
        args: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let id = self.push_pane(&harness);
        if let Some(pane) = self.panes.last_mut() {
            pane.args = args;
        }
        self.focused_pane = Some(id);
        cx.notify();
        id
    }

    pub fn close_pane(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        self.panes.retain(|p| p.id != pane_id);
        if self.focused_pane == Some(pane_id) {
            self.focused_pane = self.panes.first().map(|p| p.id);
        }
        cx.notify();
    }

    pub fn resize_pane(&mut self, pane_id: Uuid, cols: u16, rows: u16, cx: &mut Context<Self>) {
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) {
            pane.cols = cols;
            pane.rows = rows;
            cx.notify();
        }
    }

    pub fn focus_pane(&mut self, pane_id: Uuid, cx: &mut Context<Self>) {
        if self.panes.iter().any(|p| p.id == pane_id) {
            self.focused_pane = Some(pane_id);
            cx.notify();
        }
    }

    // ── Workbench chrome ────────────────────────────────────────────

    pub fn set_rail_mode(&mut self, mode: RailMode, cx: &mut Context<Self>) {
        self.workbench.rail_mode = mode;
        self.workbench.open_menu = None;
        cx.notify();
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = self.workbench.theme_id.toggled();
        self.workbench.theme_id = next;
        theme::set_mode(next, cx);
        cx.notify();
    }

    pub fn open_menu(&mut self, menu: MenuId, cx: &mut Context<Self>) {
        self.workbench.open_menu = Some(menu);
        cx.notify();
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.pending_close = None;
        cx.notify();
    }

    // ── Projects ────────────────────────────────────────────────────

    /// Point this window at another project. A project that was only remembered becomes open.
    pub fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.workbench.projects.len() {
            return;
        }
        self.workbench.projects[index].open = true;
        self.workbench.project = index;
        self.close_menu(cx);
    }

    /// Close a project. A project with terminals still running asks first: the menu row turns into
    /// a confirmation rather than taking the click.
    pub fn close_project(&mut self, index: usize, force: bool, cx: &mut Context<Self>) {
        let closed = self.workbench.close_project(index, force);
        if closed && self.workbench.projects.iter().all(|p| !p.open) {
            // Never leave the window pointed at nothing: the last project stays open.
            self.workbench.projects[index].open = true;
            self.workbench.project = index;
        }
        cx.notify();
    }

    pub fn cancel_close(&mut self, cx: &mut Context<Self>) {
        self.workbench.pending_close = None;
        cx.notify();
    }

    // ── Explorer ────────────────────────────────────────────────────

    pub fn toggle_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.explorer.toggle(&path);
        cx.notify();
    }

    pub fn select_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer.selected = Some(path.clone());
        if let Some(index) = self.editor.open.iter().position(|f| f.path == path) {
            self.activate_editor_tab(index, window, cx);
        } else {
            cx.notify();
        }
    }

    // ── Editor ──────────────────────────────────────────────────────

    /// Switch tabs, writing the outgoing buffer back first so an edit survives the move.
    pub fn activate_editor_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.editor.open.len() || index == self.editor.active {
            return;
        }

        let current = self.editor_state.read(cx).value().to_string();
        if let Some(file) = self.editor.active_file_mut() {
            file.dirty = file.dirty || file.source != current;
            file.source = current;
        }

        self.editor.active = index;
        let Some(file) = self.editor.active_file() else {
            return;
        };
        let (source, language) = (file.source.clone(), file.language);
        self.explorer.selected = Some(file.path.clone());

        self.editor_state.update(cx, |state, cx| {
            state.set_highlighter(ui::editor::highlighter_language(language), cx);
            state.set_value(source, window, cx);
        });
        cx.notify();
    }

    pub fn close_editor_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = index == self.editor.active;
        self.editor.close(index);
        if was_active && let Some(file) = self.editor.active_file() {
            let (source, language) = (file.source.clone(), file.language);
            self.editor_state.update(cx, |state, cx| {
                state.set_highlighter(ui::editor::highlighter_language(language), cx);
                state.set_value(source, window, cx);
            });
        }
        cx.notify();
    }

    /// The caret's one-based position, as the status bar reports it.
    pub fn cursor_line_column(&self, cx: &App) -> (u32, u32) {
        let position = self.editor_state.read(cx).cursor_position();
        (position.line + 1, position.character + 1)
    }

    // ── Chat ────────────────────────────────────────────────────────

    pub fn send_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat.draft.trim().is_empty() {
            return;
        }
        self.chat.send();

        let input = self.chat_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.chat_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn select_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.chat.chats.len() {
            self.chat.active = index;
            self.chat_scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.chat.new_chat();
        cx.notify();
    }

    pub fn toggle_tool(&mut self, message: usize, block: usize, cx: &mut Context<Self>) {
        self.chat.toggle_tool(message, block);
        cx.notify();
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::shell::render(self, window, cx)
    }
}

/// Re-exported so `main.rs` can name the palette it boots with.
pub fn boot_theme() -> ThemeId {
    ThemeId::Dark
}

/// Open a window on a project.
///
/// This is the only place a window is created, so `main.rs` and the project menu's "open in a new
/// window" reach the same code. Each window owns its own `AppState`; they share nothing but the
/// palette, which is process-wide.
pub fn open_project_window(project: usize, cx: &mut App) {
    // Step successive windows down and across, so a new one does not land exactly on its parent.
    let offset = (cx.windows().len() as f32) * 28.0;
    let mut bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    bounds.origin += point(px(offset), px(offset));

    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Ubiq - Agent Harness Multiplexer".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| AppState::for_project(project, window, cx));
            cx.new(|cx| gpui_component::Root::new(view, window, cx).bg(crate::theme::app_bg()))
        },
    );

    if let Ok(handle) = opened {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .ok();
    }
}
