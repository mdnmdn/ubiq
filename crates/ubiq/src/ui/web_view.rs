//! The embedded browser a web panel draws in.
//!
//! A child webview is not an element: it is a native view the platform stacks **over** the whole
//! GPUI window, and it knows nothing about the dock, the tab strip or a modal. All this module
//! does is give it a rectangle each frame and take it away again the moment nothing asked for
//! one — which is what stops an Excalidraw canvas from floating over a settings page or over the
//! tab the user switched to.
//!
//! The mechanism is a mark and a sweep, both in prepaint:
//!
//! - [`element`] puts a zero-cost `canvas` beside the webview that marks its slot **seen**. It
//!   only runs when the panel that owns it is actually being drawn, which is the one fact GPUI
//!   will not tell us any other way — a dock tab that is not on screen is never rendered at all.
//! - [`sweeper`], the last child of the window's root, prepaints after every panel has and shows
//!   the seen ones, hides the rest, and clears the marks for the next frame.
//!
//! The registry is a thread-local rather than window state because every webview on the thread
//! has to be swept by whichever window is drawing, and because a `wry::WebView` is UI-thread-only
//! anyway. Slots are keyed by window as well as by tab, so one window's frame never hides
//! another's panels.
//!
//! **macOS and Windows only.** `gpui-wry`'s Unix path is unfinished upstream, so everywhere else
//! [`SUPPORTED`] is `false`, nothing here is compiled, and `app/web_panel.rs` keeps the external
//! browser that phase 5 shipped.

use gpui::{AnyElement, App, IntoElement, ParentElement, Styled, Window, canvas, div, px};

/// Whether this build can host a webview inside the window at all. The one thing the rest of the
/// interface asks before offering to edit in place.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod platform {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use gpui::{App, AppContext as _, Entity, Window, WindowId};
    use gpui_wry::WebView;

    /// One open webview: the entity that lays it out, the window it belongs to, whether a panel
    /// asked for it this frame, and whether it is currently on screen.
    pub(super) struct Slot {
        pub(super) view: Entity<WebView>,
        pub(super) window: WindowId,
        pub(super) seen: bool,
        pub(super) shown: bool,
    }

    thread_local! {
        pub(super) static VIEWS: RefCell<HashMap<String, Slot>> =
            RefCell::new(HashMap::new());
    }

    /// Build a child webview over this window and register it under `key`.
    ///
    /// It starts hidden: the sweep is what puts it on screen, and only once a panel has asked for
    /// a rectangle. A webview that flashed at the window's origin before its first layout would
    /// be visible for exactly the frame the loader is meant to cover.
    pub(super) fn open(
        key: &str,
        url: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String> {
        open_with(key, wry::WebViewBuilder::new().with_url(url), window, cx)
    }

    /// [`open`] over a builder the caller already configured — the internal browser's, with its
    /// handlers on.
    pub(super) fn open_with(
        key: &str,
        builder: wry::WebViewBuilder<'_>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String> {
        let id = window.window_handle().window_id();
        // Spelled out rather than called as a method: `Window` carries both gpui's own
        // `window_handle` and `raw-window-handle`'s, and the inherent one wins.
        let raw = raw_window_handle::HasWindowHandle::window_handle(window)
            .map_err(|err| format!("this window has no native handle: {err}"))?;

        #[cfg(debug_assertions)]
        let builder = builder.with_devtools(true);
        let webview = builder
            .build_as_child(&raw)
            .map_err(|err| format!("could not start the embedded browser: {err}"))?;
        #[cfg(target_os = "macos")]
        super::quiet_host_input_context(&webview);

        let view = cx.new(|cx| WebView::new(webview, window, cx));
        view.update(cx, |view, _| view.hide());

        VIEWS.with(|views| {
            views.borrow_mut().insert(
                key.to_string(),
                Slot {
                    view,
                    window: id,
                    seen: false,
                    shown: false,
                },
            )
        });
        Ok(())
    }

    /// Drop a webview. The entity goes with the slot, and `WebView::drop` hides the native view.
    pub(super) fn close(key: &str) {
        VIEWS.with(|views| views.borrow_mut().remove(key));
    }

    pub(super) fn is_open(key: &str) -> bool {
        VIEWS.with(|views| views.borrow().contains_key(key))
    }

    pub(super) fn view(key: &str) -> Option<Entity<WebView>> {
        VIEWS.with(|views| views.borrow().get(key).map(|slot| slot.view.clone()))
    }

    pub(super) fn mark_seen(key: &str) {
        VIEWS.with(|views| {
            if let Some(slot) = views.borrow_mut().get_mut(key) {
                slot.seen = true;
            }
        });
    }

    /// Show what was asked for, hide what was not, and clear the marks — for `window` only.
    ///
    /// `suppressed` is the window saying "nothing may be on top of me this frame": a modal or a
    /// settings page is drawn by GPUI and a child webview would cover it.
    pub(super) fn sweep(window: WindowId, suppressed: bool, cx: &mut App) {
        let mut changed = Vec::new();
        VIEWS.with(|views| {
            for slot in views.borrow_mut().values_mut() {
                if slot.window != window {
                    continue;
                }
                let wanted = slot.seen && !suppressed;
                slot.seen = false;
                if wanted != slot.shown {
                    slot.shown = wanted;
                    changed.push((slot.view.clone(), wanted));
                }
            }
        });
        for (view, wanted) in changed {
            view.update(cx, |view, cx| {
                if wanted {
                    view.show();
                    // A webview that was hidden was never given bounds — `gpui-wry` skips that in
                    // prepaint while it is invisible — so it needs one more frame to be laid out,
                    // and this is what asks for it. Showing is what notifies, never staying
                    // shown, so there is no loop.
                    cx.notify();
                } else {
                    view.hide();
                }
            });
        }
    }
}

/// Open an embedded browser for `key` at `url`, or say why it could not be opened.
///
/// Idempotent: a key that already has one keeps it, because the session behind it is the same.
pub fn open(key: &str, url: &str, window: &mut Window, cx: &mut App) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if platform::is_open(key) {
            return Ok(());
        }
        platform::open(key, url, window, cx)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, url, window, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// What an internal-browser page reports. Pushed from the webview's handlers, which are `'static`
/// and know nothing of the window, and drained on the UI thread by `AppState`.
#[derive(Clone, Debug, PartialEq)]
pub enum BrowserEvent {
    Title(String),
    LoadStarted(String),
    LoadFinished(String),
    Navigated(String),
    /// A link the page wants somewhere else — `target=_blank`, `window.open`, ctrl/⌘-click.
    NewTab(String),
    /// The page was clicked or focused into, so it has the keyboard (see [`claim_keyboard`]).
    PageFocused,
    /// A script run for a pending command finished: the token it was started under, and the
    /// webview's JSON-encoded result.
    Script { token: u64, raw: String },
    /// A snapshot of the visible page, as PNG bytes and their pixel size; empty when it could not
    /// be taken. `token` is the command that asked, `None` for the menu's clipboard copy.
    Screenshot {
        token: Option<u64>,
        png: Vec<u8>,
        size: (u32, u32),
    },
}

static EVENTS: std::sync::Mutex<Vec<(String, BrowserEvent)>> = std::sync::Mutex::new(Vec::new());

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn push(key: &str, event: BrowserEvent) {
    if let Ok(mut events) = EVENTS.lock() {
        events.push((key.to_string(), event));
    }
}

/// What the pages of the tabs `mine` accepts have reported since the last call, oldest first.
///
/// The queue is process-wide but a window drains only its own tabs: another window's events stay
/// queued for it. An event for a key with no live webview at all (a tab closed under a late
/// callback) belongs to no one and is dropped.
pub fn take_events(mine: impl Fn(&str) -> bool) -> Vec<(String, BrowserEvent)> {
    let Ok(mut events) = EVENTS.lock() else {
        return Vec::new();
    };
    let mut taken = Vec::new();
    events.retain(|(key, event)| {
        if mine(key) {
            taken.push((key.clone(), event.clone()));
            false
        } else {
            is_open(key)
        }
    });
    taken
}

/// A newtab request is honoured only for a plain web address, and at most [`NEWTAB_BURST`] times a
/// second per tab: any page script can post one, so neither the scheme nor the rate is the page's.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const NEWTAB_BURST: usize = 3;

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn newtab_allowed(
    url: &str,
    recent: &std::sync::Mutex<std::collections::VecDeque<std::time::Instant>>,
) -> bool {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    let Ok(mut recent) = recent.lock() else {
        return false;
    };
    let now = std::time::Instant::now();
    while recent
        .front()
        .is_some_and(|at| now.duration_since(*at) > std::time::Duration::from_secs(1))
    {
        recent.pop_front();
    }
    if recent.len() >= NEWTAB_BURST {
        return false;
    }
    recent.push_back(now);
    true
}

/// Catches a ctrl/⌘-click (or a middle click) on a link, cancels it, and hands the address to the
/// host through the ipc handler: the page would otherwise open it in a window nobody asked for.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const NEW_TAB_SCRIPT: &str = r#"
(function () {
  function hit(e) {
    if (!e.isTrusted) return;
    var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    e.preventDefault();
    e.stopPropagation();
    window.ipc.postMessage('newtab:' + a.href);
  }
  document.addEventListener('click', function (e) {
    if (e.ctrlKey || e.metaKey) hit(e);
  }, true);
  document.addEventListener('auxclick', function (e) {
    if (e.button === 1) hit(e);
  }, true);
  // The page took the keyboard: the host moves its own focus off the address field.
  function took() { window.ipc.postMessage('focus'); }
  document.addEventListener('mousedown', took, true);
  document.addEventListener('focusin', took, true);
})();
"#;

/// Open an internal-browser webview for `key` at `url`: a raw webview with handlers that report
/// into [`take_events`]. Idempotent, like [`open`].
pub fn open_browser(
    key: &str,
    url: &str,
    profile: crate::state::browser::Profile,
    persist: bool,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if platform::is_open(key) {
            return Ok(());
        }
        let (k_title, k_load, k_nav, k_win, k_ipc) = (
            key.to_string(),
            key.to_string(),
            key.to_string(),
            key.to_string(),
            key.to_string(),
        );
        let recent = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        let recent_win = recent.clone();
        let builder = profiled_builder(profile, persist)
            .with_url(url)
            .with_initialization_script(NEW_TAB_SCRIPT)
            .with_document_title_changed_handler(move |title| {
                push(&k_title, BrowserEvent::Title(title))
            })
            .with_on_page_load_handler(move |event, url| {
                push(
                    &k_load,
                    match event {
                        wry::PageLoadEvent::Started => BrowserEvent::LoadStarted(url),
                        wry::PageLoadEvent::Finished => BrowserEvent::LoadFinished(url),
                    },
                )
            })
            .with_navigation_handler(move |url| {
                push(&k_nav, BrowserEvent::Navigated(url));
                true
            })
            .with_new_window_req_handler(move |url, _| {
                if newtab_allowed(&url, &recent_win) {
                    push(&k_win, BrowserEvent::NewTab(url));
                }
                wry::NewWindowResponse::Deny
            })
            .with_ipc_handler(move |request| {
                if request.body() == "focus" {
                    push(&k_ipc, BrowserEvent::PageFocused);
                } else if let Some(url) = request.body().strip_prefix("newtab:")
                    && newtab_allowed(url, &recent)
                {
                    push(&k_ipc, BrowserEvent::NewTab(url.to_string()));
                }
            });
        platform::open_with(key, builder, window, cx)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, url, profile, persist, window, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// A builder on `profile`'s cookie jar and storage. macOS keys a persistent data store by a
/// 16-byte id (macOS 14 and later; older systems fall back to the shared store); Windows keeps a
/// directory per profile.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn profiled_builder(
    profile: crate::state::browser::Profile,
    persist: bool,
) -> wry::WebViewBuilder<'static> {
    // Ephemeral: a non-persistent store, so nothing reaches disk and there is no profile to name.
    if !persist {
        return wry::WebViewBuilder::new().with_incognito(true);
    }
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin as _;
        wry::WebViewBuilder::new().with_data_store_identifier(profile.data_id())
    }
    #[cfg(target_os = "windows")]
    {
        // Leaked once per profile: the context has to outlive the webview built over it, and a
        // handful of profiles per process is all there ever is.
        thread_local! {
            static CONTEXTS: std::cell::RefCell<
                std::collections::HashMap<crate::state::browser::Profile, *mut wry::WebContext>,
            > = std::cell::RefCell::new(std::collections::HashMap::new());
        }
        let context = CONTEXTS.with(|contexts| {
            *contexts.borrow_mut().entry(profile).or_insert_with(|| {
                Box::into_raw(Box::new(wry::WebContext::new(Some(windows_profile_dir(profile)))))
            })
        });
        // SAFETY: the context is leaked, so it outlives every webview, and it is only ever handed
        // out on this one (UI) thread.
        wry::WebViewBuilder::new_with_web_context(unsafe { &mut *context })
    }
}

#[cfg(target_os = "windows")]
fn windows_profile_dir(profile: crate::state::browser::Profile) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
        .join("Ubiq")
        .join("browser")
        .join(profile.dir_name())
}

/// Page zoom for the webview behind `key`.
pub fn zoom(key: &str, factor: f64, cx: &App) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let view = platform::view(key).ok_or("no browser behind this tab")?;
        view.read(cx).raw().zoom(factor).map_err(|err| err.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, factor, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// Reload the page behind `key` bypassing the cache: `Ok(true)` when it was, `Ok(false)` where the
/// platform has no such reload (the caller reloads plainly), `Err` when there is no webview.
pub fn reload_from_origin(key: &str, cx: &App) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewExtMacOS as _;
        let view = platform::view(key).ok_or("no browser behind this tab")?;
        let webview = view.read(cx).raw().webview();
        // SAFETY: `-[WKWebView reloadFromOrigin:]`, the action form, with a nil sender; main thread.
        let _: () = unsafe {
            objc2::msg_send![&*webview, reloadFromOrigin: std::ptr::null::<objc2::runtime::AnyObject>()]
        };
        Ok(true)
    }
    #[cfg(target_os = "windows")]
    {
        platform::view(key).ok_or("no browser behind this tab")?;
        let _ = cx;
        Ok(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// Clear a profile's cookies, storage and cache. `live` is a webview built on it, when one is up:
/// its store is in use, so it is cleared in place. With none, the store itself is removed.
pub fn reset_profile(live: Option<&str>, profile: crate::state::browser::Profile, cx: &App) {
    if let Some(key) = live {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(view) = platform::view(key) {
            if let Err(err) = view.read(cx).raw().clear_all_browsing_data() {
                tracing::warn!("browser: could not clear the profile: {err}");
            }
        }
        let _ = (key, cx);
        return;
    }
    let _ = cx;
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewExtDarwin as _;
        wry::WebView::remove_data_store(&profile.data_id(), |result| {
            if let Err(err) = result {
                tracing::debug!("browser: no data store to remove: {err}");
            }
        });
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::fs::remove_dir_all(windows_profile_dir(profile));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = profile;
}

/// Run script in the page behind `key`.
pub fn eval(key: &str, js: &str, cx: &App) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let view = platform::view(key).ok_or("no browser behind this tab")?;
        view.read(cx)
            .raw()
            .evaluate_script(js)
            .map_err(|err| err.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, js, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// [`eval`] with the script's result — a JSON-encoded string — handed to `callback`. The callback
/// runs on the webview's own thread and has to be `Send`.
pub fn eval_with_callback(
    key: &str,
    js: &str,
    cx: &App,
    callback: impl Fn(String) + Send + 'static,
) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let view = platform::view(key).ok_or("no browser behind this tab")?;
        view.read(cx)
            .raw()
            .evaluate_script_with_callback(js, callback)
            .map_err(|err| err.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, js, cx, callback);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// [`eval_with_callback`] whose result comes back through [`take_events`] as
/// [`BrowserEvent::Script`] under `token` — the way to answer on the UI thread from a callback that
/// runs on the webview's.
pub fn eval_token(key: &str, js: &str, token: u64, cx: &App) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let owner = key.to_string();
        eval_with_callback(key, js, cx, move |raw| {
            push(&owner, BrowserEvent::Script { token, raw })
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, js, token, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// Whether [`screenshot`] can do anything on this platform.
pub const SCREENSHOT: bool = cfg!(target_os = "macos");

/// Snapshot the visible page; the PNG arrives as [`BrowserEvent::Screenshot`] under `token`,
/// scaled down proportionally so neither side exceeds `max` when one is given. macOS only, through
/// `WKWebView takeSnapshotWithConfiguration:completionHandler:`.
pub fn screenshot(key: &str, max: Option<u32>, token: Option<u64>, cx: &App) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSImage;
        use objc2_foundation::NSError;
        use objc2_web_kit::WKSnapshotConfiguration;
        use wry::WebViewExtMacOS as _;

        let view = platform::view(key).ok_or("no browser behind this tab")?;
        let webview = view.read(cx).raw().webview();
        let owner = key.to_string();
        let handler = block2::RcBlock::new(move |image: *mut NSImage, _: *mut NSError| {
            let (png, size) = unsafe { image.as_ref() }
                .and_then(|image| image.TIFFRepresentation())
                .and_then(|tiff| {
                    let mut img =
                        image::load_from_memory_with_format(&tiff.to_vec(), image::ImageFormat::Tiff)
                            .ok()?;
                    // `resize` keeps the aspect ratio, fitting inside the box it is given.
                    if let Some(max) = max
                        && (img.width() > max || img.height() > max)
                    {
                        img = img.resize(max, max, image::imageops::FilterType::Triangle);
                    }
                    let size = (img.width(), img.height());
                    let mut out = std::io::Cursor::new(Vec::new());
                    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
                    Some((out.into_inner(), size))
                })
                .unwrap_or_default();
            push(&owner, BrowserEvent::Screenshot { token, png, size });
        });
        let mtm = objc2::MainThreadMarker::new().ok_or("not on the main thread")?;
        let config = unsafe { WKSnapshotConfiguration::new(mtm) };
        unsafe {
            webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &handler)
        };
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (key, max, token, cx);
        Err("screenshots are not available on this platform".to_string())
    }
}

/// How wide the page behind `key` was last laid out, in logical pixels; `None` before its first
/// frame on screen.
pub fn laid_out_width(key: &str, cx: &App) -> Option<f32> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let width = f32::from(platform::view(key)?.read(cx).bounds().size.width);
        (width > 0.0).then_some(width)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, cx);
        None
    }
}

/// Navigate the page behind `key`.
pub fn load(key: &str, url: &str, cx: &App) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let view = platform::view(key).ok_or("no browser behind this tab")?;
        view.read(cx)
            .raw()
            .load_url(url)
            .map_err(|err| err.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, url, cx);
        Err("an embedded browser is not available on this platform".to_string())
    }
}

/// Where the page behind `key` is now. `None` before any page has committed.
pub fn current_url(key: &str, cx: &App) -> Option<String> {
    // Not `wry`'s `url()` on macOS: it unwraps `WKWebView.URL`, which is nil until a page commits —
    // a tab opened straight at an address that refuses the connection never gets one, and the
    // panic on the UI thread took the whole window with it.
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewExtMacOS as _;
        let webview = platform::view(key)?.read(cx).raw().webview();
        let url = unsafe { webview.URL() }?;
        url.absoluteString().map(|url| url.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        platform::view(key)?.read(cx).raw().url().ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (key, cx);
        None
    }
}

/// Close the one behind `key`. Silent when there is none — a tab may close before it ever had one.
pub fn close(key: &str) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    platform::close(key);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = key;
}

// ── The keyboard, when a page holds it (macOS) ──────────────────────────
//
// A webview is a subview of gpui's own view, and gpui's view answers every key *equivalent*
// itself (`performKeyEquivalent:`) without passing it down. AppKit offers two kinds of key that
// way before the first responder ever sees them: anything with ⌘, and the function keys — the
// arrows, Page Up/Down, Home, End, forward delete, F1–F19. So with the page clicked into but gpui's
// focus still on the address field, ⌘C/⌘V went to the field, and an arrow went through gpui's
// text-input path into the field as its private-use character (U+F700…).
//
// Two halves. [`claim_keyboard`] moves gpui's focus off any text field and onto the webview's own
// focus handle, which has no text input: gpui then has nothing to do with an arrow, reports it
// unhandled, and AppKit delivers it to the page as an ordinary key — scrolling, caret moves and
// their modifiers all native. The window's own shortcuts (⌘W, ⌘K, F1) still match, because the
// handle sits inside the window's key contexts. [`forward_edit_key`] hands the edit shortcuts to
// the page outright, because the workbench binds ⌘V and ⌘Z itself.
//
// That alone still delivered every non-printing key twice. gpui's `performKeyEquivalent:` hands an
// arrow, Home/End, Page Up/Down, Esc, ⌫ or F-key to its view's text input context
// (`[[self inputContext] handleEvent:]`) *before* its keymap, whether or not gpui's view is the
// first responder; with the page holding the keyboard that context is not the active one, and the
// text system turns the key into its legacy character code — U+001C–U+001F for the arrows — and
// inserts it, into gpui's text field or into the page. Then gpui reports the key unhandled and
// AppKit gives the page its real `keyDown:`, so the caret moved *and* a glyph was typed. Every
// webview, browser tab or web panel alike, sits over the same gpui view, so the fix is that view's:
// [`quiet_host_input_context`] makes it answer `inputContext` with nil while a native subview of
// it is the first responder. gpui's `handleEvent:` becomes a message to nil (NO), its keymap still
// runs (F1, ⌘W), and the key reaches the page once, through the page's own input context.
//
// Windows needs none of this: WebView2 is a window of its own that takes the keyboard natively, so
// gpui never hears a key the page has, and a click back in gpui's area gives gpui's window the
// keyboard back (`gpui-wry`'s `focus_parent`).

/// The `inputContext` gpui's view class inherited, for [`host_input_context`] to fall back on.
#[cfg(target_os = "macos")]
static HOST_INPUT_CONTEXT: std::sync::OnceLock<objc2::runtime::Imp> = std::sync::OnceLock::new();

/// gpui's view's `inputContext` while a webview is on it: nil when a native subview (a webview) is
/// the window's first responder, the inherited `NSView` answer otherwise.
#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn host_input_context(
    this: *mut objc2::runtime::AnyObject,
    cmd: objc2::runtime::Sel,
) -> *mut objc2::runtime::AnyObject {
    use objc2_app_kit::NSView;

    // SAFETY: installed only on gpui's view class, an `NSView` subclass, and AppKit calls it on
    // the main thread.
    let view = unsafe { &*this.cast::<NSView>() };
    let page_has_keys = view
        .window()
        .and_then(|window| window.firstResponder())
        .and_then(|responder| {
            responder
                .downcast_ref::<NSView>()
                .map(|focused| !std::ptr::eq(focused, view) && focused.isDescendantOf(view))
        })
        .unwrap_or(false);
    if page_has_keys {
        return std::ptr::null_mut();
    }
    let Some(&inherited) = HOST_INPUT_CONTEXT.get() else {
        return std::ptr::null_mut();
    };
    // SAFETY: `inherited` is `-[NSView inputContext]` (or whatever gpui's class resolved it to),
    // whose signature is exactly this one.
    let inherited: unsafe extern "C-unwind" fn(
        *mut objc2::runtime::AnyObject,
        objc2::runtime::Sel,
    ) -> *mut objc2::runtime::AnyObject = unsafe { std::mem::transmute(inherited) };
    unsafe { inherited(this, cmd) }
}

/// Install [`host_input_context`] on the class of the gpui view `webview` was built on. Once per
/// process — every gpui window's view is the same class — and only on gpui's own class, never on
/// `NSView` itself.
#[cfg(target_os = "macos")]
fn quiet_host_input_context(webview: &wry::WebView) {
    use objc2::runtime::AnyObject;
    use wry::WebViewExtMacOS as _;

    static INSTALL: std::sync::Once = std::sync::Once::new();
    // SAFETY: a plain accessor, read on the main thread the webview was just built on.
    let Some(host) = (unsafe { webview.webview().superview() }) else {
        return;
    };
    let host: &AnyObject = host.as_ref();
    let class = host.class();
    if class.name() != c"GPUIView" {
        tracing::warn!(target: "ubiq::browser", class = ?class.name(), "webview host is not gpui's view; arrows may reach the page twice");
        return;
    }
    INSTALL.call_once(|| {
        let sel = objc2::sel!(inputContext);
        let Some(method) = class.instance_method(sel) else {
            return;
        };
        let _ = HOST_INPUT_CONTEXT.set(method.implementation());
        let imp: unsafe extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel) -> *mut AnyObject =
            host_input_context;
        // SAFETY: gpui's class does not define `inputContext` itself (it inherits `NSView`'s), so
        // this adds an override with the inherited one's `@@:` signature; the fallback above calls
        // the inherited implementation.
        let added = unsafe {
            objc2::ffi::class_addMethod(
                class as *const _ as *mut _,
                sel,
                std::mem::transmute::<_, objc2::runtime::Imp>(imp),
                c"@@:".as_ptr(),
            )
        };
        tracing::debug!(target: "ubiq::browser", added = added.as_bool(), "gpui view's inputContext yields to a focused webview");
    });
}

/// The webview of `window` the macOS keyboard is in, and the native responder that has it.
#[cfg(target_os = "macos")]
fn keyboard_page(
    window: &Window,
    cx: &App,
) -> Option<(
    gpui::Entity<gpui_wry::WebView>,
    objc2::rc::Retained<objc2_app_kit::NSResponder>,
)> {
    use objc2_app_kit::NSView;
    use wry::WebViewExtMacOS as _;

    let id = window.window_handle().window_id();
    let views: Vec<_> = platform::VIEWS.with(|views| {
        views
            .borrow()
            .values()
            .filter(|slot| slot.window == id && slot.shown)
            .map(|slot| slot.view.clone())
            .collect()
    });
    views.into_iter().find_map(|view| {
        let webview = view.read(cx).raw().webview();
        let responder = webview.window()?.firstResponder()?;
        let inside = responder
            .downcast_ref::<NSView>()
            .is_some_and(|focused| focused.isDescendantOf(&webview));
        inside.then_some((view, responder))
    })
}

/// Put gpui's focus on the webview the keyboard is in, when the page has it and gpui's focus is
/// elsewhere. Idempotent; a no-op everywhere but macOS. Called on a key (late for that key, in time
/// for the next) and when a browser page reports a click ([`BrowserEvent::PageFocused`]) — before
/// the first key. Keyed on the native first responder, never on the page's word: a page that
/// focuses an input of its own while the user types an address must not take the field away.
pub fn claim_keyboard(window: &mut Window, cx: &mut App) {
    #[cfg(target_os = "macos")]
    if let Some((view, _)) = keyboard_page(window, cx) {
        let handle = gpui::Focusable::focus_handle(view.read(cx), cx);
        if !handle.is_focused(window) {
            window.focus(&handle, cx);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (window, cx);
}

/// Hand an edit shortcut to the page when a webview of `window` holds the keyboard: the matching
/// selector (`copy:`, `paste:` …) is sent down the responder chain from it, the way an Edit menu
/// would, and `true` says gpui must not dispatch the key. Anywhere else this is `false` and gpui
/// keeps the key. macOS only (see above).
pub fn forward_edit_key(keystroke: &gpui::Keystroke, window: &Window, cx: &App) -> bool {
    #[cfg(target_os = "macos")]
    {
        let modifiers = &keystroke.modifiers;
        if !modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
            return false;
        }
        let action = match (keystroke.key.as_str(), modifiers.shift) {
            ("c", false) => objc2::sel!(copy:),
            ("x", false) => objc2::sel!(cut:),
            ("v", false) => objc2::sel!(paste:),
            ("a", false) => objc2::sel!(selectAll:),
            ("z", false) => objc2::sel!(undo:),
            ("z", true) => objc2::sel!(redo:),
            _ => return false,
        };
        let Some((_, responder)) = keyboard_page(window, cx) else {
            return false;
        };
        // Whether or not the page had anything to do with it (an undo with nothing to undo), the
        // key was the page's: it must not fall through to the address field.
        // SAFETY: a standard edit selector, sent with no argument the way a menu sends it.
        let _ = unsafe { responder.tryToPerform_with(action, None) };
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (keystroke, window, cx);
        false
    }
}

/// Whether `key` has a live webview behind it.
pub fn is_open(key: &str) -> bool {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return platform::is_open(key);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = key;
        false
    }
}

/// The element that gives `key`'s webview its rectangle, and marks it wanted this frame.
///
/// `None` when there is no webview behind the key — the caller draws its loader or its notice
/// instead, and the sweep takes the native view off screen.
pub fn element(key: &str) -> Option<AnyElement> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let view = platform::view(key)?;
        let key = key.to_string();
        Some(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .relative()
                // Marked in prepaint rather than here, so the mark means "this frame reached the
                // screen" rather than "an element was built for it".
                .child(
                    canvas(move |_, _, _| mark(&key), |_, _, _, _| {})
                        .absolute()
                        .size_full(),
                )
                .child(view)
                .into_any_element(),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = key;
        None
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn mark(key: &str) {
    platform::mark_seen(key);
}

/// The window root's last child: it shows every webview a panel asked for this frame and hides
/// the rest. `suppressed` hides all of them regardless — a modal or a full-window page is drawn
/// by GPUI, and a native child view would sit on top of it.
pub fn sweeper(suppressed: bool) -> AnyElement {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        canvas(
            move |_, window: &mut Window, cx: &mut App| {
                platform::sweep(window.window_handle().window_id(), suppressed, cx);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .w(px(0.))
        .h(px(0.))
        .into_any_element()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = suppressed;
        div().into_any_element()
    }
}
