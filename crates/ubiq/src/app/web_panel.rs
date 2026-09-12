//! Editing an open file through a web panel — the Excalidraw cycle, end to end.
//!
//! ```text
//! Editor selected ── EnsureWebBundle ──▶ host      (once per process; idempotent)
//!              ◀── WebBundleReady { path }
//!   open_session ▸ set_vendor_root(path) ▸ a child webview in the panel
//!              ◀── Ready                             the chrome mounted; the loader lifts
//!   ToWeb::Open { document, palette } ──▶            the buffer's text, the window's palette
//!              ◀── Dirty                             the dot lights at once
//!              ◀── Changed { document }              written into the *existing* buffer
//!              ◀── Save { document }                 written, then saved down the normal path
//! ```
//!
//! **The whole trick is `Changed`.** It is `EditorState::set_value` on the buffer `FileBody::Text`
//! already holds — the same buffer the dirty dot, `⌘S`, save-as and `D37`'s version check are all
//! already attached to. There is no new save path and no new transport message: `Save` is the
//! same `⌘S` the tab has always had, pressed from inside the webview because the webview has the
//! keyboard and GPUI never sees the chord.
//!
//! **The native painter keeps the read path.** A `.excalidraw` file opens on `Preview` and draws
//! instantly through `ui/viewer/scene.rs`, with no browser, no download and no delay. Nothing
//! here runs until the tab is switched to `Editor` — a web panel is opened, never fallen into.
//!
//! **Failure is a downgrade, never an error the user has to act on.** No network on a first run,
//! a hash that did not match, a failed fetch: the `Editor` position says why and `Preview` is
//! untouched. On a platform with no embedded browser the session is a browser window beside this
//! one, exactly as phase 5 shipped it.

use std::time::Duration;

use gpui::{AppContext, Context};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

use super::AppState;
use crate::state::web_panel::{BundleState, WebPanelSession};
use crate::theme;
use crate::ui::web_view;
use crate::web_export::{self, bridge};

/// How often the window drains the frames the browser has posted.
///
/// The bridge's own long-poll is what makes the *other* direction prompt; this side only has a
/// queue to read, and a queue read four times a second is imperceptible next to a debounce that is
/// already 600 ms on the web side. Frame rate would be a busy loop for nothing.
const DRAIN: Duration = Duration::from_millis(250);

impl AppState {
    /// The palette the chrome is told to wear, as Excalidraw's own `theme` takes it.
    fn web_palette() -> String {
        if theme::Theme::current().is_dark() {
            "dark".to_string()
        } else {
            "light".to_string()
        }
    }

    /// Whether the file behind this tab can be edited in a browser at all.
    ///
    /// Being editable is a property of the file — a document of a bundle-backed viewer kind whose
    /// buffer is whole and carries a version — and being *available* is a property of the bundle,
    /// which is a separate question the header asks
    /// [`crate::state::web_panel::BundleState::unavailable`].
    pub fn web_editable(&self, key: &str, cx: &gpui::App) -> bool {
        self.file(key, cx)
            .is_some_and(|file| bridge::web_app(file.viewer).is_some() && file.savable())
    }

    /// Every tab this window is drawing in the `Editor` layout, made to have a live session and
    /// a browser to draw it in.
    ///
    /// Driven from `render` rather than from the toggle's click, for the reason
    /// `attach_arrived_files` is: building a child webview needs a `Window`, and a layout can
    /// also be restored from a saved arrangement, which arrives on a message that has none. One
    /// place covers both, and being idempotent is what makes running it every frame free.
    ///
    /// A session outlives a switch back to `Preview`: the component is expensive to mount and the
    /// sweep has already taken its browser off screen. It ends when the tab does.
    pub(super) fn settle_web_panels(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let wanted: Vec<String> = self
            .editor(cx)
            .map(|editor| {
                editor
                    .open
                    .iter()
                    .filter(|file| {
                        file.layout == crate::state::editor::ViewLayout::Edit && file.savable()
                    })
                    .map(|file| file.key())
                    .collect()
            })
            .unwrap_or_default();

        for key in wanted {
            self.ensure_web_session(&key, cx);
        }
        self.settle_web_views(window, cx);
    }

    /// Make sure the bundle is there, then open a session over this tab.
    ///
    /// Idempotent in both directions — a tab that already has a session is left alone, and asking
    /// for a bundle already in flight joins that fetch rather than starting a second.
    fn ensure_web_session(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.web_panels.sessions.contains_key(key) || !self.web_editable(key, cx) {
            return;
        }
        let Some(app) = self
            .file(key, cx)
            .and_then(|file| bridge::web_app(file.viewer))
        else {
            return;
        };

        match self.web_panels.bundle(app) {
            BundleState::Ready { .. } => self.open_web_session(key, app, cx),
            BundleState::Unavailable { .. } => {}
            BundleState::Fetching { .. } => {
                self.wait_for_bundle(key);
                cx.notify();
            }
            BundleState::Unknown => {
                tracing::info!("web panel: asking the host for the {app} bundle");
                self.wait_for_bundle(key);
                self.web_panels.bundles.insert(
                    app.to_string(),
                    BundleState::Fetching {
                        done: 0,
                        total: 0,
                        file: String::new(),
                    },
                );
                self.bus.send(Message::EnsureWebBundle {
                    app: app.to_string(),
                });
                cx.notify();
            }
        }
    }

    /// Give every live session a browser to draw in. A failure here is the session's own error,
    /// so the panel says it where the drawing would have been.
    fn settle_web_views(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        if !web_view::SUPPORTED {
            return;
        }
        let pending: Vec<(String, String)> = self
            .web_panels
            .sessions
            .values()
            .filter(|session| !web_view::is_open(&session.key))
            .map(|session| (session.key.clone(), session.url.clone()))
            .collect();
        for (key, url) in pending {
            tracing::debug!("web panel: building the embedded browser for {key} at {url}");
            if let Err(err) = web_view::open(&key, &url, window, cx) {
                tracing::warn!("web panel: no embedded browser for {key}: {err}");
                if let Some(session) = self.web_panels.sessions.get_mut(&key) {
                    session.error = Some(err);
                }
                cx.notify();
            }
        }
    }

    fn wait_for_bundle(&mut self, key: &str) {
        if !self.web_panels.awaiting.iter().any(|k| k == key) {
            self.web_panels.awaiting.push(key.to_string());
        }
    }

    /// Mint a token, register the bundle as this session's vendor root, and open the browser.
    fn open_web_session(&mut self, key: &str, app: &'static str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(root) = self.web_panels.bundle(app).path().cloned() else {
            return;
        };
        let session = match web_export::open_session(app) {
            Ok(session) => session,
            Err(err) => {
                self.web_panels
                    .bundles
                    .insert(app.to_string(), BundleState::Unavailable { reason: err });
                cx.notify();
                return;
            }
        };
        web_export::set_vendor_root(&session.token, Some(root));

        // With no embedded browser the container is a window beside this one, exactly as phase 5
        // shipped it. Where there is one, `settle_web_views` builds it in the panel instead.
        if !web_view::SUPPORTED
            && let Err(err) = super::open_url(&session.url)
        {
            web_export::close_session(&session.token);
            self.web_panels.bundles.insert(
                app.to_string(),
                BundleState::Unavailable {
                    reason: format!("Could not open a browser: {err}"),
                },
            );
            cx.notify();
            return;
        }

        tracing::info!("web panel: session for {key} open at {}", session.url);
        self.web_panels.sessions.insert(
            key.to_string(),
            WebPanelSession {
                key: key.to_string(),
                project,
                token: session.token,
                url: session.url,
                ready: false,
                palette: Self::web_palette(),
                error: None,
            },
        );
        self.drain_web_frames_loop(cx);
        cx.notify();
    }

    /// End a session: the browser tab may still be on screen, but nothing it posts is read and
    /// nothing more is sent. Called as the file tab closes, and as the window goes.
    pub(super) fn close_web_session(&mut self, key: &str) {
        if let Some(session) = self.web_panels.sessions.remove(key) {
            web_export::close_session(&session.token);
        }
        web_view::close(key);
        self.web_panels.awaiting.retain(|k| k != key);
        self.web_panels.incoming.remove(key);
        self.web_panels.saving.retain(|k| k != key);
    }

    /// The host's side of the bundle family. `None` means the message was ours.
    pub(super) fn receive_web_assets(
        &mut self,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::WebBundlePending {
                app,
                done,
                total,
                file,
            } => {
                tracing::debug!("web panel: the {app} bundle is at {done}/{total} ({file})");
                self.web_panels
                    .bundles
                    .insert(app, BundleState::Fetching { done, total, file });
                cx.notify();
                None
            }
            Message::WebBundleReady { app, path, .. } => {
                tracing::info!("web panel: the {app} bundle is at {path}");
                self.web_panels
                    .bundles
                    .insert(app.clone(), BundleState::Ready { path: path.into() });
                let mut still_waiting = Vec::new();
                for key in std::mem::take(&mut self.web_panels.awaiting) {
                    match self
                        .file(&key, cx)
                        .and_then(|file| bridge::web_app(file.viewer))
                    {
                        Some(tenant) if tenant == app => self.open_web_session(&key, tenant, cx),
                        _ => still_waiting.push(key),
                    }
                }
                self.web_panels.awaiting = still_waiting;
                cx.notify();
                None
            }
            Message::WebBundleFailed { app, error } => {
                // A downgrade, not an error: editing goes away and says why, and every native
                // read path — Source, Preview, Split, the painter — is exactly as it was.
                tracing::warn!("web panel: the {app} bundle is unavailable: {error}");
                self.web_panels
                    .bundles
                    .insert(app.clone(), BundleState::Unavailable { reason: error });
                let waiting = std::mem::take(&mut self.web_panels.awaiting);
                self.web_panels.awaiting = waiting
                    .into_iter()
                    .filter(|key| {
                        self.file(key, cx)
                            .and_then(|file| bridge::web_app(file.viewer))
                            != Some(app.as_str())
                    })
                    .collect();
                cx.notify();
                None
            }
            other => Some(other),
        }
    }

    /// The drain loop, in the shape `poll_stats` already uses: one per window, guarded by a flag,
    /// broken by the first failed `update` (the window has gone) or by there being no session left.
    fn drain_web_frames_loop(&mut self, cx: &mut Context<Self>) {
        if self.web_panels.polling {
            return;
        }
        self.web_panels.polling = true;
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            loop {
                let running = this.update(cx, |this, cx| {
                    if this.web_panels.sessions.is_empty() {
                        this.web_panels.polling = false;
                        return false;
                    }
                    this.drain_web_frames(cx);
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

    /// One pass: push a palette change out, pull whatever came in, act on it.
    fn drain_web_frames(&mut self, cx: &mut Context<Self>) {
        let palette = Self::web_palette();
        let keys: Vec<String> = self.web_panels.sessions.keys().cloned().collect();
        let mut notify = false;

        for key in keys {
            let Some(session) = self.web_panels.sessions.get(&key) else {
                continue;
            };
            let token = session.token.clone();

            // A theme change is noticed once, and switches the component's theme without touching
            // its scene.
            if session.ready && session.palette != palette {
                web_export::send(
                    &token,
                    bridge::to_value(&bridge::ToWeb::Palette {
                        palette: palette.clone(),
                    }),
                );
                if let Some(session) = self.web_panels.sessions.get_mut(&key) {
                    session.palette = palette.clone();
                }
            }

            for frame in web_export::receive(&token) {
                // An unknown variant decodes to nothing; `decode_from_web` has already logged it.
                let Some(decoded) = bridge::decode_from_web(&frame) else {
                    continue;
                };
                notify |= self.apply_web_frame(&key, decoded, cx);
            }
        }

        if notify {
            cx.notify();
        }
    }

    /// One decoded frame. Answers whether the window needs redrawing.
    fn apply_web_frame(
        &mut self,
        key: &str,
        frame: bridge::FromWeb,
        cx: &mut Context<Self>,
    ) -> bool {
        match frame {
            bridge::FromWeb::Ready => {
                let Some(file) = self.file(key, cx) else {
                    return false;
                };
                let Some(document) = file.buffer().map(|b| b.read(cx).value().to_string()) else {
                    return false;
                };
                let palette = Self::web_palette();
                let Some(session) = self.web_panels.sessions.get_mut(key) else {
                    return false;
                };
                tracing::info!("web panel: the chrome for {key} mounted; the loader lifts");
                session.ready = true;
                session.error = None;
                session.palette = palette.clone();
                let token = session.token.clone();
                web_export::send(
                    &token,
                    bridge::to_value(&bridge::ToWeb::Open { document, palette }),
                );
                true
            }
            bridge::FromWeb::Dirty => {
                // The dot lights now rather than after the debounce. The web side only reports a
                // change it has already found to differ, so a document looked at stays clean.
                let project = self.web_panels.sessions.get(key).map(|s| s.project);
                self.with_web_file(project, key, |file| file.mark_dirty())
            }
            bridge::FromWeb::Changed { document } => {
                // `set_value` needs a `Window` and a bridge frame does not come with one, so the
                // text waits for the frame that has both. See `apply_web_documents`.
                self.web_panels.incoming.insert(key.to_string(), document);
                true
            }
            bridge::FromWeb::Save { document } => {
                // `⌘S` inside the webview never reaches GPUI: the native view has the keyboard.
                // So the chrome says so, and the write goes down the one save path there is —
                // the buffer first, then `save_file`, with the version the host gave the tab.
                self.web_panels.incoming.insert(key.to_string(), document);
                if !self.web_panels.saving.iter().any(|k| k == key) {
                    self.web_panels.saving.push(key.to_string());
                }
                true
            }
            bridge::FromWeb::Preview { svg } => {
                // The document as the buffer holds it now is what the picture is filed under, so
                // a preview that lands after the next edit is simply not found again — the panel
                // exports another one.
                let document = self
                    .file(key, cx)
                    .and_then(|file| file.buffer().map(|b| b.read(cx).value().to_string()));
                match document {
                    Some(document) => self.keep_exported(&document, svg, cx),
                    None => false,
                }
            }
            bridge::FromWeb::Error { message } => {
                tracing::warn!("web panel: the chrome reported a failure: {message}");
                if let Some(session) = self.web_panels.sessions.get_mut(key) {
                    session.error = Some(message);
                }
                true
            }
            bridge::FromWeb::Unknown => false,
        }
    }

    /// Reach the open file behind a session's tab, in its own project rather than whichever one is
    /// on screen.
    fn with_web_file(
        &mut self,
        project: Option<ProjectId>,
        key: &str,
        edit: impl FnOnce(&mut crate::state::OpenFile),
    ) -> bool {
        let Some(project) = project else {
            return false;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return false;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return false;
        };
        edit(file);
        true
    }

    /// Write the documents the web side sent into the buffers they belong to.
    ///
    /// In `render`, for the reason `attach_arrived_files` is: a buffer write needs a `Window` and a
    /// message does not come with one. Nothing else about the file is touched — the baseline, the
    /// version and the save state are the ones the host set, so `⌘S` still writes with the version
    /// it read and `D37` still refuses a save whose file moved underneath it.
    pub(super) fn apply_web_documents(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.web_panels.incoming.is_empty() {
            self.flush_web_saves(window, cx);
            return;
        }
        for (key, document) in std::mem::take(&mut self.web_panels.incoming) {
            let Some(file) = self.file(&key, cx) else {
                // The tab closed while the frame was in flight. Dropping it is right: there is no
                // buffer to write into and nothing was promised.
                continue;
            };
            let Some(buffer) = file.buffer().cloned() else {
                continue;
            };
            if buffer.read(cx).value() == document.as_str() {
                continue;
            }
            buffer.update(cx, |state, cx| {
                state.set_value(&document, window, cx);
            });
        }
        self.flush_web_saves(window, cx);
        cx.notify();
    }

    /// The saves the chrome asked for, run after the text it sent has reached the buffer.
    ///
    /// Nothing new: `save_file` is the tab context menu's own row, so the version check, the
    /// dirty dot and `ProjectFileWritten` all behave exactly as they do for `⌘S`.
    fn flush_web_saves(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        for key in std::mem::take(&mut self.web_panels.saving) {
            self.save_file(&key, window, cx);
        }
    }
}

/// The Preview position of a format the interface has no renderer for.
///
/// **Nothing here renders anything.** A `.drawio` file becomes a picture exactly once: inside the
/// panel, which exports an SVG as it edits. This is the two tiers around that — the window's
/// memory cache, shared with the diagrams because it is the same kind of picture in the same
/// directory, and the disk tier in the project's workarea, which is what makes a preview survive
/// a restart without ever opening the panel again.
impl AppState {
    /// What the panel has exported for a document, asking the disk tier once if the window holds
    /// nothing. `&self` and a clone, for the same reason [`AppState::diagram`] is.
    pub fn exported_preview(&self, source: &str) -> crate::app::DiagramEntry {
        use crate::app::DiagramEntry;

        let palette = self.exported_palette();
        let key =
            crate::state::diagrams::key_for(crate::state::diagrams::EXPORTED, source, palette);

        let mut cache = self.diagrams.borrow_mut();
        if let Some(entry) = cache.get(&key) {
            return entry.clone();
        }
        cache.insert(key, DiagramEntry::Pending);
        self.exported_asks
            .borrow_mut()
            .push((source.to_string(), palette));
        DiagramEntry::Pending
    }

    /// Read the disk tier for every preview the frame turned out to need, off the frame thread.
    /// Beside `drain_diagram_asks` in `render`, and for the same reason.
    pub(super) fn drain_exported_asks(&mut self, cx: &mut Context<Self>) {
        let asks = std::mem::take(&mut *self.exported_asks.borrow_mut());
        if asks.is_empty() {
            return;
        }
        let dir = self
            .project_snapshot(cx)
            .map(|project| crate::state::diagrams::cache_dir(&project.workarea));

        for (source, palette) in asks {
            let dir = dir.clone();
            let reading = cx.background_spawn(async move {
                crate::state::diagrams::resolve_exported(&source, palette, dir)
            });
            cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
                let answer = reading.await;
                let _ = this.update(cx, |this, cx| this.exported_landed(answer, cx));
            })
            .detach();
        }
    }

    /// One export from the panel: filed under the document it is of, and written down.
    fn keep_exported(&mut self, source: &str, svg: String, cx: &mut Context<Self>) -> bool {
        use crate::app::DiagramEntry;

        let palette = self.exported_palette();
        let dir = self
            .project_snapshot(cx)
            .map(|project| crate::state::diagrams::cache_dir(&project.workarea));
        let Some((key, image)) = crate::state::diagrams::keep_exported(source, palette, dir, svg)
        else {
            // Markup with no `viewBox` is half an export; the next one replaces it.
            return false;
        };
        self.diagrams
            .borrow_mut()
            .insert(key, DiagramEntry::Ready(crate::app::diagram_picture(image)));
        true
    }

    fn exported_landed(
        &mut self,
        answer: crate::state::diagrams::DiagramAnswer,
        cx: &mut Context<Self>,
    ) {
        use crate::app::DiagramEntry;

        let entry = match answer.result {
            Ok(image) => DiagramEntry::Ready(crate::app::diagram_picture(image)),
            // Not exported yet, which is what the viewer says. The panel fills it in when opened.
            Err(reason) => DiagramEntry::Failed(reason),
        };
        self.diagrams.borrow_mut().insert(answer.key, entry);
        cx.notify();
    }

    /// Which palette an export is filed under. draw.io bakes its theme into the SVG exactly as
    /// merman does, so the two palettes are two entries.
    fn exported_palette(&self) -> crate::state::diagrams::DiagramPalette {
        match self.workbench.theme_id.mode() {
            crate::theme::Mode::Dark => crate::state::diagrams::DiagramPalette::Dark,
            crate::theme::Mode::Light => crate::state::diagrams::DiagramPalette::Light,
        }
    }
}
