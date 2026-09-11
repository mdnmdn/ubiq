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
        let id = window.window_handle().window_id();
        // Spelled out rather than called as a method: `Window` carries both gpui's own
        // `window_handle` and `raw-window-handle`'s, and the inherent one wins.
        let raw = raw_window_handle::HasWindowHandle::window_handle(window)
            .map_err(|err| format!("this window has no native handle: {err}"))?;

        let builder = wry::WebViewBuilder::new().with_url(url);
        #[cfg(debug_assertions)]
        let builder = builder.with_devtools(true);
        let webview = builder
            .build_as_child(&raw)
            .map_err(|err| format!("could not start the embedded browser: {err}"))?;

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

/// Close the one behind `key`. Silent when there is none — a tab may close before it ever had one.
pub fn close(key: &str) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    platform::close(key);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = key;
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
