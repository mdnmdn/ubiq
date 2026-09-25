//! The markdown viewer header's customisation popover — proposal §12
//! (`_docs/inbox/markdown-improvement-proposal.md`), T-118, plus T-188's reading options.
//!
//! One control, `kit::popover`, holding the things the proposal's control panel asks for: the
//! width preset and density T-116 drew as loose pills in the header (folded in here, and removed
//! from the header itself), the minimap's visibility and side, generalised from the plan modal's
//! own `plan_minimap` flag so the standard viewer and the plan surface share one setting, plus
//! (T-188) a character-size slider and a four-way text-colour picker, both per document and in
//! memory only — [`crate::state::editor::MdReading`] — and a button that writes the current pair
//! down as what a newly opened document starts from.
//!
//! **Two persistence scopes, one landed.** `md_width`/`md_density`/the minimap flags already
//! persist window-wide through `UiSettings` (`AppState::remember_settings`), which rides
//! `Message::SetSettings { layer: SettingsLayer::Ui }` — a layer the host stores as an opaque blob
//! and never parses, exactly the shape a second "system-wide default" for `MdReading` needs, so
//! [`AppState::make_md_reading_default`] reuses it rather than inventing anything. **A per-project
//! default has no such landing spot.** `ubiq-proto/src/messages.rs` carries no message that scopes
//! a settings blob to one project — `SetSettings` is window-wide, `HostSettings` is machine-wide —
//! so a "make default, per project" button would need one. It would have to carry at least a
//! `ProjectId` and an opaque value the interface encodes and decodes on the same terms `UiSettings`
//! already does (a schema, discarded whole on a mismatch), most likely `SetProjectSettings { project:
//! ProjectId, value: String }` echoed back as part of whatever a project's own settings record
//! already answers with when a project opens. This card stops at reporting that shape rather than
//! adding it — see `AppState::make_md_reading_default`'s own doc comment.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{
    AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Rgba, StatefulInteractiveElement as _, Styled as _, Subscription, div, px,
};
use gpui_component::slider::{SliderEvent, SliderState};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::editor::{MD_CHAR_SCALE_MAX, MD_CHAR_SCALE_MIN, OpenFile, TextShade};
use crate::theme::{MdDensity, MdMinimapSide, MdWidth};
use crate::ui::kit::{
    Slider, UbiqIcon, check_box, choice_pill, ghost_button, icon_button, popover, section_label,
    slider_state,
};
use crate::ui::{eid, eid2, handler};

/// How wide the popover draws — wide enough for the widest pill row (three width presets) on one
/// line at the default scale.
const PANEL_WIDTH: f32 = 220.;

/// Whether the popover is up.
pub fn is_open(app: &AppState) -> bool {
    app.workbench.open_menu == Some(MenuId::MdOptions)
}

/// The header's trigger: an icon-only button — tooltip carries the meaning, per the kit's own
/// rule for icon-only controls — that opens the panel beside it when lit.
pub fn control(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    let open = is_open(app);
    let mut trigger = icon_button(
        "md-options-control",
        UbiqIcon::TitlebarSettings,
        open,
        cx.listener(|this, _, _, cx| this.open_md_options(cx)),
    )
    .tooltip(|window, cx| {
        gpui_component::tooltip::Tooltip::new(
            "Reading options \u{2014} width, density, minimap, size, colour",
        )
        .build(window, cx)
    });
    if open {
        trigger = trigger.child(panel(app, file, cx));
    }
    trigger.into_any_element()
}

/// The panel itself: width, density, the minimap's two facets, then T-188's own three rows.
fn panel(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    let ui = &app.workbench.settings.ui;
    let view = cx.entity();
    let key = file.key();
    let reading = file.md_reading;

    popover(
        ElementId::Name("md-options-popover".into()),
        px(PANEL_WIDTH),
        Some("md-options-popover"),
        Some(std::rc::Rc::new(handler(&view, |this, _, cx| {
            this.close_menu(cx)
        }))),
        vec![
            row("width", width_pills(ui.md_width, cx)),
            row("density", density_pills(ui.md_density, cx)),
            minimap_row(ui.md_minimap, cx),
            row("minimap side", side_pills(ui.md_minimap_side, cx)),
            row(
                "character size",
                char_size_slider(&key, reading.char_scale, cx),
            ),
            row("text colour", shade_swatches(&key, reading.text_shade, cx)),
            make_default_row(&key, cx),
        ],
    )
}

/// One labelled row: a section label above its control, the same shape every group in the panel
/// takes.
fn row(label: &str, control: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .py_1()
        .child(section_label(label))
        .child(control)
        .into_any_element()
}

fn width_pills(current: MdWidth, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(MdWidth::ALL.iter().copied().map(|width| {
            choice_pill(
                eid2("md-options-width", "popover", width.label()),
                width.label(),
                current == width,
                cx.listener(move |this, _, _, cx| this.set_md_width(width, cx)),
            )
        }))
        .into_any_element()
}

fn density_pills(current: MdDensity, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(MdDensity::ALL.iter().copied().map(|density| {
            choice_pill(
                eid2("md-options-density", "popover", density.label()),
                density.label(),
                current == density,
                cx.listener(move |this, _, _, cx| this.set_md_density(density, cx)),
            )
        }))
        .into_any_element()
}

fn side_pills(current: MdMinimapSide, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(MdMinimapSide::ALL.iter().copied().map(|side| {
            choice_pill(
                eid2("md-options-side", "popover", side.label()),
                side.label(),
                current == side,
                cx.listener(move |this, _, _, cx| this.set_md_minimap_side(side, cx)),
            )
        }))
        .into_any_element()
}

/// The one on/off row of the panel — a checkbox rather than a pill, since it is a single flag
/// beside a label rather than a set to pick one of.
fn minimap_row(visible: bool, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .child(section_label("minimap"))
        .child(check_box(
            "md-options-minimap",
            visible,
            cx.listener(|this, _, _, cx| this.toggle_md_minimap(cx)),
        ))
        .into_any_element()
}

thread_local! {
    /// One character-size slider per open document (T-188), lazily built the first time its
    /// popover draws — on `crate::ui::viewer::markdown::SCAN_CACHE`'s own footing: a tab's worth
    /// of UI state is small, and a session opens few enough documents that never evicting this is
    /// not a problem worth a lifecycle to solve. The `Subscription` has to be held somewhere for
    /// as long as the entity is or the library drops it, so the two travel together.
    ///
    /// **Known gap:** the slider is seeded from the document's value once, at creation, so a tab
    /// closed and reopened under the same key sees whatever position the slider was last left at
    /// rather than the fresh `MdReading::default()` the reopened file actually holds, until it is
    /// dragged. The value the document actually draws at is never wrong — only the slider's own
    /// handle can read stale for one open.
    static CHAR_SLIDERS: RefCell<HashMap<String, (Entity<SliderState>, Subscription)>> =
        RefCell::new(HashMap::new());
}

/// How many stops the character-size ladder offers between
/// [`crate::state::editor::MD_CHAR_SCALE_MIN`] and `_MAX` — a step of `0.1` over a range of `0.8`.
const CHAR_SCALE_STOPS: usize = 9;

fn char_slider_entity(key: &str, current: f32, cx: &mut Context<AppState>) -> Entity<SliderState> {
    CHAR_SLIDERS.with_borrow_mut(|cache| {
        if let Some((entity, _)) = cache.get(key) {
            return entity.clone();
        }
        let entity = cx.new(|_| {
            slider_state(
                MD_CHAR_SCALE_MIN,
                MD_CHAR_SCALE_MAX,
                CHAR_SCALE_STOPS,
                current,
            )
        });
        let owned_key = key.to_string();
        let subscription = cx.subscribe(&entity, move |this, _slider, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event else {
                return;
            };
            this.set_md_char_scale(&owned_key, value.end(), cx);
        });
        cache.insert(key.to_string(), (entity.clone(), subscription));
        entity
    })
}

/// The character-size slider (T-188): a multiplier over the system font size, never an absolute
/// point size of its own — `MdReading::char_scale`, per document, in memory only.
fn char_size_slider(key: &str, current: f32, cx: &mut Context<AppState>) -> AnyElement {
    let entity = char_slider_entity(key, current, cx);
    Slider::new(
        "md-options-char-size",
        &entity,
        "Character size \u{2014} a multiplier over the system font size",
    )
    .leading(UbiqIcon::SizeTextSmall)
    .trailing(UbiqIcon::SizeTextLarge)
    .into_any_element()
}

/// One rectangle of the four-way text-colour picker.
fn shade_swatch(
    key: &str,
    shade: TextShade,
    current: TextShade,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let selected = shade == current;
    let colour: Rgba = shade.colour();
    let owned_key = key.to_string();
    div()
        .id(eid("md-options-shade", shade.label()))
        .size(px(20.))
        .flex_none()
        .bg(colour)
        .border_2()
        .border_color(if selected {
            crate::theme::accent()
        } else {
            crate::theme::border()
        })
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| this.set_md_text_shade(&owned_key, shade, cx)))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(shade.label()).build(window, cx)
        })
        .into_any_element()
}

/// The four rectangles side by side — four steps of lightness/darkness for body text, each a
/// theme token (`theme::text_faint`/`text_muted`/`text`/`text_strong`), never a computed alpha
/// over one.
fn shade_swatches(key: &str, current: TextShade, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(
            TextShade::ALL
                .iter()
                .copied()
                .map(|shade| shade_swatch(key, shade, current, cx)),
        )
        .into_any_element()
}

/// "Make default, system-wide": this tab's own character size and text colour, written down as
/// what a newly opened document starts from — see `AppState::make_md_reading_default` and this
/// module's own doc comment for why there is no "per project" beside it yet.
fn make_default_row(key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let owned_key = key.to_string();
    div()
        .flex()
        .px_2()
        .py_1()
        .child(
            ghost_button(
                "md-options-make-default",
                None,
                "Make default (system-wide)",
                move |_, _window, cx| {
                    let key = owned_key.clone();
                    view.update(cx, |this, cx| this.make_md_reading_default(&key, cx));
                },
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(
                    "Every newly opened document starts at this size and colour",
                )
                .build(window, cx)
            }),
        )
        .into_any_element()
}
