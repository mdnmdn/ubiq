//! The internal browser: tabs that are dock editors drawing a raw webview.
//!
//! Three halves in one file. The **tab lifecycle** (open, reveal, close) copies the SQL tab's:
//! a `BrowserTab` in the project's `BrowserState`, a `PanelEdit::Open` + `Reveal`, and a
//! `closed_browser_panel` that runs when the dock drops the panel. The **page's reports** — title,
//! load, navigation, a link wanting a new tab — arrive from the webview's `'static` handlers
//! through `ui::web_view::take_events` and are drained on a timer. The **API** at the bottom is the
//! UI-side surface the MCP handler calls; every tab id in it is the panel key.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui_component::input::{InputEvent, InputState};

use super::*;
use serde_json::{Value, json};
use ubiq_proto::browser::{BrowserCmd, BrowserContentFormat};
use ubiq_proto::ids::BrowserReqId;

use crate::state::browser::{self, BrowserTab, HistoryEntry, Profile, SearchEngine};
use crate::ui::web_view::{self, BrowserEvent};

/// How often the page's reports are drained. A title or a load state arriving a tenth of a second
/// late is invisible; a busy loop for it would not be.
const DRAIN: Duration = Duration::from_millis(100);

/// How long a script's answer is waited for before the command is answered with an error. A
/// webview that has not committed a page holds scripts back and never answers them.
const REPLY_DEADLINE: Duration = Duration::from_secs(10);

/// How long a tab may stay "loading" with no word from the page. A hash navigation or a failed
/// load never fires `Finished`.
const LOADING_MAX: Duration = Duration::from_secs(30);

/// The address field's dropdown: whose it is, and the row the arrow keys are on (`None` until one
/// is pressed, so Enter on typed text is a navigation rather than the first suggestion).
#[derive(Clone, Debug, PartialEq)]
pub struct Dropdown {
    pub key: String,
    pub cursor: Option<usize>,
}

/// One browser tab, as the API reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowserTabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
}

/// What [`AppState::browser_content`] reads out of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentKind {
    /// `document.body.innerText`
    Text,
    /// `document.documentElement.outerHTML`
    Html,
}

impl AppState {
    // ── Lifecycle ───────────────────────────────────────────────────

    /// Open `url` in the browser: in the tab on screen (or the newest one) unless `new_tab`, and
    /// in a fresh one when there is none. An empty `url` is the start page. Returns the tab's id.
    pub fn open_browser(
        &mut self,
        url: &str,
        new_tab: bool,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let project = self.project(cx)?;
        self.open_browser_in(project, url, new_tab, cx)
    }

    /// [`Self::open_browser`] in a named project rather than the one on screen.
    fn open_browser_in(
        &mut self,
        project: ProjectId,
        url: &str,
        new_tab: bool,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let engine = self.browser_engine();
        let target = browser::normalize(url, engine);
        self.ensure_browser_history(project);
        if !new_tab {
            let active = self
                .workbench
                .active_panel
                .as_ref()
                .and_then(|kind| kind.browser_key())
                .map(str::to_string);
            let reuse = {
                let open = self.projects.get(&project)?;
                active
                    .filter(|key| open.browser.tab(key).is_some())
                    .or_else(|| open.browser.tabs.last().map(|tab| tab.key.clone()))
            };
            if let Some(key) = reuse {
                self.browser_navigate(&key, &target, cx);
                self.pending_panels
                    .push(PanelEdit::Reveal(PanelKind::Browser(key.clone())));
                cx.notify();
                return Some(key);
            }
        }
        let tab = BrowserTab::new(
            &target,
            self.browser_profile(project),
            self.browser_persist(project),
        );
        let key = tab.key.clone();
        self.projects.get_mut(&project)?.browser.tabs.push(tab);
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::Browser(key.clone())));
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::Browser(key.clone())));
        self.drain_browser_loop(cx);
        cx.notify();
        Some(key)
    }

    /// Ask the host for a project's history, once per window.
    fn ensure_browser_history(&mut self, project: ProjectId) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if !open.browser.history_requested {
            open.browser.history_requested = true;
            self.bus.send(Message::BrowserHistoryGet {
                project_id: project,
            });
        }
    }

    /// ⌘W over a browser tab: close it if it is the displayed panel.
    pub fn close_active_browser_tab(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(kind @ PanelKind::Browser(_)) = self.workbench.active_panel.clone() else {
            return false;
        };
        self.pending_panels.push(PanelEdit::Close(kind));
        cx.notify();
        true
    }

    /// The dock dropped a browser tab's panel: forget the tab and destroy its webview.
    pub fn closed_browser_panel(&mut self, kind: &PanelKind, cx: &mut Context<Self>) {
        let Some(key) = kind.browser_key() else {
            return;
        };
        for open in self.projects.values_mut() {
            open.browser.tabs.retain(|tab| tab.key != key);
        }
        self.release_browser_tab(key);
        self.panels.remove(kind);
        cx.notify();
    }

    /// Destroy a tab's webview and answer every command still waiting on it.
    pub(super) fn release_browser_tab(&mut self, key: &str) {
        web_view::close(key);
        if self
            .browser_dropdown
            .as_ref()
            .is_some_and(|drop| drop.key == key)
        {
            self.browser_dropdown = None;
        }
        let tokens: Vec<u64> = self
            .browser_pending
            .iter()
            .filter(|(_, pending)| pending.tab == key)
            .map(|(token, _)| *token)
            .collect();
        for token in tokens {
            if let Some(pending) = self.browser_pending.remove(&token) {
                self.browser_reply(
                    pending.project,
                    pending.req_id,
                    Err("the browser tab was closed".to_string()),
                );
            }
        }
    }

    /// Close every browser tab of a project that is leaving this window: the webviews go now and
    /// the panels with the next panel edit.
    pub(super) fn close_project_browsers(&mut self, tabs: &[BrowserTab]) {
        for tab in tabs {
            self.release_browser_tab(&tab.key);
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::Browser(tab.key.clone())));
        }
    }

    /// A project adopted from another window: its tabs reopen here, and get webviews on the next
    /// frame.
    pub(super) fn reopen_project_browsers(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let keys: Vec<String> = self
            .projects
            .get(&project)
            .map(|open| open.browser.tabs.iter().map(|tab| tab.key.clone()).collect())
            .unwrap_or_default();
        if keys.is_empty() {
            return;
        }
        for key in keys {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::Browser(key)));
        }
        self.drain_browser_loop(cx);
    }

    /// Render-time upkeep: give every tab of every project its address field and its webview (the
    /// sweep hides the ones not on screen, and a command may target a background project), and
    /// bring a field up to date with the page it fronts. Idempotent.
    pub(super) fn settle_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut bare: Vec<(ProjectId, String)> = Vec::new();
        let mut unbuilt: Vec<(String, String, Profile, bool)> = Vec::new();
        for (project, open) in &self.projects {
            for tab in &open.browser.tabs {
                if tab.input.is_none() {
                    bare.push((*project, tab.key.clone()));
                }
                if web_view::SUPPORTED && !web_view::is_open(&tab.key) {
                    unbuilt.push((tab.key.clone(), tab.url.clone(), tab.profile, tab.persist));
                }
            }
        }
        if let Some(project) = self.project(cx) {
            self.ensure_browser_history(project);
        }
        for (project, key) in bare {
            self.build_browser_field(project, &key, window, cx);
        }
        for (key, url, profile, persist) in unbuilt {
            if let Err(err) = web_view::open_browser(&key, &url, profile, persist, window, cx) {
                tracing::warn!("browser: no embedded browser for {key}: {err}");
            }
        }
        for open in self.projects.values_mut() {
            for tab in &mut open.browser.tabs {
                let (Some(input), Some(text)) = (tab.input.clone(), tab.pending_field.clone())
                else {
                    continue;
                };
                // Never under the caret: the field is the user's while they are in it.
                if input.read(cx).focus_handle(cx).is_focused(window) {
                    continue;
                }
                tab.pending_field = None;
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
    }

    fn build_browser_field(
        &mut self,
        project: ProjectId,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search, or enter an address\u{2026}")
        });
        let tab_key = key.to_string();
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    // Only a keystroke is a query: the field is also set from the page's address,
                    // and that must not raise the list.
                    if input.read(cx).focus_handle(cx).is_focused(window) {
                        this.open_browser_dropdown(&tab_key, cx);
                    }
                }
                InputEvent::Focus => this.open_browser_dropdown(&tab_key, cx),
                InputEvent::Blur => this.close_browser_dropdown(&tab_key, cx),
                InputEvent::PressEnter { .. } => this.submit_browser_bar(&tab_key, cx),
            },
        );
        if let Some(tab) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.browser.tab_mut(key))
        {
            tab.input = Some(input);
            tab.subscription = Some(subscription);
        }
        cx.notify();
    }

    // ── The page's reports ──────────────────────────────────────────

    /// The drain loop, in `web_panel`'s shape: one per window, broken by the first failed update
    /// or by there being no tab left.
    fn drain_browser_loop(&mut self, cx: &mut Context<Self>) {
        if self.browser_polling {
            return;
        }
        self.browser_polling = true;
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            loop {
                let running = this.update(cx, |this, cx| {
                    if this.projects.values().all(|open| open.browser.tabs.is_empty()) {
                        this.browser_polling = false;
                        return false;
                    }
                    this.drain_browser_events(cx);
                    true
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
                cx.background_executor().timer(DRAIN).await;
            }
        })
        .detach();
    }

    fn drain_browser_events(&mut self, cx: &mut Context<Self>) {
        let expired = self.expire_browser_waits();
        let events = {
            let projects = &self.projects;
            web_view::take_events(|key| projects.values().any(|open| open.browser.tab(key).is_some()))
        };
        if events.is_empty() {
            if expired {
                cx.notify();
            }
            return;
        }
        for (key, event) in events {
            self.apply_browser_event(&key, event, cx);
        }
        cx.notify();
    }

    /// Give up on what outlived its deadline: scripts nobody answered, and loads that never ended.
    fn expire_browser_waits(&mut self) -> bool {
        let now = Instant::now();
        let late: Vec<u64> = self
            .browser_pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(token, _)| *token)
            .collect();
        for token in &late {
            if let Some(pending) = self.browser_pending.remove(token) {
                self.browser_reply(
                    pending.project,
                    pending.req_id,
                    Err("the page did not answer within 10s; it may still be loading".to_string()),
                );
            }
        }
        let mut changed = !late.is_empty();
        for open in self.projects.values_mut() {
            for tab in &mut open.browser.tabs {
                if tab
                    .loading_since
                    .is_some_and(|since| now.duration_since(since) >= LOADING_MAX)
                {
                    tab.loading = false;
                    tab.loading_since = None;
                    changed = true;
                }
            }
        }
        changed
    }

    fn apply_browser_event(&mut self, key: &str, event: BrowserEvent, cx: &mut Context<Self>) {
        match event {
            BrowserEvent::NewTab(url) => {
                // Deny-and-reopen: the page asked for a window, and a tab is what it gets.
                self.open_browser(&url, true, cx);
                return;
            }
            BrowserEvent::Script { token, raw } => {
                self.finish_browser_script(token, raw, cx);
                return;
            }
            BrowserEvent::Screenshot(png) => {
                if !png.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(
                        gpui::ImageFormat::Png,
                        png,
                    )));
                }
                return;
            }
            _ => {}
        }
        let live = web_view::current_url(key, cx);
        let Some((project, open)) = self
            .projects
            .iter_mut()
            .find(|(_, open)| open.browser.tab(key).is_some())
        else {
            return;
        };
        let project = *project;
        let mut visit = None;
        let state = &mut open.browser;
        let Some(tab) = state.tab_mut(key) else {
            return;
        };
        match event {
            BrowserEvent::Title(title) => {
                tab.title = title.clone();
                let url = tab.url.clone();
                state.retitle(&url, &title);
            }
            BrowserEvent::LoadStarted(url) => {
                tab.loading = true;
                tab.loading_since = Some(Instant::now());
                // The old page's title must not label (or be recorded against) the new one.
                tab.title.clear();
                tab.url = url.clone();
                tab.pending_field = Some(url);
            }
            BrowserEvent::LoadFinished(url) => {
                tab.loading = false;
                tab.loading_since = None;
                tab.committed = true;
                // The page's own answer wins over the event's, which a redirect can leave stale.
                let url = live.filter(|live| !live.is_empty()).unwrap_or(url);
                tab.url = url.clone();
                tab.pending_field = Some(url.clone());
                let title = tab.title.clone();
                // Optimistic: the host's broadcast replaces the list with the record.
                state.record(&url, &title);
                visit = Some((url, title));
            }
            // The handler also hears subframes, so only the webview's own `url()` is trusted.
            BrowserEvent::Navigated(_) => {
                if let Some(live) = live.filter(|live| !live.is_empty()) {
                    tab.url = live.clone();
                    tab.pending_field = Some(live);
                }
            }
            _ => {}
        }
        if let Some((url, title)) = visit.filter(|(url, _)| url != "about:blank") {
            self.bus.send(Message::BrowserHistoryAdd {
                project_id: project,
                url,
                title,
            });
        }
    }

    // ── The address field's dropdown ────────────────────────────────

    pub fn open_browser_dropdown(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.browser_dropdown.as_ref().map(|d| d.key.as_str()) != Some(key) {
            self.browser_dropdown = Some(Dropdown {
                key: key.to_string(),
                cursor: None,
            });
        } else if let Some(drop) = &mut self.browser_dropdown {
            drop.cursor = None;
        }
        cx.notify();
    }

    pub fn close_browser_dropdown(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.browser_dropdown.as_ref().map(|d| d.key.as_str()) == Some(key) {
            self.browser_dropdown = None;
            cx.notify();
        }
    }

    /// What the dropdown offers for tab `key` now: its history, filtered by what is typed.
    pub fn browser_suggestions(&self, key: &str, cx: &App) -> Vec<HistoryEntry> {
        let Some(tab) = self.open_project(cx).and_then(|open| open.browser.tab(key)) else {
            return Vec::new();
        };
        let typed = tab
            .input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        // A field that is just showing the page's own address offers everything recent.
        let typed = if typed == tab.url { String::new() } else { typed };
        self.open_project(cx)
            .map(|open| open.browser.suggestions(&typed))
            .unwrap_or_default()
    }

    /// Arrow keys: step the cursor, opening the list if it was shut.
    pub fn move_browser_dropdown(&mut self, key: &str, down: bool, cx: &mut Context<Self>) {
        if self.browser_dropdown.as_ref().map(|d| d.key.as_str()) != Some(key) {
            self.open_browser_dropdown(key, cx);
            return;
        }
        let last = self.browser_suggestions(key, cx).len().saturating_sub(1);
        if let Some(drop) = &mut self.browser_dropdown {
            drop.cursor = match (drop.cursor, down) {
                (None, true) => Some(0),
                (None, false) => Some(last),
                (Some(at), true) => Some((at + 1).min(last)),
                (Some(at), false) => Some(at.saturating_sub(1)),
            };
        }
        cx.notify();
    }

    /// Enter in the address field: the row under the cursor, else what was typed.
    pub fn submit_browser_bar(&mut self, key: &str, cx: &mut Context<Self>) {
        let picked = self
            .browser_dropdown
            .as_ref()
            .filter(|drop| drop.key == key)
            .and_then(|drop| drop.cursor)
            .and_then(|at| self.browser_suggestions(key, cx).get(at).cloned());
        let typed = self
            .open_project(cx)
            .and_then(|open| open.browser.tab(key))
            .and_then(|tab| tab.input.as_ref())
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        self.browser_dropdown = None;
        match picked {
            Some(entry) => self.browser_navigate(key, &entry.url, cx),
            None => self.browser_navigate(key, &typed, cx),
        };
    }

    /// A dropdown row pressed with the mouse.
    pub fn pick_browser_suggestion(&mut self, key: &str, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.browser_suggestions(key, cx).get(index).cloned() else {
            return;
        };
        self.browser_dropdown = None;
        self.browser_navigate(key, &entry.url, cx);
    }

    // ── Settings ────────────────────────────────────────────────────

    /// The engine the address field searches with.
    pub fn browser_engine(&self) -> SearchEngine {
        self.workbench.settings.ui.search_engine
    }

    pub fn set_browser_engine(&mut self, engine: SearchEngine, cx: &mut Context<Self>) {
        self.workbench.settings.ui.search_engine = engine;
        self.remember_settings();
        cx.notify();
    }

    /// Whether `project`'s browser tabs keep a profile of their own.
    pub fn browser_private(&self, project: ProjectId) -> bool {
        self.projects
            .get(&project)
            .is_some_and(|open| open.prefs.browser_private)
    }

    /// Whether the profile a new tab of `project` is built on is kept on disk.
    pub fn browser_persist(&self, project: ProjectId) -> bool {
        match self.browser_private(project) {
            true => self
                .projects
                .get(&project)
                .is_none_or(|open| open.prefs.browser_persist),
            false => self.workbench.settings.ui.browser_persist,
        }
    }

    pub fn set_browser_persist_common(&mut self, persist: bool, cx: &mut Context<Self>) {
        self.workbench.settings.ui.browser_persist = persist;
        self.remember_settings();
        cx.notify();
    }

    pub fn set_browser_persist_private(
        &mut self,
        project: ProjectId,
        persist: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.prefs.browser_persist = persist;
        self.store_prefs(project);
        cx.notify();
    }

    /// The profile a new tab of `project` is built on.
    fn browser_profile(&self, project: ProjectId) -> Profile {
        match self.browser_private(project) {
            true => Profile::Private(project),
            false => Profile::Common,
        }
    }

    /// Switch a project between the common profile and its own. Tabs already open keep the profile
    /// they were built on until they are reopened.
    pub fn set_browser_private(&mut self, project: ProjectId, private: bool, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.prefs.browser_private = private;
        self.store_prefs(project);
        cx.notify();
    }

    /// Clear what the project's address dropdown remembers.
    pub fn clear_browser_history(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        if let Some(open) = self.projects.get_mut(&project) {
            open.browser.clear_history();
        }
        self.bus.send(Message::BrowserHistoryClear {
            project_id: project,
        });
        cx.notify();
    }

    /// Clear a profile's cookies, storage and cache, through a live tab on it when there is one.
    pub fn reset_browser_profile(&mut self, profile: Profile, cx: &mut Context<Self>) {
        let live = self
            .projects
            .values()
            .flat_map(|open| open.browser.tabs.iter())
            .find(|tab| tab.profile == profile && web_view::is_open(&tab.key))
            .map(|tab| tab.key.clone());
        web_view::reset_profile(live.as_deref(), profile, cx);
    }

    // ── The `⋯` menu and zoom ───────────────────────────────────────

    /// Put a tab's `⋯` down where it was clicked.
    pub fn open_browser_more(&mut self, key: &str, at: (f32, f32), cx: &mut Context<Self>) {
        self.browser_more = Some((key.to_string(), at));
        self.open_menu(MenuId::BrowserMore, cx);
    }

    pub fn dismiss_browser_more(&mut self, cx: &mut Context<Self>) {
        self.browser_more = None;
        if self.workbench.open_menu == Some(MenuId::BrowserMore) {
            self.workbench.open_menu = None;
        }
        cx.notify();
    }

    /// Step a tab's page zoom (`Some(true)` in, `Some(false)` out) or put it back to 100% (`None`).
    pub fn browser_zoom(&mut self, id: &str, step: Option<bool>, cx: &mut Context<Self>) -> bool {
        let Some(tab) = self
            .projects
            .values_mut()
            .find_map(|open| open.browser.tab_mut(id))
        else {
            return false;
        };
        tab.zoom = match step {
            Some(up) => browser::zoom_step(tab.zoom, up),
            None => 1.0,
        };
        let zoom = tab.zoom;
        let ok = web_view::zoom(id, zoom, cx).is_ok();
        cx.notify();
        ok
    }

    // ── The API (what the MCP handler calls) ────────────────────────

    /// The current project's browser tabs, in tab order.
    pub fn browser_tabs(&self, cx: &App) -> Vec<BrowserTabInfo> {
        self.open_project(cx)
            .map(|open| {
                open.browser
                    .tabs
                    .iter()
                    .map(|tab| BrowserTabInfo {
                        id: tab.key.clone(),
                        url: tab.url.clone(),
                        title: tab.title.clone(),
                        loading: tab.loading,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Go to `input` — an address, or words that become a search — in tab `id`.
    pub fn browser_navigate(&mut self, id: &str, input: &str, cx: &mut Context<Self>) -> bool {
        let url = browser::normalize(input, self.browser_engine());
        let Some(tab) = self
            .projects
            .values_mut()
            .find_map(|open| open.browser.tab_mut(id))
        else {
            return false;
        };
        tab.url = url.clone();
        tab.pending_field = Some(url.clone());
        // `loading` is the page's to set (`LoadStarted`): a hash navigation never reports an end.
        // No webview yet means the tab has not been drawn: it will be built at `tab.url`.
        if web_view::is_open(id) {
            let _ = web_view::load(id, &url, cx);
        }
        cx.notify();
        true
    }

    pub fn browser_back(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        self.browser_script(id, "history.back()", cx)
    }

    pub fn browser_forward(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        self.browser_script(id, "history.forward()", cx)
    }

    pub fn browser_reload(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        self.browser_script(id, "location.reload()", cx)
    }

    /// Stop loading. The tab stops being "loading" at once rather than when the page says so,
    /// because a stopped load may never say.
    pub fn browser_stop(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let ok = self.browser_script(id, "window.stop()", cx);
        if let Some(tab) = self
            .projects
            .values_mut()
            .find_map(|open| open.browser.tab_mut(id))
        {
            tab.loading = false;
            tab.loading_since = None;
            cx.notify();
        }
        ok
    }

    /// Close tab `id`; the dock's drop then destroys the webview.
    pub fn browser_close(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let known = self
            .projects
            .values()
            .any(|open| open.browser.tab(id).is_some());
        if known {
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::Browser(id.to_string())));
            cx.notify();
        }
        known
    }

    /// Run `js` in tab `id`; `callback` gets the result as a JSON string, on the webview's thread.
    pub fn browser_eval(
        &self,
        id: &str,
        js: &str,
        callback: impl Fn(String) + Send + 'static,
        cx: &App,
    ) -> Result<(), String> {
        web_view::eval_with_callback(id, js, cx, callback)
    }

    /// The page's text or HTML, as a JSON string through `callback`.
    pub fn browser_content(
        &self,
        id: &str,
        kind: ContentKind,
        callback: impl Fn(String) + Send + 'static,
        cx: &App,
    ) -> Result<(), String> {
        let js = match kind {
            ContentKind::Text => "document.body.innerText",
            ContentKind::Html => "document.documentElement.outerHTML",
        };
        self.browser_eval(id, js, callback, cx)
    }

    fn browser_script(&mut self, id: &str, js: &str, cx: &mut Context<Self>) -> bool {
        web_view::eval(id, js, cx).is_ok()
    }
}

// ── MCP: the host's commands ────────────────────────────────────────

/// The longest `content` a `GetContent` reply carries, in characters.
const CONTENT_MAX: usize = 500_000;

static TOKENS: AtomicU64 = AtomicU64::new(1);

/// What a script's answer is turned into once it arrives.
#[derive(Clone, Debug)]
enum Pending {
    Eval,
    Content(BrowserContentFormat),
    Back,
    Forward,
    Info,
}

/// One command waiting on a script: whose it is, and what to make of the answer.
#[derive(Clone, Debug)]
pub(super) struct PendingReply {
    project: ProjectId,
    req_id: BrowserReqId,
    tab: String,
    what: Pending,
    deadline: Instant,
}

/// Asks the page whether it can go back or forward. `navigation` is where WebKit has it; without
/// it only "back" is knowable (`null` forward means unknown, and forward is then tried).
const PROBE_JS: &str = "JSON.stringify({b: window.navigation ? navigation.canGoBack : history.length > 1, \
    f: window.navigation ? navigation.canGoForward : null})";

/// Decode what a callback hands back: a JSON value, which for a script that returned a string is
/// a JSON string literal.
fn unquote(raw: &str) -> String {
    serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string())
}

/// `script`, wrapped so the answer is `{ok, v}` or `{ok: false, e}` — a thrown exception is an
/// error and a value that will not serialise is its `String(..)` form.
fn wrap_eval(script: &str) -> String {
    let literal = serde_json::to_string(script).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "(function(){{try{{var r=(0,eval)({literal});var s;\
         try{{s=JSON.stringify(r);}}catch(e){{s=JSON.stringify(String(r));}}\
         return JSON.stringify({{ok:true,v:s===undefined?null:JSON.parse(s)}});}}\
         catch(e){{return JSON.stringify({{ok:false,e:String(e&&e.message||e)}});}}}})()"
    )
}

impl AppState {
    /// The host's side of the browser family. `None` means the message was ours.
    pub(super) fn receive_browser(
        &mut self,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::BrowserHistory {
                project_id,
                entries,
            } => {
                if let Some(open) = self.projects.get_mut(&project_id) {
                    open.browser.set_history(entries);
                    cx.notify();
                }
                None
            }
            Message::BrowserCommand {
                project_id,
                req_id,
                cmd,
            } => {
                self.run_browser_command(project_id, req_id, cmd, cx);
                None
            }
            other => Some(other),
        }
    }

    fn browser_reply(
        &self,
        project: ProjectId,
        req_id: BrowserReqId,
        result: Result<Value, String>,
    ) {
        self.bus.send(Message::BrowserReply {
            project_id: project,
            req_id,
            result: result.map(|value| value.to_string()),
        });
    }

    /// The tab a command means: the named one, else the one on screen, else the newest.
    fn resolve_browser_tab(&self, project: ProjectId, tab: Option<&str>) -> Result<String, String> {
        let state = &self
            .projects
            .get(&project)
            .ok_or_else(|| "no browser tab".to_string())?
            .browser;
        match tab {
            Some(id) => state
                .tab(id)
                .map(|tab| tab.key.clone())
                .ok_or_else(|| format!("no browser tab '{id}'")),
            None => self
                .workbench
                .active_panel
                .as_ref()
                .and_then(|kind| kind.browser_key())
                .filter(|key| state.tab(key).is_some())
                .map(str::to_string)
                .or_else(|| state.tabs.last().map(|tab| tab.key.clone()))
                .ok_or_else(|| "no browser tab".to_string()),
        }
    }

    /// `(url, title, loading)` of a tab.
    fn browser_facts(&self, project: ProjectId, id: &str) -> (String, String, bool) {
        self.projects
            .get(&project)
            .and_then(|open| open.browser.tab(id))
            .map(|tab| (tab.url.clone(), tab.title.clone(), tab.loading))
            .unwrap_or_default()
    }

    /// Start a script whose answer finishes `what` for this command; an error replies at once.
    fn browser_ask(
        &mut self,
        (project, req_id, tab): (ProjectId, BrowserReqId, String),
        js: &str,
        what: Pending,
        cx: &mut Context<Self>,
    ) {
        let token = TOKENS.fetch_add(1, Ordering::Relaxed);
        match web_view::eval_token(&tab, js, token, cx) {
            Ok(()) => {
                self.browser_pending.insert(
                    token,
                    PendingReply {
                        project,
                        req_id,
                        tab,
                        what,
                        deadline: Instant::now() + REPLY_DEADLINE,
                    },
                );
            }
            Err(err) => self.browser_reply(project, req_id, Err(err)),
        }
    }

    fn run_browser_command(
        &mut self,
        project: ProjectId,
        req: BrowserReqId,
        cmd: BrowserCmd,
        cx: &mut Context<Self>,
    ) {
        tracing::debug!("browser: command {} for {project}", cmd.name());
        // Open is the one command that needs no tab.
        if let BrowserCmd::Open { url, new_tab } = &cmd {
            let before = self
                .projects
                .get(&project)
                .map_or(0, |open| open.browser.tabs.len());
            let result = match self.open_browser_in(project, url, *new_tab, cx) {
                Some(id) => {
                    let (url, ..) = self.browser_facts(project, &id);
                    let created = self
                        .projects
                        .get(&project)
                        .is_some_and(|open| open.browser.tabs.len() > before);
                    Ok(json!({"tab": id, "url": url, "new_tab": created}))
                }
                None => Err("no browser tab".to_string()),
            };
            self.browser_reply(project, req, result);
            return;
        }
        if matches!(cmd, BrowserCmd::ListTabs) {
            let active = self
                .workbench
                .active_panel
                .as_ref()
                .and_then(|kind| kind.browser_key())
                .map(str::to_string);
            let tabs: Vec<Value> = self
                .projects
                .get(&project)
                .map(|open| {
                    open.browser
                        .tabs
                        .iter()
                        .map(|tab| {
                            json!({
                                "tab": tab.key,
                                "url": tab.url,
                                "title": tab.title,
                                "loading": tab.loading,
                                "active": active.as_deref() == Some(tab.key.as_str()),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.browser_reply(project, req, Ok(json!({ "tabs": tabs })));
            return;
        }

        let wanted = match &cmd {
            BrowserCmd::Navigate { tab, .. }
            | BrowserCmd::Back { tab }
            | BrowserCmd::Forward { tab }
            | BrowserCmd::Reload { tab }
            | BrowserCmd::Stop { tab }
            | BrowserCmd::Close { tab }
            | BrowserCmd::GetContent { tab, .. }
            | BrowserCmd::Eval { tab, .. }
            | BrowserCmd::Info { tab } => tab.as_deref(),
            BrowserCmd::ListTabs | BrowserCmd::Open { .. } => None,
        };
        let id = match self.resolve_browser_tab(project, wanted) {
            Ok(id) => id,
            Err(err) => {
                self.browser_reply(project, req, Err(err));
                return;
            }
        };
        let at = (project, req, id.clone());
        match cmd {
            BrowserCmd::Navigate { url, .. } => {
                self.browser_navigate(&id, &url, cx);
                let (url, ..) = self.browser_facts(project, &id);
                self.browser_reply(project, req, Ok(json!({"tab": id, "url": url})));
            }
            BrowserCmd::Reload { .. } | BrowserCmd::Stop { .. } => {
                match matches!(cmd, BrowserCmd::Reload { .. }) {
                    true => self.browser_reload(&id, cx),
                    false => self.browser_stop(&id, cx),
                };
                let (url, ..) = self.browser_facts(project, &id);
                self.browser_reply(project, req, Ok(json!({"tab": id, "url": url})));
            }
            BrowserCmd::Close { .. } => {
                self.browser_close(&id, cx);
                self.browser_reply(project, req, Ok(json!({"tab": id, "closed": true})));
            }
            BrowserCmd::Info { .. } => {
                let committed = self
                    .projects
                    .get(&project)
                    .and_then(|open| open.browser.tab(&id))
                    .is_some_and(|tab| tab.committed);
                match committed {
                    true => self.browser_ask(at, PROBE_JS, Pending::Info, cx),
                    // Nothing has been committed, so the webview may be holding scripts back:
                    // answer from what the UI knows, and say there is nowhere to go.
                    false => {
                        let (url, title, loading) = self.browser_facts(project, &id);
                        self.browser_reply(
                            project,
                            req,
                            Ok(json!({
                                "tab": id, "url": url, "title": title, "loading": loading,
                                "can_go_back": false, "can_go_forward": false,
                            })),
                        );
                    }
                }
            }
            BrowserCmd::Back { .. } | BrowserCmd::Forward { .. } => {
                let back = matches!(cmd, BrowserCmd::Back { .. });
                let committed = self
                    .projects
                    .get(&project)
                    .and_then(|open| open.browser.tab(&id))
                    .is_some_and(|tab| tab.committed);
                match (committed, back) {
                    (true, true) => self.browser_ask(at, PROBE_JS, Pending::Back, cx),
                    (true, false) => self.browser_ask(at, PROBE_JS, Pending::Forward, cx),
                    (false, true) => {
                        self.browser_reply(project, req, Err("no page to go back to".to_string()))
                    }
                    (false, false) => self.browser_reply(
                        project,
                        req,
                        Err("no page to go forward to".to_string()),
                    ),
                }
            }
            BrowserCmd::GetContent { format, .. } => {
                let js = match format {
                    BrowserContentFormat::Text => "document.body ? document.body.innerText : ''",
                    BrowserContentFormat::Html => "document.documentElement.outerHTML",
                };
                self.browser_ask(at, js, Pending::Content(format), cx);
            }
            BrowserCmd::Eval { script, .. } => {
                self.browser_ask(at, &wrap_eval(&script), Pending::Eval, cx)
            }
            BrowserCmd::ListTabs | BrowserCmd::Open { .. } => {}
        }
    }

    /// A script's answer arrived, on the UI thread: finish the command that was waiting on it.
    fn finish_browser_script(&mut self, token: u64, raw: String, cx: &mut Context<Self>) {
        let Some(PendingReply {
            project,
            req_id,
            tab,
            what,
            ..
        }) = self.browser_pending.remove(&token)
        else {
            return;
        };
        let (url, title, loading) = self.browser_facts(project, &tab);
        let result = match what {
            Pending::Eval => match serde_json::from_str::<Value>(&unquote(&raw)) {
                Ok(answer) if answer["ok"] == true => {
                    Ok(json!({"tab": tab, "value": answer["v"].clone()}))
                }
                Ok(answer) => Err(answer["e"]
                    .as_str()
                    .unwrap_or("the script failed")
                    .to_string()),
                Err(_) => Err("the script returned nothing readable".to_string()),
            },
            Pending::Content(format) => {
                let text = unquote(&raw);
                let truncated = text.chars().count() > CONTENT_MAX;
                let content: String = text.chars().take(CONTENT_MAX).collect();
                Ok(json!({
                    "tab": tab, "url": url, "title": title,
                    "format": match format {
                        BrowserContentFormat::Text => "text",
                        BrowserContentFormat::Html => "html",
                    },
                    "content": content, "truncated": truncated,
                }))
            }
            Pending::Back | Pending::Forward | Pending::Info => {
                let probe: Value = serde_json::from_str(&unquote(&raw)).unwrap_or(Value::Null);
                let can_back = probe["b"].as_bool().unwrap_or(false);
                // `null` is "this webview cannot say": forward is attempted rather than refused.
                let can_forward = probe["f"].as_bool();
                match what {
                    Pending::Info => Ok(json!({
                        "tab": tab, "url": url, "title": title, "loading": loading,
                        "can_go_back": can_back, "can_go_forward": can_forward.unwrap_or(false),
                    })),
                    Pending::Back if !can_back => Err("no page to go back to".to_string()),
                    Pending::Forward if can_forward == Some(false) => {
                        Err("no page to go forward to".to_string())
                    }
                    other => {
                        match matches!(other, Pending::Back) {
                            true => self.browser_back(&tab, cx),
                            false => self.browser_forward(&tab, cx),
                        };
                        Ok(json!({"tab": tab, "url": url}))
                    }
                }
            }
        };
        self.browser_reply(project, req_id, result);
    }

    /// Copy a PNG of the page to the clipboard; it arrives as an event once WebKit has drawn it.
    pub fn browser_screenshot(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        web_view::screenshot(id, cx).is_ok()
    }
}
