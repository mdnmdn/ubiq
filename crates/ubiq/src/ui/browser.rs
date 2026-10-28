//! The internal browser's tab: an action bar over a native webview.
//!
//! The bar is the markdown viewer header's shape — a 32px row on `pane_bg` with a bottom rule and
//! flush icon buttons. The page under it is `ui::web_view`'s rectangle; a webview is a native view
//! stacked over the window, so while the address dropdown is down `ui::shell::overlaid` hides it
//! rather than let it cover the list.
//!
//! The dropdown is the ⌘K navigator's: `deferred(anchored(..))` hung off the field, key context and
//! handlers on the field itself.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Focusable as _, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, ParentElement, StatefulInteractiveElement as _, Styled, Transformation, anchored,
    deferred, div, percentage, point, pulsating_between, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::browser::BrowserTab;
use crate::theme;
use crate::ui::dock::TabInfo;
use crate::ui::kit::controls::pulse_animation;
use crate::ui::kit::{
    ContextItem, context_menu, elided, field, file_row, icon_button, icon_button_tip, row_font,
};
use crate::ui::{eid, web_view};

/// The bar's height: the viewer header's.
const HEADER: f32 = 32.0;

/// The longest a tab label is drawn before it is cut; the tooltip carries the whole address.
const LABEL_MAX: usize = 28;

const CONTEXT: &str = "BrowserBar";
const FIELD_CONTEXT: &str = "BrowserBar > Input";

gpui::actions!(
    ubiq_browser,
    [BrowserUp, BrowserDown, BrowserEnter, BrowserDismiss]
);

/// The address field's keys, bound for the bar and for the input inside it — the navigator's
/// device, for its reason (the input binds the same keys one node deeper).
pub fn key_bindings() -> Vec<KeyBinding> {
    fn both<A: gpui::Action + Clone>(key: &str, action: A) -> [KeyBinding; 2] {
        [
            KeyBinding::new(key, action.clone(), Some(CONTEXT)),
            KeyBinding::new(key, action, Some(FIELD_CONTEXT)),
        ]
    }
    [
        both("up", BrowserUp),
        both("down", BrowserDown),
        both("enter", BrowserEnter),
        both("escape", BrowserDismiss),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn tab_of<'a>(app: &'a AppState, key: &str, cx: &gpui::App) -> Option<&'a BrowserTab> {
    app.open_project(cx)?.browser.tab(key)
}

/// The dock tab: the page's title (else its host), a globe before it.
pub fn tab(app: &AppState, key: &str, cx: &gpui::App) -> TabInfo {
    let Some(tab) = tab_of(app, key, cx) else {
        return TabInfo {
            label: "Browser".into(),
            icon: Some(IconName::Globe),
            ..TabInfo::default()
        };
    };
    let label = tab.label();
    let short = match label.chars().count() > LABEL_MAX {
        true => format!("{}\u{2026}", label.chars().take(LABEL_MAX - 1).collect::<String>()),
        false => label,
    };
    TabInfo {
        label: short.into(),
        tooltip: Some(tab.url.clone().into()).filter(|url: &gpui::SharedString| !url.is_empty()),
        icon: Some(IconName::Globe),
        ..TabInfo::default()
    }
}

/// The tab's body: the bar, then the page.
pub fn render(
    app: &AppState,
    key: &str,
    window: &gpui::Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(tab) = tab_of(app, key, cx) else {
        return div().into_any_element();
    };
    let loading = tab.loading;
    let body = web_view::element(key).unwrap_or_else(|| {
        let notice = match web_view::SUPPORTED {
            true => "Starting the browser\u{2026}",
            false => "The internal browser is not available on this platform.",
        };
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
            .text_color(theme::text_faint())
            .child(notice)
            .into_any_element()
    });

    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(theme::pane_bg())
        .child(bar(app, tab, loading, window, cx))
        .child(body)
        .into_any_element()
}

fn bar(
    app: &AppState,
    tab: &BrowserTab,
    loading: bool,
    window: &gpui::Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let key = tab.key.clone();
    let (back, forward, stop) = (key.clone(), key.clone(), key.clone());

    let mut row = div()
        .h(px(HEADER))
        .flex()
        .flex_none()
        .items_center()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(icon_button_tip(
            eid("browser-back", &key),
            IconName::ArrowLeft,
            "Back",
            true,
            cx.listener(move |this, _, _, cx| {
                this.browser_back(&back, cx);
            }),
        ))
        .child(icon_button_tip(
            eid("browser-forward", &key),
            IconName::ArrowRight,
            "Forward",
            true,
            cx.listener(move |this, _, _, cx| {
                this.browser_forward(&forward, cx);
            }),
        ))
        .child(loader(&key, loading));

    if loading {
        row = row.child(stop_button(&key, stop, cx));
    }
    row.child(address(app, tab, window, cx))
        .child(more(app, tab, cx))
        .into_any_element()
}

/// The bar's `⋯`, in the viewer header's idiom: an icon button that puts a context menu down where
/// it was clicked. Zoom, per tab, with the current percentage on the row that resets it.
fn more(app: &AppState, tab: &BrowserTab, cx: &mut Context<AppState>) -> AnyElement {
    let key = tab.key.clone();
    let menu = app
        .browser_more
        .clone()
        .filter(|(at, _)| *at == key && app.workbench.open_menu == Some(MenuId::BrowserMore));
    let trigger = key.clone();
    let mut root = div().h_full().flex().flex_none().items_center().child(
        icon_button(
            eid("browser-more", &key),
            IconName::EllipsisVertical,
            menu.is_some(),
            cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                let at = event.position();
                this.open_browser_more(&trigger, (at.x.into(), at.y.into()), cx);
            }),
        )
        .h_full()
        .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("More: zoom").build(window, cx)),
    );
    if let Some((_, at)) = menu {
        let percent = format!("{}%", (tab.zoom * 100.0).round() as i64);
        let mut items = Vec::new();
        if web_view::SCREENSHOT {
            items.push(ContextItem::new("Copy screenshot"));
        }
        items.extend([
            ContextItem::new("Zoom in"),
            ContextItem::new("Zoom out"),
            ContextItem::new("Reset zoom").detail(percent),
        ]);
        let view = cx.entity();
        let target = key.clone();
        root = root.child(context_menu(
            eid("browser-more-menu", &key),
            point(px(at.0), px(at.1)),
            items,
            crate::ui::indexed(&view, move |this, index, _, cx| {
                this.dismiss_browser_more(cx);
                let index = match web_view::SCREENSHOT {
                    true => match index.checked_sub(1) {
                        Some(index) => index,
                        None => {
                            this.browser_screenshot(&target, cx);
                            return;
                        }
                    },
                    false => index,
                };
                let step = match index {
                    0 => Some(true),
                    1 => Some(false),
                    _ => None,
                };
                this.browser_zoom(&target, step, cx);
            }),
            crate::ui::handler(&view, |this, _, cx| this.dismiss_browser_more(cx)),
        ));
    }
    root.into_any_element()
}

/// The 30px slot after the arrows: a spinning loader while the page loads, a quiet globe after.
fn loader(key: &str, loading: bool) -> AnyElement {
    let slot = div()
        .size(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center();
    match loading {
        true => slot.child(
            Icon::new(IconName::LoaderCircle)
                .with_size(Size::Small)
                .text_color(theme::accent())
                .with_animation(
                    eid("browser-spin", key),
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
                ),
        ),
        false => slot.child(
            Icon::new(IconName::Globe)
                .with_size(Size::Small)
                .text_color(theme::text_faint()),
        ),
    }
    .into_any_element()
}

/// The red, pulsing, round stop button; only drawn while the page is loading.
fn stop_button(key: &str, target: String, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id(eid("browser-stop", key))
        .w(px(24.))
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("Stop loading").build(window, cx))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.browser_stop(&target, cx);
        }))
        .child(
            div()
                .size(px(12.))
                .rounded_full()
                .bg(theme::danger())
                .with_animation(
                    eid("browser-stop-pulse", key),
                    pulse_animation(Duration::from_millis(1_000))
                        .with_easing(pulsating_between(0.35, 1.0)),
                    |dot, delta| dot.opacity(delta),
                ),
        )
        .into_any_element()
}

/// The address field, with its dropdown hung off it while one is down.
fn address(
    app: &AppState,
    tab: &BrowserTab,
    window: &gpui::Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(input) = tab.input.clone() else {
        return div().flex_1().into_any_element();
    };
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let key = tab.key.clone();
    let (up, down, enter, dismiss) = (key.clone(), key.clone(), key.clone(), key.clone());

    let mut bar = field(theme::border(), focused)
        .relative()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .px_2()
        .key_context(CONTEXT)
        .on_action(cx.listener(move |this, _: &BrowserUp, _, cx| {
            this.move_browser_dropdown(&up, false, cx)
        }))
        .on_action(cx.listener(move |this, _: &BrowserDown, _, cx| {
            this.move_browser_dropdown(&down, true, cx)
        }))
        .on_action(cx.listener(move |this, _: &BrowserEnter, _, cx| {
            this.submit_browser_bar(&enter, cx)
        }))
        // Escape closes the list and leaves the text; with no list up it is the window's.
        .on_action(cx.listener(move |this, _: &BrowserDismiss, _, cx| {
            match this.browser_dropdown.is_some() {
                true => this.close_browser_dropdown(&dismiss, cx),
                false => cx.propagate(),
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .child(Input::new(&input).appearance(false)),
        );

    if app
        .browser_dropdown
        .as_ref()
        .is_some_and(|drop| drop.key == key)
    {
        bar = bar.child(dropdown(app, &key, cx));
    }
    bar.into_any_element()
}

/// The history list, in the navigator's shape: a zero-size box at the field's bottom-left carrying
/// a deferred, anchored panel.
fn dropdown(app: &AppState, key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let found = app.browser_suggestions(key, cx);
    if found.is_empty() {
        return div().into_any_element();
    }
    let at = app
        .browser_dropdown
        .as_ref()
        .and_then(|drop| drop.cursor)
        .map(|at| at.min(found.len() - 1));

    let rows: Vec<AnyElement> = found
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let label = match entry.title.is_empty() {
                true => entry.url.clone(),
                false => entry.title.clone(),
            };
            let target = key.to_string();
            file_row(
                ("browser-row", index),
                0,
                false,
                at == Some(index),
                true,
                row_font(),
            )
            .child(elided(
                ("browser-row-label", index),
                label,
                theme::text(),
                theme::font(theme::Family::Chrome, theme::Role::Body),
            ))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .max_w(px(220.))
                    .font_family(theme::MONO_FONT)
                    .child(elided(
                        ("browser-row-url", index),
                        entry.url.clone(),
                        theme::text_faint(),
                        theme::font(theme::Family::Chrome, theme::Role::Meta),
                    )),
            )
            // Mouse-down rather than click: the field blurs on the press and takes the list with
            // it before a click would land.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.pick_browser_suggestion(&target, index, cx)),
            )
            .into_any_element()
        })
        .collect();

    let list = deferred(
        anchored().snap_to_window_with_margin(px(8.)).child(
            div()
                .id("browser-dropdown")
                .w(px(520.))
                .max_h(px(320.))
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .bg(theme::surface_raised())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::accent())
                .shadow_lg()
                .occlude()
                .children(rows),
        ),
    )
    .priority(2);

    div()
        .absolute()
        .bottom_0()
        .left_0()
        .w_0()
        .h_0()
        .child(list)
        .into_any_element()
}
