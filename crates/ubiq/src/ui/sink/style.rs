//! The style reference: every token, surface, control and field on one page.
//!
//! **One entry per thing the interface is built out of, and nothing that only exists here.** The
//! page draws the theme's tokens and the kit's primitives — the same functions every screen calls —
//! so a token whose value is wrong in one palette, a control whose off state reads as absent, or a
//! surface whose coloured edge floats a few pixels inside its container all show up on this page
//! before they show up on a screen.
//!
//! It is also where a primitive with no call site yet gets one. `Kbd` and the `border_focus` token
//! are drawn here, which is the difference between a convention that is available and a convention
//! nobody can see.
//!
//! Every interactive thing on the page is wired to real state on [`crate::state::sink::SinkState`],
//! because a control that cannot hold a value is not being tested. Nothing it holds means anything:
//! this is the sink.

use gpui::{
    AnyElement, App, Context, ElementId, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
    relative,
};
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
use gpui_component::kbd::Kbd;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::sink::{CHOICES, FACETS, MENU_ITEMS, SinkModal};
use crate::theme;
use crate::ui::kit::{
    Picker, PickerStyle, Tab, badge, card, choice_pill, disclosure, ghost_button, icon_button,
    meter, mono, panel_header, pill, primary_button, progress_ring, section_label, slab,
    state_chip, status_dot, stepper, tab_strip, toggle_pill,
};
use crate::ui::{handler, indexed};

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id("sink-style")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .overflow_y_scroll()
        .child(tokens())
        .child(typography())
        .child(surfaces(cx))
        .child(controls(app, cx))
        .child(fields(app, window, cx))
        .child(modals(cx))
        .into_any_element()
}

// ── Tokens ──────────────────────────────────────────────────────────

/// Every colour the interface has, by the name a call site reaches it under.
///
/// Grouped exactly as `theme.rs` groups them, because the grouping is the claim: a token names what
/// a colour is *for*, and two tokens in one group are two roles rather than two shades.
fn tokens() -> AnyElement {
    let mut swatches: Vec<(&'static str, Rgba)> = vec![
        ("app_bg", theme::app_bg()),
        ("pane_bg", theme::pane_bg()),
        ("surface", theme::surface()),
        ("surface_raised", theme::surface_raised()),
        ("hover", theme::hover()),
        ("selected", theme::selected()),
        ("scrim", theme::scrim()),
        ("text", theme::text()),
        ("text_muted", theme::text_muted()),
        ("text_faint", theme::text_faint()),
        ("on_accent", theme::on_accent()),
        ("accent", theme::accent()),
        ("accent_muted", theme::accent_muted()),
        ("accent_soft", theme::accent_soft()),
        ("border", theme::border()),
        ("border_focus", theme::border_focus()),
        ("danger", theme::danger()),
        ("danger_soft", theme::danger_soft()),
        ("success", theme::success()),
        ("success_soft", theme::success_soft()),
        ("warning", theme::warning()),
        ("warning_soft", theme::warning_soft()),
        ("info", theme::info()),
        ("info_soft", theme::info_soft()),
    ];

    // The project group's members carry no role, so they are numbered rather than named.
    let projects: Vec<AnyElement> = (0..theme::project_colour_count())
        .map(|index| swatch_of(index.to_string(), theme::project_colour(index)))
        .collect();

    let named: Vec<AnyElement> = swatches
        .drain(..)
        .map(|(name, colour)| swatch_of(name.to_string(), colour))
        .collect();

    group(
        "Tokens",
        "Every accessor in theme.rs, in both palettes. A swatch that vanishes has no value in \
         this one.",
        vec![
            row(named),
            labelled(
                "project_colour(n)",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(projects)
                    .into_any_element(),
            ),
        ],
    )
}

/// One colour, over its own name. The name sits under the block rather than on it, because a token
/// this page cannot read is exactly the token this page is for.
fn swatch_of(name: impl Into<SharedString>, colour: Rgba) -> AnyElement {
    div()
        .w(px(92.))
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .child(
            div()
                .h(px(34.))
                .w_full()
                .bg(colour)
                .border_1()
                .border_color(theme::border()),
        )
        .child(mono(name, theme::text_muted()).text_size(px(10.)))
        .into_any_element()
}

// ── Type ────────────────────────────────────────────────────────────

/// The sizes the window actually uses, each said at the size it is.
fn typography() -> AnyElement {
    let sizes = [
        (15.0, "15 · a screen's title"),
        (13.5, "13.5 · a composer, a menu row"),
        (12.5, "12.5 · body copy, a tab, a button"),
        (11.0, "11 · a footnote, a badge"),
    ];

    let lines: Vec<AnyElement> = sizes
        .iter()
        .map(|(size, label)| {
            div()
                .text_size(px(*size))
                .text_color(theme::text())
                .child(SharedString::from(*label))
                .into_any_element()
        })
        .collect();

    group(
        "Type",
        "Four sizes, one mono family, and the three text tiers.",
        vec![
            div()
                .flex()
                .flex_col()
                .gap_1()
                .children(lines)
                .into_any_element(),
            row(vec![
                labelled("text", tinted("primary copy", theme::text())),
                labelled("text_muted", tinted("secondary copy", theme::text_muted())),
                labelled(
                    "text_faint",
                    tinted("a hint, a timestamp", theme::text_faint()),
                ),
            ]),
            row(vec![
                labelled(
                    "mono",
                    mono("crates/ubiq/src/theme.rs:42", theme::text()).into_any_element(),
                ),
                labelled(
                    "section_label",
                    section_label("a group heading").into_any_element(),
                ),
                labelled(
                    "badge",
                    div()
                        .flex()
                        .gap_2()
                        .child(badge("M", theme::warning()))
                        .child(badge("A", theme::success()))
                        .child(badge("ignored", theme::text_faint()))
                        .into_any_element(),
                ),
                labelled("Kbd", keystroke("cmd-s")),
                labelled("Icon", icons()),
            ]),
        ],
    )
}

fn tinted(text: &str, colour: Rgba) -> AnyElement {
    div()
        .text_size(px(12.5))
        .text_color(colour)
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

/// The component library's key badge. Nothing else in the window draws one yet.
fn keystroke(binding: &str) -> AnyElement {
    match gpui::Keystroke::parse(binding) {
        Ok(stroke) => Kbd::new(stroke).into_any_element(),
        Err(_) => mono(binding.to_string(), theme::text_faint()).into_any_element(),
    }
}

// ── Surfaces ────────────────────────────────────────────────────────

/// The shape everything is drawn in, and the three things it becomes.
fn surfaces(cx: &mut Context<AppState>) -> AnyElement {
    let slabs = row(vec![
        labelled(
            "slab(accent)",
            slab(theme::accent())
                .w(px(180.))
                .p_2()
                .child(tinted("a surface", theme::text()))
                .into_any_element(),
        ),
        labelled(
            "slab(danger)",
            slab(theme::danger())
                .w(px(180.))
                .p_2()
                .child(tinted("something reported", theme::text()))
                .into_any_element(),
        ),
        labelled(
            "slab(border_focus)",
            slab(theme::border_focus())
                .w(px(180.))
                .p_2()
                .child(tinted("the focused surface", theme::text()))
                .into_any_element(),
        ),
    ]);

    let cards = row(vec![
        labelled(
            "card",
            card("sink-card", theme::success(), false)
                .w(px(200.))
                .p_2()
                .child(tinted("a card you can pick", theme::text()))
                .into_any_element(),
        ),
        labelled(
            "card · selected",
            card("sink-card-on", theme::success(), true)
                .w(px(200.))
                .p_2()
                .child(tinted("the one being looked at", theme::text()))
                .into_any_element(),
        ),
        labelled(
            "pill",
            pill(theme::info())
                .child(mono("a chip", theme::text()).text_size(px(11.5)))
                .into_any_element(),
        ),
    ]);

    let chrome = div()
        .flex()
        .flex_col()
        .w(relative(1.))
        .bg(theme::pane_bg())
        .border_1()
        .border_color(theme::border())
        .child(panel_header(
            "panel_header",
            div()
                .flex()
                .gap_1()
                .child(ghost_button(
                    "sink-header-action",
                    Some(IconName::Plus),
                    "New",
                    |_, _, _| {},
                ))
                .child(icon_button(
                    "sink-header-icon",
                    IconName::Settings,
                    false,
                    |_, _, _| {},
                ))
                .into_any_element(),
        ))
        .child(tab_strip(
            "sink-demo-tabs",
            vec![
                Tab::new("open").dot(theme::success()).closable(true),
                Tab::new("dirty \u{2022}")
                    .dot(theme::warning())
                    .closable(true),
                Tab::new("failed").dot(theme::danger()).closable(true),
            ],
            0,
            |_, _, _| {},
            None,
            Some(
                mono("trailing", theme::text_faint())
                    .text_size(px(10.5))
                    .into_any_element(),
            ),
        ))
        .child(
            div()
                .p_3()
                .child(tinted("a panel's body", theme::text_muted())),
        )
        .child(disclosure(
            "sink-disclosure",
            "disclosure",
            mono("3 open", theme::text_faint())
                .text_size(px(11.))
                .into_any_element(),
            false,
            cx.listener(|this, _, _, cx| this.toggle_sink_disclosure(cx)),
        ))
        .into_any_element();

    group(
        "Surfaces",
        "Square, filled, identified by the left edge — and the edge sits on the container's own \
         boundary, never inset from it.",
        vec![slabs, cards, chrome],
    )
}

// ── Controls ────────────────────────────────────────────────────────

/// Everything that takes a click, in both of its states where it has two.
fn controls(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let sink = &app.sink;

    let buttons = row(vec![
        labelled(
            "primary_button",
            primary_button("sink-primary", Some(IconName::Play), "Run", |_, _, _| {})
                .into_any_element(),
        ),
        labelled(
            "ghost_button",
            ghost_button("sink-ghost", Some(IconName::Plus), "New chat", |_, _, _| {})
                .into_any_element(),
        ),
        labelled(
            "icon_button",
            div()
                .flex()
                .gap_1()
                .child(icon_button(
                    "sink-icon",
                    IconName::Search,
                    false,
                    |_, _, _| {},
                ))
                .child(icon_button(
                    "sink-icon-on",
                    IconName::PanelBottom,
                    true,
                    |_, _, _| {},
                ))
                .into_any_element(),
        ),
        labelled(
            "Picker",
            Picker::new("sink-picker", MENU_ITEMS[sink.picked])
                .items(MENU_ITEMS)
                .selected(sink.picked)
                .style(PickerStyle::Chip)
                .open(app.workbench.open_menu == Some(MenuId::SinkPicker))
                .on_toggle(handler(&cx.entity(), |this, _, cx| {
                    this.open_menu(MenuId::SinkPicker, cx)
                }))
                .on_pick(indexed(&cx.entity(), |this, index, _, cx| {
                    this.pick_sink_menu(index, cx)
                }))
                .on_dismiss(handler(&cx.entity(), |this, _, cx| this.close_menu(cx)))
                .into_any_element(),
        ),
    ]);

    let facets: Vec<AnyElement> = FACETS
        .iter()
        .enumerate()
        .map(|(index, label)| {
            toggle_pill(
                ElementId::Name(format!("sink-facet-{index}").into()),
                *label,
                bucket_colour(index),
                sink.facets[index],
                cx.listener(move |this, _, _, cx| this.toggle_sink_facet(index, cx)),
            )
            .into_any_element()
        })
        .collect();

    let choices: Vec<AnyElement> = CHOICES
        .iter()
        .enumerate()
        .map(|(index, label)| {
            choice_pill(
                ElementId::Name(format!("sink-choice-{index}").into()),
                *label,
                sink.choice == index,
                cx.listener(move |this, _, _, cx| this.set_sink_choice(index, cx)),
            )
            .into_any_element()
        })
        .collect();

    let pills = row(vec![
        labelled(
            "toggle_pill",
            div().flex().gap_1().children(facets).into_any_element(),
        ),
        labelled(
            "choice_pill",
            div().flex().gap_1().children(choices).into_any_element(),
        ),
    ]);

    let reports = row(vec![
        labelled(
            "status_dot",
            div()
                .flex()
                .gap_2()
                .child(status_dot(theme::success(), theme::success_soft()))
                .child(status_dot(theme::warning(), theme::warning_soft()))
                .child(status_dot(theme::danger(), theme::danger_soft()))
                .child(status_dot(theme::info(), theme::info_soft()))
                .into_any_element(),
        ),
        labelled(
            "state_chip",
            div()
                .flex()
                .gap_1()
                .child(state_chip("running", theme::success(), 1.0))
                .child(state_chip("waiting", theme::warning(), 1.0))
                .child(state_chip("error", theme::danger(), 1.0))
                .into_any_element(),
        ),
    ]);

    // One value driving three controls, so a nudge is visible in all of them at once.
    let level = row(vec![
        labelled(
            "stepper",
            stepper(
                "sink-stepper",
                format!("{}%", sink.level),
                cx.listener(|this, _, _, cx| this.nudge_sink(-10, cx)),
                cx.listener(|this, _, _, cx| this.nudge_sink(10, cx)),
            )
            .into_any_element(),
        ),
        labelled(
            "meter",
            div()
                .w(px(160.))
                .child(meter(sink.fraction(), theme::accent()))
                .into_any_element(),
        ),
        labelled(
            "progress_ring",
            progress_ring(sink.level, 28.0).into_any_element(),
        ),
    ]);

    group(
        "Controls",
        "A toggle is an independent facet; a choice is one value of a set. Off keeps its outline \
         so turning it back on does not move the row.",
        vec![buttons, pills, reports, level],
    )
}

/// The colour each bucket of agent states is drawn in. The same four the graph's filter row uses.
fn bucket_colour(index: usize) -> Rgba {
    match index {
        0 => theme::success(),
        1 => theme::warning(),
        2 => theme::text_muted(),
        _ => theme::danger(),
    }
}

// ── Fields ──────────────────────────────────────────────────────────

/// The component library's own inputs, on the surface Ubiq puts them on.
///
/// A field is a library widget in a Ubiq container: the widget draws no border of its own —
/// `appearance(false)` — and the container is the surface, with the coloured edge on its boundary.
fn fields(app: &AppState, window: &Window, cx: &App) -> AnyElement {
    group(
        "Fields",
        "The library's Input and Textarea, drawn with their own appearance off so the container is \
         the surface. The active one is underlined.",
        vec![
            labelled(
                "Input",
                framed_active(theme::border(), input_on(&app.sink_input, window, cx))
                    .h(px(30.))
                    .items_center()
                    .child(Input::new(&app.sink_input).appearance(false))
                    .into_any_element(),
            ),
            labelled(
                "Input · focused edge",
                framed_active(theme::border_focus(), true)
                    .h(px(30.))
                    .items_center()
                    .child(mono(
                        "what a focused field's edge says",
                        theme::text_muted(),
                    ))
                    .into_any_element(),
            ),
            labelled(
                "Textarea",
                framed_active(theme::accent(), textarea_on(&app.sink_textarea, window, cx))
                    .p_2()
                    .child(
                        Textarea::new(&app.sink_textarea)
                            .appearance(false)
                            .bordered(false)
                            .w_full()
                            .text_size(px(13.)),
                    )
                    .into_any_element(),
            ),
        ],
    )
}

/// The container a field sits in: a surface with its edge on the boundary and nothing rounded.
pub fn framed(edge: Rgba) -> gpui::Div {
    framed_active(edge, false)
}

/// The same container, with the focused field's underline.
///
/// A field is identified on the left like every other surface; when it holds the keyboard the
/// bottom edge lights as well, so the active box is the one that is underlined.
pub fn framed_active(edge: Rgba, focused: bool) -> gpui::Div {
    let colour = if focused { theme::border_focus() } else { edge };
    let mut root = div()
        .w(relative(1.))
        .px_2()
        .flex()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(colour);
    if focused {
        root = root.border_b(px(theme::ACCENT_EDGE));
    }
    root
}

pub fn input_on(state: &Entity<InputState>, window: &Window, cx: &App) -> bool {
    state.read(cx).focus_handle(cx).is_focused(window)
}

pub fn textarea_on(state: &Entity<TextareaState>, window: &Window, cx: &App) -> bool {
    state.read(cx).focus_handle(cx).is_focused(window)
}

// ── Modals ──────────────────────────────────────────────────────────

/// The three shapes a modal comes in, each raised by its own trigger.
fn modals(cx: &mut Context<AppState>) -> AnyElement {
    let triggers: Vec<AnyElement> = [
        (SinkModal::Confirm, "A question"),
        (SinkModal::Form, "A form"),
        (SinkModal::Danger, "Something irreversible"),
    ]
    .into_iter()
    .map(|(which, label)| {
        ghost_button(
            ElementId::Name(format!("sink-modal-{label}").into()),
            Some(IconName::Inspector),
            label,
            cx.listener(move |this, _, _, cx| this.open_sink_modal(which, cx)),
        )
        .into_any_element()
    })
    .collect();

    group(
        "Modals",
        "One question, over the window. Dismissed by a click outside it or by its own close; the \
         scrim is a token, so both palettes dim by what their ground needs.",
        vec![row(triggers)],
    )
}

// ── The page's own furniture ─────────────────────────────────────────

/// One group of the reference: what it is called, what its rule is, and the specimens.
pub fn group(title: &str, note: &str, children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap_3()
        .px_4()
        .py_4()
        .border_b_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(section_label(title))
                .child(
                    div()
                        .max_w(px(620.))
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(note.to_string())),
                ),
        )
        .children(children)
        .into_any_element()
}

/// A row of specimens that wraps rather than pushing the page sideways.
///
/// Public for the same reason [`labelled`] is: the picker page is another page of the sink, and one
/// page shape is what makes the sink readable as one bench.
pub fn row(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_end()
        .gap_4()
        .children(children)
        .into_any_element()
}

/// One specimen, under the name a call site reaches it by.
///
/// Public because the style reference's form modal wants a field labelled the same way, and one
/// label shape is the point of the page.
pub fn labelled(name: &str, child: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .font_family(theme::MONO_FONT)
                .text_size(px(10.))
                .text_color(theme::text_faint())
                .child(SharedString::from(name.to_string())),
        )
        .child(child)
        .into_any_element()
}

/// A few of the glyphs the window is built out of.
///
/// The page names them because they are the one thing the kit does not own: Ubiq ships no icon set,
/// so every glyph is the nearest one in the component library's bundle — which is a gap in
/// `_docs/backlog.md`, and a gap that is easier to argue about while looking at it.
fn icons() -> AnyElement {
    let glyphs = [
        IconName::LayoutDashboard,
        IconName::SquareTerminal,
        IconName::Asterisk,
        IconName::BookOpen,
        IconName::CircleCheck,
        IconName::Palette,
    ];

    div()
        .flex()
        .items_center()
        .gap_2()
        .children(glyphs.map(|glyph| {
            Icon::new(glyph)
                .with_size(Size::Medium)
                .text_color(theme::text_muted())
        }))
        .into_any_element()
}
