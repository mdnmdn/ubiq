//! The `Editor` half of a web panel: the embedded browser, or what is standing in for it.
//!
//! Three things can be on screen here and only ever one of them. **The browser is shown only
//! once the component inside it says it is ready** — until then it is a blank white rectangle the
//! size of the panel, so the panel draws a loader over the same space and the sweep in
//! [`crate::ui::web_view`] keeps the native view off screen. A bundle still downloading, a chrome
//! that threw, or a platform with no embedded browser at all each end up in the same place: a
//! sentence saying so, with the native painter still one press away on `Preview`.
//!
//! **The loader counts.** The host knows how many files the bundle has before it fetches any of
//! them, so the bar is a real fraction of the download and the line under it names the file that
//! last landed — a 26 MB first run says what it is doing, and one that has stopped says where.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::progress::Progress;
use gpui_component::{Sizable as _, Size};

use crate::app::AppState;
use crate::state::OpenFile;
use crate::state::editor::ViewerKind;
use crate::state::web_panel::BundleState;
use crate::theme;
use crate::ui::kit::mono;
use crate::ui::web_view;
use crate::web_export::bridge;

/// The tenant's own name, for the loader's fallback text. Every viewer that opens a web panel
/// names itself here; one that does not is never asked.
fn tenant_name(viewer: ViewerKind) -> &'static str {
    match viewer {
        ViewerKind::Excalidraw => "Excalidraw",
        ViewerKind::Drawio => "draw.io",
        _ => "the editor",
    }
}

/// Draw the editor for one tab.
pub fn render(app: &AppState, file: &OpenFile, _cx: &mut Context<AppState>) -> AnyElement {
    let key = file.key();
    let session = app.web_panels.sessions.get(&key);
    let bundle = bridge::web_app(file.viewer)
        .map(|app_id| app.web_panels.bundle(app_id))
        .unwrap_or_default();

    // A chrome that failed says so where the drawing would have been, and the buffer is untouched.
    if let Some(error) = session.and_then(|session| session.error.clone()) {
        return super::note(error, theme::danger());
    }
    if let BundleState::Unavailable { reason } = &bundle {
        return super::note(reason.clone(), theme::danger());
    }

    // Without an embedded browser the session is a window beside this one, opened when the tab
    // was switched into `Editor`. Nothing can be drawn in the panel, so it says where the drawing
    // went rather than leaving an empty box to interpret.
    if !web_view::SUPPORTED {
        return super::note(
            "Editing opens in your browser on this platform.",
            theme::text_faint(),
        );
    }

    match session.filter(|session| session.ready) {
        Some(_) => web_view::element(&key)
            .unwrap_or_else(|| loading("Starting\u{2026}", None, None, "starting the browser")),
        None => loading(
            bundle
                .unavailable()
                .unwrap_or_else(|| format!("Loading {}\u{2026}", tenant_name(file.viewer))),
            fetched(&bundle),
            bundle.fetching_file().map(str::to_string),
            "downloading the drawing editor",
        ),
    }
}

/// How far the bundle download has got, as a percentage, while there is a count to divide.
///
/// The host reports the file count before it fetches anything, so this is a real fraction of the
/// download for all but the instant between the ask and the first report. The other states answer
/// `None` and the bar sweeps instead: a component still mounting and a browser still starting take
/// an unknown amount of time, and a sweep is the honest drawing for that.
fn fetched(bundle: &BundleState) -> Option<f32> {
    match bundle {
        BundleState::Fetching { done, total, .. } if *total > 0 => {
            Some(*done as f32 / *total as f32 * 100.)
        }
        _ => None,
    }
}

/// The loader that covers the browser until the component inside it has mounted.
///
/// `value` fills the bar; `None` sweeps it. `detail` is the file the fetch last landed — the one
/// line that tells a download still moving from one that has stopped, which is why it is drawn at
/// all rather than left to the log console.
fn loading(
    text: impl Into<gpui::SharedString>,
    value: Option<f32>,
    detail: Option<String>,
    label: &'static str,
) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .flex_col()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .gap_3()
        .child(mono(text, theme::text_faint()))
        .child(
            div().w(px(260.)).child(
                Progress::new("web-panel-loading")
                    .with_size(Size::XSmall)
                    .color(theme::accent())
                    .loading(value.is_none())
                    .value(value.unwrap_or_default())
                    .accessibility_label(label),
            ),
        )
        .children(detail.map(|file| mono(file, theme::text_faint()).max_w(px(260.)).truncate()))
        .into_any_element()
}
