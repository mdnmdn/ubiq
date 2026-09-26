//! Opening help, finding the page for where the reader is standing, and reading a page's file.
//!
//! ```text
//! ? / F1 / overflow ── EnsureHelp ──▶ host   (once per window; idempotent)
//!                    ◀── HelpReady { root, catalog }
//!                    ◀── HelpUnavailable { reason }
//! ```
//!
//! **Nothing here fails.** Help that was never built is a downgrade: the panel opens on a stub
//! compiled into the binary, the `?` is never disabled, and a page the nav lists with no file
//! behind it says so where the page would have been.
//!
//! This is the one layer that touches the filesystem, and only under the `root` the host named —
//! the same standing `app::web_panel` reads a vendor bundle on. `state::help` holds the cache; a
//! path never reaches `ui/`.

use gpui::{App, Context, Window};

use ubiq_proto::messages::Message;

use crate::state::PanelKind;
use crate::state::help::{HelpState, INDEX_PAGE};
use crate::ui::dock;

use super::AppState;

impl AppState {
    /// Bring the help panel on screen, on the page for wherever the reader is standing.
    ///
    /// The titlebar's `?`, **F1** and the overflow menu's Help row are one gesture with three
    /// triggers, so they are one function. It is never refused and never disabled: with no content
    /// built the panel opens on the stub that says how to build some.
    pub fn reveal_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = self.help_target(cx);
        self.open_help_page(&page, window, cx);
    }

    /// Turn on in-place help: point at anything in the window and read what it is.
    ///
    /// **The other help gesture, not a variant of the one above.** `reveal_help` answers *where am
    /// I* with a page about the screen; this answers *what is that* with a sentence about one
    /// control, and the two want different amounts of the same content library. So it takes its
    /// own key (⇧F1) and its own row, and `F1` and the titlebar's `?` are untouched.
    ///
    /// Idempotent: asking again while it is up keeps the cursor it already has, so a second ⇧F1
    /// does not blank the highlight the user is reading.
    pub fn open_help_target(&mut self, cx: &mut Context<Self>) {
        if self.workbench.help_target.is_none() {
            self.workbench.help_target = Some(crate::state::HelpTargeting::default());
        }
        cx.notify();
    }

    /// Turn it off — Escape, or the overlay's own Done.
    pub fn close_help_target(&mut self, cx: &mut Context<Self>) {
        self.workbench.help_target = None;
        cx.notify();
    }

    /// Where the pointer is, from the overlay's full-window `on_mouse_move`.
    ///
    /// A no-op when the mode is down, because a stray move must not raise it, and a no-op when the
    /// pointer has not actually moved: the overlay is redrawn on every notify, and notifying on a
    /// repeat of the same point is a frame loop that never parks.
    pub fn move_help_target(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let Some(mode) = self.workbench.help_target.as_mut() else {
            return;
        };
        if mode.cursor == Some((x, y)) {
            return;
        }
        mode.cursor = Some((x, y));
        cx.notify();
    }

    /// Bring the panel on screen showing one page, by id. What a `ubiq://<project>/help/<id>` link
    /// arrives at, and what a link inside a page follows.
    pub fn open_help_page(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_help(cx);
        let panel = self.panel(PanelKind::Help, cx);
        dock::reveal(
            &self.dock.clone(),
            &panel,
            PanelKind::Help.home(),
            window,
            cx,
        );
        self.help.visit(id);
        self.load_help_page(id);
        cx.notify();
    }

    /// The same, arrived at through a link rather than a control.
    ///
    /// `navigate` has no `Window`, so the panel is queued for the frame that does — the same way a
    /// link to the log console is. Everything else is [`Self::open_help_page`]'s.
    pub(super) fn reveal_help_page(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ensure_help(cx);
        self.help.visit(id);
        self.load_help_page(id);
        self.pending_panels
            .push(super::PanelEdit::Reveal(PanelKind::Help));
    }

    /// Ask the host for the content, the first time this window wants it. The ask is idempotent on
    /// the host's side; the flag is only so a redraw does not repeat it.
    fn ensure_help(&mut self, _cx: &mut Context<Self>) {
        if self.help.asked {
            return;
        }
        self.help.asked = true;
        tracing::info!("help: asking the host for the documentation bundle");
        self.bus.send(Message::EnsureHelp);
    }

    /// Step the panel's own history. Back and forward are the panel's, not the window's — reading
    /// three pages must not move where the window thinks the reader is.
    pub fn step_help(&mut self, back: bool, cx: &mut Context<Self>) {
        if let Some(id) = self.help.step(back) {
            self.load_help_page(&id);
            cx.notify();
        }
    }

    /// Show or hide the contents tree beside the page.
    pub fn toggle_help_contents(&mut self, cx: &mut Context<Self>) {
        self.help.contents_open = !self.help.contents_open;
        cx.notify();
    }

    /// Flip whether the panel follows the reader. Turning it on does not itself jump the page —
    /// the next mode or window change is what moves it, the same as if the reader had just made
    /// that change with follow already on.
    pub fn toggle_help_follow(&mut self, cx: &mut Context<Self>) {
        self.help.follow = !self.help.follow;
        cx.notify();
    }

    /// While follow is on, keep the panel on the page bound to wherever the reader now stands.
    ///
    /// Silent whenever there is nothing to move to: a context with no bound page leaves the panel
    /// exactly where it was, per [`Self::bound_help_page`] — follow only ever carries the reader
    /// forward onto a page that exists, never blanks the one already open.
    pub fn sync_help_follow(&mut self, cx: &mut Context<Self>) {
        if !self.help.follow {
            return;
        }
        let Some(page) = self.bound_help_page(cx) else {
            return;
        };
        if self.help.page.as_deref() == Some(page.as_str()) {
            return;
        }
        self.help.visit(&page);
        self.load_help_page(&page);
        cx.notify();
    }

    /// Fold or unfold one branch of the contents tree.
    pub fn toggle_help_fold(&mut self, id: &str, cx: &mut Context<Self>) {
        self.help.toggle_fold(id);
        cx.notify();
    }

    /// Read one page's markdown, once, and cache it by id.
    ///
    /// A page the nav lists and no file answers caches [`None`], which is what the panel draws the
    /// "not written yet" body from — the miss costs one look at the filesystem, not one per frame.
    fn load_help_page(&mut self, id: &str) {
        if self.help.sources.contains_key(id) {
            return;
        }
        let Some(root) = self.help.state.root().cloned() else {
            return;
        };
        let Some(page) = self.help.state.catalog().and_then(|c| c.page(id)) else {
            self.help.sources.insert(id.to_string(), None);
            return;
        };
        let source = std::fs::read_to_string(root.join(&page.path)).ok();
        if source.is_none() {
            tracing::warn!(
                "help: the page {id} is listed but {} is not there",
                page.path
            );
        }
        self.help.sources.insert(id.to_string(), source);
    }

    /// The page to open for wherever the reader is standing — the ladder in `wip/help.md` §5,
    /// ending at the landing page so F1 always opens something.
    pub fn help_target(&self, cx: &App) -> String {
        self.bound_help_page(cx)
            .unwrap_or_else(|| INDEX_PAGE.to_string())
    }

    /// The page bound to wherever the reader now stands, if the catalogue binds one — rungs 1 to
    /// 4 of the ladder, with no fallback to the index. [`Self::help_target`]'s landing-page
    /// fallback is right for **?**/F1, which always has to open something; follow mode reads this
    /// instead, because a context that binds nothing is not itself a reason to leave the page the
    /// reader is already on.
    fn bound_help_page(&self, cx: &App) -> Option<String> {
        if let Some(page) = self.claimed_help_page(cx) {
            return Some(page.to_string());
        }
        self.context_key(cx)
            .and_then(|key| self.help.state.catalog()?.context.get(&key).cloned())
    }

    /// The context key for where the reader is standing, most specific first, stopping at the
    /// first one the catalogue answers.
    ///
    /// Rungs 2 to 4 of the ladder: the focused panel, the destination's view, and the rail mode.
    /// Every string is the value's own — [`PanelKind::name`], [`crate::state::nav::View::slug`]
    /// and [`crate::state::RailMode::slug`] — so a renamed variant renames its key rather than
    /// quietly unbinding a page.
    pub fn context_key(&self, cx: &App) -> Option<String> {
        let catalog = self.help.state.catalog()?;
        let claimed = |key: String| catalog.context.contains_key(&key).then_some(key);
        let panel = self
            .workbench
            .active_panel
            .as_ref()
            .and_then(|kind| claimed(format!("panel.{}", kind.name())));
        panel
            .or_else(|| {
                self.current_destination(cx)
                    .and_then(|dest| claimed(format!("view.{}", dest.view.slug())))
            })
            .or_else(|| claimed(format!("rail.{}", self.workbench.rail_mode.slug())))
    }

    /// Rung 1: a screen that named a page for itself, where one of those screens is up.
    ///
    /// **The only place in the interface that writes a help id**, and deliberately short: a modal
    /// or an overlay has no context key of its own, so it claims a page directly. Everything else
    /// binds itself by editing the page's `context:` list, which needs no Rust change at all.
    fn claimed_help_page(&self, _cx: &App) -> Option<&'static str> {
        if self.workbench.settings.open {
            return crate::ui::settings::help_page();
        }
        if self.workbench.feedback.is_some() {
            return crate::ui::feedback::help_page();
        }
        if self.workbench.rail_mode == crate::state::RailMode::SINK {
            return crate::ui::sink::help_page();
        }
        None
    }

    /// Note which panel the dock is displaying, so the context ladder has a rung 2 to read.
    ///
    /// Pushed from the dock — where focus is decided — rather than read back from it, the same way
    /// the focused pane is. Also the "window changed" edge follow mode watches for.
    pub fn note_active_panel(&mut self, kind: PanelKind, cx: &mut Context<Self>) {
        self.workbench.active_panel = Some(kind);
        self.sync_help_follow(cx);
    }

    /// The help family.
    ///
    /// Answers with the message when it belongs to another family, like every other `receive_*`.
    pub(super) fn receive_help(
        &mut self,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::HelpReady { root, catalog } => {
                tracing::info!(
                    "help: {} pages of {} documentation at {root}",
                    catalog.pages.len(),
                    catalog.namespace
                );
                self.help.state = HelpState::Ready {
                    root: root.into(),
                    catalog,
                };
                // Anything read before the catalogue arrived was read against nothing.
                self.help.sources.clear();
                if let Some(id) = self.help.page.clone() {
                    self.load_help_page(&id);
                }
                cx.notify();
                None
            }
            Message::HelpUnavailable { reason } => {
                // A downgrade, not a fault: the panel opens on the built-in stub and says how to
                // build the content. Nothing is disabled and no dialog is raised.
                tracing::warn!("help: no documentation to show: {reason}");
                self.help.state = HelpState::Unavailable { reason };
                cx.notify();
                None
            }
            other => Some(other),
        }
    }
}
