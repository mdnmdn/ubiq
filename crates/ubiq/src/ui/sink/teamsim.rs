//! The teamsim page: the teams graph's block-positioning arrangements, against a scenario that can
//! be loaded, edited by hand and dragged about.
//!
//! **It draws the production arrangements, not a copy of them.** Every rectangle on this canvas came
//! out of `state::layout` through `state::teamsim`, and every block, fence and connector is drawn by
//! `ui::kit::blocks` — the same module the Teams screen draws through. A new `Algo` variant appears
//! in the picker with no edit here, because the picker is `Algo::ALL`.
//!
//! **Nothing here mutates.** Every interaction is a call on `AppState`, and every measurement was
//! taken in `state::teamsim` — this file positions what it is handed and no more.
//!
//! **The clipboard is the save file.** `Copy JSON` writes the scenario, hand positions and all, and
//! `Paste JSON` reads one back; a paste that does not parse says so above the canvas and leaves what
//! is on screen alone. A bench with a store behind it would be a screen.
//!
//! **A card carries its own two buttons**: `+sub` gives it a delegate, `link` arms an edge whose
//! target is the next card clicked. The armed card wears the accent, because a mode with no mark on
//! screen is a mode nobody can leave.

use gpui::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, Entity, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Window, div, point, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::layout::{Algo, GROUP_LABEL, GROUP_PAD};
use crate::state::teams::ZOOM_STEP;
use crate::state::teamsim::{
    Drawing, EdgeKind, LinkKind, PRESETS, SESSION_LABEL, SESSION_PAD, Sim, TeamsimHeld,
};
use crate::state::workbench::MenuId;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::blocks::{self, Board, Fence, Look, Word};
use crate::ui::kit::canvas::Link;
use crate::ui::kit::menu::{Picker, PickerStyle};
use crate::ui::kit::{
    UbiqIcon, ghost_button, harness_icon, icon_button, mono, primary_button, section_label, stepper,
};
use crate::ui::work::{activity_colour, role_mark};
use crate::ui::{eid, eid2, handler, indexed};

/// What the pointer is carrying. Its own type rather than the Teams screen's, because a drag is
/// bound to the state it writes into and this page writes into the sink's.
#[derive(Clone)]
pub struct Carried(pub TeamsimHeld);

/// GPUI wants a view for the drag preview. What is being dragged is already on the canvas and
/// already following the pointer, so this one draws nothing.
struct Empty;

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The room left past the outermost thing on the canvas, so a block at the edge can still be picked
/// up and put down without fighting the scroll.
const MARGIN: f32 = 60.0;

/// How wide the two dropdowns are. A `PickerStyle::Field` takes the width it is given, and in a flex
/// row it would take all of it and push the buttons off the end.
const PRESET_WIDTH: f32 = 200.0;
const ALGO_WIDTH: f32 = 320.0;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let sim = &app.sink.teamsim.sim;
    let drawing = sim.drawing();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(toolbar(app, cx))
        .child(strip(app, &drawing, cx))
        .children(app.sink.teamsim.error.clone().map(problem))
        .child(canvas(app, &drawing, window, cx))
        .into_any_element()
}

/// The row of controls: which scenario, which arrangement, what zoom, and the five things that
/// change the scenario or move it on and off the clipboard.
fn toolbar(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let page = &app.sink.teamsim;
    let preset = PRESETS
        .get(page.preset)
        .map(|preset| preset.name)
        .unwrap_or("scenario");

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .p_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(section_label("Scenario"))
        .child(
            div().flex_none().w(px(PRESET_WIDTH)).child(
                Picker::new("teamsim-preset", preset)
                    .items(PRESETS.iter().map(|preset| preset.name.to_string()))
                    .selected(page.preset)
                    .style(PickerStyle::Field)
                    .open(app.workbench.open_menu == Some(MenuId::SinkTeamsimPreset))
                    .on_toggle(handler(&view, |this, _, cx| {
                        this.open_menu(MenuId::SinkTeamsimPreset, cx)
                    }))
                    .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
                    .on_pick(indexed(&view, |this, index, _, cx| {
                        this.load_teamsim_preset(index, cx)
                    })),
            ),
        )
        .child(
            div().flex_none().w(px(ALGO_WIDTH)).child(
                // The list is `Algo::ALL`, and a row carries the arrangement's hint as well as its
                // name — exactly as the Teams toolbar does, so a variant added there appears here
                // with no edit.
                Picker::new("teamsim-algo", page.sim.algo.label())
                    .icon(IconName::LayoutDashboard)
                    .items(Algo::ALL.map(|a| format!("{} \u{2014} {}", a.label(), a.hint())))
                    .selected(
                        Algo::ALL
                            .iter()
                            .position(|&a| a == page.sim.algo)
                            .unwrap_or(0),
                    )
                    .style(PickerStyle::Field)
                    .open(app.workbench.open_menu == Some(MenuId::SinkTeamsimAlgo))
                    .on_toggle(handler(&view, |this, _, cx| {
                        this.open_menu(MenuId::SinkTeamsimAlgo, cx)
                    }))
                    .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
                    .on_pick(indexed(&view, |this, index, _, cx| {
                        this.set_teamsim_algo(index, cx)
                    })),
            ),
        )
        .child(primary_button(
            "teamsim-tidy",
            None,
            "Tidy",
            click(cx, |this, cx| this.tidy_teamsim(cx)),
        ))
        .child(ghost_button(
            "teamsim-add-task",
            None,
            "Add task",
            click(cx, |this, cx| this.add_teamsim_task(cx)),
        ))
        .child(ghost_button(
            "teamsim-add-agent",
            None,
            "Add agent",
            click(cx, |this, cx| this.add_teamsim_agent(cx)),
        ))
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "teamsim-copy",
            None,
            "Copy JSON",
            click(cx, |this, cx| this.copy_teamsim_json(cx)),
        ))
        .child(ghost_button(
            "teamsim-paste",
            None,
            "Paste JSON",
            click(cx, |this, cx| this.paste_teamsim_json(cx)),
        ))
        .child(stepper(
            "teamsim-zoom",
            format!("{}%", page.zoom_pct()),
            click(cx, |this, cx| this.zoom_teamsim(-ZOOM_STEP, cx)),
            click(cx, |this, cx| this.zoom_teamsim(ZOOM_STEP, cx)),
        ))
        .child(icon_button(
            "teamsim-fit",
            IconName::Maximize,
            false,
            click(cx, |this, cx| this.reset_teamsim_zoom(cx)),
        ))
        .into_any_element()
}

/// The numbers under the toolbar: how big the arrangement came out, its aspect, how far past one
/// screen it runs in each axis, and what the last add did to everything that was already placed.
///
/// The arithmetic is `state::teamsim`'s — this reads it out.
fn strip(app: &AppState, drawing: &Drawing, cx: &mut Context<AppState>) -> AnyElement {
    let page = &app.sink.teamsim;
    let sim = &page.sim;
    let metrics = sim.metrics(drawing.bbox);
    let viewport = sim.scenario.viewport;

    // Past one screen in either axis is what makes an arrangement unreadable, so the ratios say it
    // in colour as well as in figures.
    let over = |ratio: f32| {
        if ratio > 1.5 {
            theme::danger()
        } else if ratio > 1.0 {
            theme::warning()
        } else {
            theme::success()
        }
    };

    let mut row = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px_2()
        .py_1()
        .bg(theme::surface())
        .border_b_1()
        .border_color(theme::border())
        .child(figure(
            "bbox",
            format!("{:.0}\u{d7}{:.0}", metrics.w, metrics.h),
            theme::text(),
        ))
        .child(figure(
            "aspect",
            format!("{:.2}", metrics.aspect),
            theme::text_muted(),
        ))
        .child(figure(
            "screens x",
            format!("{:.2}", metrics.screens_x),
            over(metrics.screens_x),
        ))
        .child(figure(
            "screens y",
            format!("{:.2}", metrics.screens_y),
            over(metrics.screens_y),
        ))
        .child(figure(
            "viewport",
            format!("{:.0}\u{d7}{:.0}", viewport.w, viewport.h),
            theme::text_faint(),
        ))
        // Zero is what an arrival is aiming for: a block that arrives should find a place among what
        // is on screen, not shuffle it.
        .child(figure(
            "drift",
            if sim.drift.moved == 0 {
                "0".to_string()
            } else {
                format!("{} moved, worst {:.0}", sim.drift.moved, sim.drift.worst)
            },
            if sim.drift.moved == 0 {
                theme::success()
            } else {
                theme::warning()
            },
        ))
        .child(figure(
            "blocks",
            format!(
                "{} cards \u{b7} {} delegates \u{b7} {} links",
                drawing.cards.len(),
                drawing.subs.len(),
                sim.scenario.links.len()
            ),
            theme::text_faint(),
        ))
        .child(div().flex_1().min_w(px(0.)));

    if let Some(from) = page.arming.clone() {
        row = row.child(
            mono(
                format!("linking from {from} \u{2014} click a card"),
                theme::accent(),
            )
            .text_size(theme::font(Family::Chrome, Role::Meta)),
        );
    }
    row = row.child(
        mono(sim.scenario.note.clone(), theme::text_faint())
            .text_size(theme::font(Family::Chrome, Role::Meta)),
    );
    // Every extra edge, removable: the canvas draws them and this is where one is taken back off,
    // because a line on a canvas is not something a click can pick out.
    row = row.children(sim.scenario.links.iter().enumerate().map(|(ix, link)| {
        ghost_button(
            eid("teamsim-unlink", ix),
            None,
            format!(
                "{}\u{2192}{} {} \u{d7}",
                link.from,
                link.to,
                link.kind.label()
            ),
            click(cx, move |this, cx| this.unlink_teamsim(ix, cx)),
        )
        .into_any_element()
    }));

    row.into_any_element()
}

/// One figure on the strip: what it measures, quietly, and the number, in the tone it earns.
fn figure(label: &str, value: String, colour: gpui::Rgba) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(
            mono(label.to_string(), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Micro)),
        )
        .child(mono(value, colour).text_size(theme::font(Family::Chrome, Role::Meta)))
        .into_any_element()
}

/// What the last paste said, where it failed. Above the canvas, so the scenario being looked at is
/// still the one on screen.
fn problem(reason: String) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .px_2()
        .py_1()
        .bg(theme::danger_soft())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::danger())
        .child(mono(reason, theme::text()).text_size(theme::font(Family::Chrome, Role::Meta)))
        .into_any_element()
}

/// The canvas: the ground, the two levels of fence, the connectors, and the blocks — every one of
/// them through `ui::kit::blocks`, in the order it stacks them.
fn canvas(
    app: &AppState,
    drawing: &Drawing,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let page = &app.sink.teamsim;
    let sim = &page.sim;
    let zoom = page.zoom;
    let view = cx.entity();
    let held = page.carry.as_ref().map(|carry| carry.held.clone());
    let mut board = Board::new(zoom, MARGIN);

    // The session outlines, outermost.
    for group in &drawing.sessions {
        let Some(session) = sim.scenario.sessions.get(group.ix) else {
            continue;
        };
        let mut label = vec![Word::new(
            session.name.clone(),
            theme::text_muted(),
            Role::Meta,
        )];
        if !session.branch.is_empty() {
            label.push(Word::new(
                session.branch.clone(),
                theme::text_faint(),
                Role::Micro,
            ));
        }
        board.fence(
            Fence::new(group.rect, theme::fade(theme::border(), 0.9), false).labelled(
                label,
                (SESSION_PAD * 0.5, 4.0),
                SESSION_LABEL,
            ),
        );
    }

    // The task containers, and the drag that moves one with everything in it.
    for group in &drawing.tasks {
        let Some(task) = sim.scenario.tasks.get(group.ix) else {
            continue;
        };
        let id = task.id.clone();
        let carried = held == Some(TeamsimHeld::Task(id.clone()));
        let mut label = Vec::new();
        if let Some(shape) = task.shape {
            label.push(Word::new(shape.label(), theme::text_faint(), Role::Micro));
            label.push(Word::new("\u{b7}", theme::text_faint(), Role::Micro));
        }
        label.push(Word::new(
            task.title.clone(),
            theme::text_muted(),
            Role::Meta,
        ));

        let grabbed = id.clone();
        let carry_view = view.clone();
        board.fence(
            Fence::new(
                group.rect,
                if carried {
                    theme::accent()
                } else {
                    theme::border()
                },
                carried,
            )
            // The same inset and the same label room the Teams canvas gives a container, because a
            // bench drawn to different measurements is not a bench.
            .labelled(label, (GROUP_PAD * 0.5, 4.0), GROUP_LABEL)
            .handle(
                blocks::handle(eid("teamsim-task", &id), group.rect, zoom)
                    .on_drag(
                        Carried(TeamsimHeld::Task(grabbed.clone())),
                        move |carried, grab, _, cx: &mut App| {
                            let grab = (f32::from(grab.x), f32::from(grab.y));
                            let held = carried.0.clone();
                            carry_view
                                .update(cx, |this, cx| this.start_teamsim_carry(held, grab, cx));
                            cx.new(|_| Empty)
                        },
                    )
                    .into_any_element(),
            ),
        );
    }

    for edge in &drawing.edges {
        board.link(Link {
            from: point(edge.from.0 * zoom, edge.from.1 * zoom),
            to: point(edge.to.0 * zoom, edge.to.1 * zoom),
            colour: edge_colour(edge.kind),
        });
    }

    // A card's delegate ring, over the connectors and under the blocks.
    for group in &drawing.rings {
        board.inner_fence(Fence::new(
            group.rect,
            theme::fade(theme::accent_muted(), 0.8),
            false,
        ));
    }

    for card in &drawing.cards {
        let Some(row) = sim.scenario.agents.get(card.agent) else {
            continue;
        };
        board.block(
            card.rect,
            agent_block(
                sim,
                card.agent,
                card.rect,
                page.selected.as_deref() == Some(row.id.as_str()),
                page.arming.as_deref() == Some(row.id.as_str()),
                held == Some(TeamsimHeld::Agent(row.id.clone())),
                zoom,
                &view,
                cx,
            ),
        );
    }

    for sub in &drawing.subs {
        let Some(row) = sim.scenario.agents.get(sub.agent) else {
            continue;
        };
        let Some(delegate) = row.subagents.get(sub.sub) else {
            continue;
        };
        board.block(
            sub.rect,
            sub_block(
                &row.id,
                &delegate.id,
                delegate.name.clone(),
                sub.rect,
                held == Some(TeamsimHeld::Sub {
                    agent: row.id.clone(),
                    sub: delegate.id.clone(),
                }),
                zoom,
                &view,
                cx,
            ),
        );
    }

    // The whole canvas is the drop target, and where something landed is worked out from where it is
    // rather than from what it was dropped on.
    let content = board
        .content(window.viewport_size())
        .on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<Carried>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let Some(carry) = this.sink.teamsim.carry.clone() else {
                    return;
                };
                let zoom = this.sink.teamsim.zoom;
                let local = event.event.position - event.bounds.origin;
                // The grab point is where inside the block the pointer went down, so taking it off
                // gives the top-left of whatever is held.
                let at = (
                    ((f32::from(local.x) - carry.grab.0) / zoom).max(0.0),
                    ((f32::from(local.y) - carry.grab.1) / zoom).max(0.0),
                );
                this.move_teamsim_carry(at, cx);
            }),
        )
        .on_drop(cx.listener(|this, _: &Carried, _, cx| this.end_teamsim_carry(cx)));

    blocks::scroller("teamsim-canvas", &app.teamsim_scroll)
        .child(content)
        .into_any_element()
}

/// Which colour a connector reads in: what the relation is, not what either end is doing.
fn edge_colour(kind: EdgeKind) -> gpui::Rgba {
    match kind {
        EdgeKind::Parent => theme::fade(theme::accent_muted(), 0.6),
        EdgeKind::Delegate => theme::fade(theme::accent_muted(), 0.45),
        EdgeKind::Extra(LinkKind::Spawn) => theme::fade(theme::info(), 0.55),
        EdgeKind::Extra(LinkKind::Handoff) => theme::fade(theme::accent(), 0.6),
        EdgeKind::Extra(LinkKind::Watch) => theme::fade(theme::warning(), 0.55),
    }
}

/// One card: what the scenario says it is, and the three things a click on it can do — select it,
/// give it a delegate, or arm a link out of it.
#[allow(clippy::too_many_arguments)]
fn agent_block(
    sim: &Sim,
    ix: usize,
    rect: blocks::Rect,
    selected: bool,
    armed: bool,
    carried: bool,
    zoom: f32,
    view: &Entity<AppState>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(row) = sim.scenario.agents.get(ix) else {
        return div().into_any_element();
    };
    let id = row.id.clone();
    // The armed card takes the accent for its edge, because a mode with no mark on screen is a mode
    // nobody can leave.
    let colour = if armed {
        theme::accent()
    } else {
        activity_colour(row.activity.activity())
    };
    let carry_view = view.clone();

    blocks::block(
        eid("teamsim-card", &id),
        rect,
        Look::new(colour)
            .selected(selected || armed)
            .carried(carried),
        zoom,
    )
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0 * zoom))
            .child(role_mark(&row.role, colour, 22.0 * zoom))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .text_size(theme::font(Family::Chrome, Role::Body) * zoom)
                            .text_color(theme::text())
                            .child(SharedString::from(row.name.clone())),
                    )
                    .child(
                        mono(row.role.to_uppercase(), theme::text_faint())
                            .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
                    ),
            )
            .child(
                mono(row.id.clone(), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
            ),
    )
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0 * zoom))
            .children((!row.harness.is_empty()).then(|| {
                Icon::new(harness_icon(&row.harness))
                    .with_size(px(12.0 * zoom))
                    .flex_none()
                    .text_color(theme::text_muted())
            }))
            .child(
                mono(row.harness.clone(), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom)
                    .flex_1()
                    .min_w(px(0.))
                    .truncate(),
            )
            .child(
                mono(row.activity.word(), colour)
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
            ),
    )
    .child(div().flex_1().min_h(px(0.)))
    // The card's own controls. They stop the click reaching the card, so `+sub` does not also
    // select it and `link` does not also finish a link into it.
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0 * zoom))
            .child(pip(
                eid("teamsim-add-sub", &id),
                "+sub",
                "Give this card a delegate",
                false,
                zoom,
                {
                    let id = id.clone();
                    cx.listener(move |this, _, _, cx| this.add_teamsim_subagent(id.clone(), cx))
                },
            ))
            .child(pip(
                eid("teamsim-link", &id),
                "link",
                "Arm a link out of this card \u{2014} the next card clicked is the target",
                armed,
                zoom,
                {
                    let id = id.clone();
                    cx.listener(move |this, _, _, cx| this.arm_teamsim_link(id.clone(), cx))
                },
            ))
            .child(div().flex_1().min_w(px(0.)))
            .child(pip(
                eid("teamsim-drop-card", &id),
                "\u{d7}",
                "Remove this card",
                false,
                zoom,
                {
                    let id = id.clone();
                    cx.listener(move |this, _, _, cx| this.remove_teamsim_agent(id.clone(), cx))
                },
            )),
    )
    .on_click({
        let id = id.clone();
        cx.listener(move |this, _, _, cx| this.select_teamsim_card(id.clone(), cx))
    })
    .on_drag(
        Carried(TeamsimHeld::Agent(id)),
        move |carried, grab, _, cx: &mut App| {
            let grab = (f32::from(grab.x), f32::from(grab.y));
            let held = carried.0.clone();
            carry_view.update(cx, |this, cx| this.start_teamsim_carry(held, grab, cx));
            cx.new(|_| Empty)
        },
    )
    .into_any_element()
}

/// One delegate: its name, and the `\u{d7}` that takes it off again.
#[allow(clippy::too_many_arguments)]
fn sub_block(
    agent: &str,
    sub: &str,
    name: String,
    rect: blocks::Rect,
    carried: bool,
    zoom: f32,
    view: &Entity<AppState>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let held = TeamsimHeld::Sub {
        agent: agent.to_string(),
        sub: sub.to_string(),
    };
    let carry_view = view.clone();
    let (owner, delegate) = (agent.to_string(), sub.to_string());

    blocks::block(
        eid2("teamsim-sub", agent, sub),
        rect,
        Look::new(theme::accent_muted())
            .carried(carried)
            .opaque(true),
        zoom,
    )
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0 * zoom))
            .child(
                Icon::new(UbiqIcon::HarnessAny)
                    .with_size(Size::XSmall)
                    .text_color(theme::text_faint()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(theme::font(Family::Chrome, Role::Body) * zoom)
                    .text_color(theme::text())
                    .truncate()
                    .child(SharedString::from(name)),
            )
            .child(pip(
                eid2("teamsim-drop-sub", agent, sub),
                "\u{d7}",
                "Remove this delegate",
                false,
                zoom,
                cx.listener(move |this, _, _, cx| {
                    this.remove_teamsim_subagent(owner.clone(), delegate.clone(), cx)
                }),
            )),
    )
    .child(
        mono(format!("{agent}/{sub}"), theme::text_faint())
            .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
    )
    .on_drag(Carried(held), move |carried, grab, _, cx: &mut App| {
        let grab = (f32::from(grab.x), f32::from(grab.y));
        let held = carried.0.clone();
        carry_view.update(cx, |this, cx| this.start_teamsim_carry(held, grab, cx));
        cx.new(|_| Empty)
    })
    .into_any_element()
}

/// A control small enough to sit on a card, and no smaller: it carries a word rather than only a
/// glyph, and a tooltip either way, because nothing on this canvas may mean something by its shape
/// alone.
///
/// It stops the click propagating, so a control on a card is not also a click on the card.
fn pip(
    id: gpui::ElementId,
    label: &'static str,
    tip: &'static str,
    active: bool,
    zoom: f32,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .px(px(5.0 * zoom))
        .h(px(18.0 * zoom))
        .bg(if active {
            theme::accent_soft()
        } else {
            theme::surface()
        })
        .border_l(px(theme::accent_edge()))
        .border_color(if active {
            theme::accent()
        } else {
            theme::border()
        })
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            mono(
                label,
                if active {
                    theme::accent()
                } else {
                    theme::text_muted()
                },
            )
            .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
        )
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            on_click(event, window, cx);
        })
        .into_any_element()
}

/// A click handler over the window, in the shape `kit`'s buttons take.
fn click(
    cx: &Context<AppState>,
    f: impl Fn(&mut AppState, &mut Context<AppState>) + 'static,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    let view = cx.entity();
    move |_, _, cx| {
        view.update(cx, |this, cx| f(this, cx));
    }
}
