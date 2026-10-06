//! The static painter (P2.6): walks a [`Scene`] under the base viewport surface and draws it.
//!
//! The camera is the base's (D14): [`surface`] gives fit, wheel zoom about the pointer, drag pan,
//! pinch and double-click refit, keyed `archify.<tab key>`. The painter only maps viewBox units
//! (vu) onto the panel with `AppState::viewport(key).camera(..)`, so a document starts fitted
//! ("fit on first show" is the camera's `zoom: None`). The scene is already z-ordered; nothing is
//! sorted here.
//!
//! The reading tier is `zoom / fit` (D12): `Detail` decides which texts show. Colours are resolved
//! per frame through `theme.rs`; nothing here names one.
//!
//! Motion (P7.3, `animate.rs`) arrives as an [`Overlay`] sampled for the frame, never as a restyle:
//! its group multipliers are the only dim there is (focus included), its strokes sit above the edges
//! and under the nodes (`Layer::Overlay`), its highlights above the nodes, and its detail fade
//! replaces the tier's all-or-nothing text visibility while a fade runs. A glow has no blur here: it
//! is a wider, fainter copy of the stroke or ring ([`GLOW_ALPHA`]).
//!
//! Strokes and dashes are vu and scale with the camera, with a hairline floor on strokes so a
//! scaled-down diagram keeps its outlines. A halo is its edge's `halo_width()` in vu, also scaled.
//! Sigil strokes are `non_scaling` screen px. Round caps and joins need lyon's `LineCap`, which GPUI
//! does not re-export, so halos and edges have butt caps and miter joins.

use std::cell::Cell;
use std::sync::Arc;

use ubiq_archify::motion::{DetailAlpha, Highlight, Overlay, OverlayStroke};
use ubiq_archify::scene::{
    Anchor, Cmd, Detail, GroupId, GroupKind, Layer, PathShape, PolylineShape, RectShape, Scene,
    Shape, Stroke, TextShape, Tier,
};
use ubiq_archify::tokens::{self, Mode, Preset, Token};
use gpui::{
    AnyElement, App, Bounds, Context, ElementId, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, PathBuilder, Pixels, Point, Rgba, Styled, TextAlign, TextRun, Window, actions,
    canvas, div, fill, point, px, size,
};
use crate::app::AppState;
use crate::state::viewport::{Camera, Content, Viewport};
use crate::theme as ubiq_theme;
use crate::ui::kit::{choice_pill, mono, pill, popover};
use crate::ui::viewer::viewport::surface;

use crate::state::archify::animate::{self, RouteKey};
use crate::state::archify::focus::{self, Click, Lit, Reach};
use crate::state::archify::{Hover, ui};
use crate::ui::archify::{finder, lens, theme};

/// The key context of the picture: the keys below act only while it holds focus, so typing into
/// the source buffer beside it is never taken (a plain `-` or `0` would be).
pub const KEY_CONTEXT: &str = "Archify";

actions!(
    archify,
    [
        Fit,
        ZoomIn,
        ZoomOut,
        ClearFocus,
        ToggleUpstream,
        ToggleDownstream,
        FindNode,
        CycleLens,
        RouteBegin,
        RouteNext,
        RoutePrev,
        RoutePlay
    ]
);

/// The picture's keys, bound by `install_key_bindings`; none of these is a base chord (D118), and
/// all of them are scoped to [`KEY_CONTEXT`].
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("0", Fit, Some(KEY_CONTEXT)),
        KeyBinding::new("=", ZoomIn, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-=", ZoomIn, Some(KEY_CONTEXT)),
        KeyBinding::new("-", ZoomOut, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", ClearFocus, Some(KEY_CONTEXT)),
        KeyBinding::new("u", ToggleUpstream, Some(KEY_CONTEXT)),
        KeyBinding::new("d", ToggleDownstream, Some(KEY_CONTEXT)),
        KeyBinding::new("/", FindNode, Some(KEY_CONTEXT)),
        KeyBinding::new("f", FindNode, Some(KEY_CONTEXT)),
        KeyBinding::new("l", CycleLens, Some(KEY_CONTEXT)),
        KeyBinding::new("r", RouteBegin, Some(KEY_CONTEXT)),
        KeyBinding::new("n", RouteNext, Some(KEY_CONTEXT)),
        KeyBinding::new("p", RoutePrev, Some(KEY_CONTEXT)),
        KeyBinding::new("space", RoutePlay, Some(KEY_CONTEXT)),
    ]
}

/// A pointer press that moves further than this (px) before release was a pan, not a click.
const CLICK_SLOP_PX: f32 = 4.0;
/// How far from a shape (px) the pointer still hits it.
const HIT_SLOP_PX: f64 = 4.0;
/// One key press of zoom.
const ZOOM_STEP: f32 = 1.25;
/// The width (px) of the focus glow and the reach outlines.
const GLOW_PX: f32 = 2.0;

/// Below this a glyph is noise and is not drawn (the base scene painter's floor).
const MIN_TEXT_PX: f32 = 3.0;
/// A stroke never thins below this, or a scaled-down diagram loses its outlines.
const HAIRLINE: f32 = 0.75;
/// A glow's opacity against its stroke's: the CSS `drop-shadow` as a wider, fainter copy.
const GLOW_ALPHA: f64 = 0.22;
/// A label plate is dropped once its text's detail factor falls under this, not faded with it: a
/// half-faded plate reads as an empty rectangle over the chart.
const PLATE_CUTOFF: f64 = 0.4;

/// A tab's scene and the hash of the text it came from.
// --------------------------------------------------------------------------------------------- //
// The element
// --------------------------------------------------------------------------------------------- //

/// `scene` on the base viewport surface, keyed `key` (`archify.<tab key>`), with the frame's
/// motion `overlay` (`animate::advance`).
pub fn preview(
    app: &AppState,
    key: &str,
    scene: Arc<Scene>,
    motion: Arc<Overlay>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let content = content_of(&scene);
    let camera_at = app.viewport(key);
    theme::begin_frame();
    let mode = theme::chart_mode();
    let ground = theme::resolve(Token::Bg, mode);
    let state = ui(cx).peek_view(key);
    let handle = picture_handle(key, cx);
    let lit = state
        .focus
        .as_deref()
        .and_then(|node| focus::lit(&scene, node, state.reach));
    // Texts of the focused and the hovered node show whatever the tier (`00` §5.9).
    let revealed: Vec<GroupId> = lit
        .iter()
        .map(|l| l.focus)
        .chain(state.hover.map(|h| h.group))
        .collect();

    let painted = scene.clone();
    let painted_lit = lit.clone();
    let picture = canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            if w < 8.0 || h < 8.0 {
                return; // not measured yet; the surface asks for another frame once it is
            }
            let camera = camera_at.camera(content, w, h);
            let fit = Viewport::fit_scale(content, w, h);
            let frame = Frame {
                panel: bounds,
                camera,
                tier: tier_of(camera.scale, fit),
                fade: motion.detail_alpha,
                mode,
                alpha: Cell::new(1.0),
                reveal: Cell::new(false),
            };
            grid(painted.preset, &frame, window);
            paint_scene(
                &painted,
                painted_lit.as_ref(),
                &revealed,
                &motion,
                &frame,
                window,
                cx,
            );
        },
    )
    .absolute()
    .inset_0()
    .size_full();

    let layer = overlay(key, &scene, &state, &handle, cx);
    let body = surface(app, key, ground, content, picture, layer, cx);

    let k = key.to_string();
    div()
        .id(ElementId::Name(format!("archify-picture-{key}").into()))
        .key_context(KEY_CONTEXT)
        .track_focus(&handle)
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &Fit, _, cx| this.reset_viewport(&k, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &ZoomIn, _, cx| zoom_by(this, &k, ZOOM_STEP, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &ZoomOut, _, cx| zoom_by(this, &k, 1.0 / ZOOM_STEP, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |_, _: &ClearFocus, _, cx| {
                // A relationship, a lens or a route ends first; then the focus; then Escape is
                // the window's.
                if animate::escape(&k, cx) {
                    return;
                }
                if ui(cx).view(&k).focus.take().is_some() {
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &FindNode, window, cx| finder::open(this, &k, window, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &CycleLens, _, cx| animate::cycle_lens(this, &k, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &RouteBegin, _, cx| animate::begin_route(this, &k, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &RouteNext, _, cx| animate::route_key(this, &k, RouteKey::Step, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &RoutePrev, _, cx| animate::route_key(this, &k, RouteKey::Back, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |this, _: &RoutePlay, _, cx| animate::route_key(this, &k, RouteKey::Play, cx)
        }))
        .on_action(cx.listener({
            let k = k.clone();
            move |_, _: &ToggleUpstream, _, cx| {
                let v = ui(cx).view(&k);
                v.reach = v.reach.toggled(Reach::Up);
                cx.notify();
            }
        }))
        .on_action(cx.listener(move |_, _: &ToggleDownstream, _, cx| {
            let v = ui(cx).view(&k);
            v.reach = v.reach.toggled(Reach::Down);
            cx.notify();
        }))
        .child(body)
        .into_any_element()
}

/// The picture's rectangle for the camera: the scene's viewBox.
fn content_of(scene: &Scene) -> Content {
    Content::from_size(scene.view_box[0] as f32, scene.view_box[1] as f32)
}

/// The reading tier (D12): the camera's scale relative to the fit scale. At the fit it is `Read`,
/// so tags are hidden by default; `Map` is below the fit and `Full` from 1.75 times it.
pub(crate) fn tier_of(scale: f32, fit: f32) -> Tier {
    Tier::from_scale(f64::from(scale / fit))
}

/// The handle that takes the keys of the picture `key`, made on first use.
fn picture_handle(key: &str, cx: &mut Context<AppState>) -> FocusHandle {
    if let Some(handle) = ui(cx).picture_focus.get(key) {
        return handle.clone();
    }
    let handle = cx.focus_handle();
    ui(cx).picture_focus.insert(key.to_string(), handle.clone());
    handle
}

/// Zoom by `factor` about the middle of the panel (the keys have no pointer to zoom about).
fn zoom_by(this: &mut AppState, key: &str, factor: f32, cx: &mut Context<AppState>) {
    let vp = this.viewport(key);
    if !vp.measured() {
        return;
    }
    let centre = point(
        px(vp.origin_x + vp.panel_w / 2.0),
        px(vp.origin_y + vp.panel_h / 2.0),
    );
    this.zoom_viewport(key, factor, centre, cx);
}

// --------------------------------------------------------------------------------------------- //
// The pointer
// --------------------------------------------------------------------------------------------- //

/// The viewBox point under a window position, and the zoom there (px per vu); `None` before the
/// panel is measured.
fn vu_at(vp: &Viewport, content: Content, at: Point<Pixels>) -> Option<([f64; 2], f32)> {
    if !vp.measured() {
        return None;
    }
    let camera = vp.camera(content, vp.panel_w, vp.panel_h);
    let x = (f32::from(at.x) - vp.origin_x - camera.offset_x) / camera.scale;
    let y = (f32::from(at.y) - vp.origin_y - camera.offset_y) / camera.scale;
    Some(([f64::from(x), f64::from(y)], camera.scale))
}

/// The group a tooltip is for at `p`: the topmost node, edge or edge label. Frames and the legend
/// have none.
fn tip_target(scene: &Scene, p: [f64; 2], slop: f64) -> Option<GroupId> {
    let id = scene.hit_test(p, slop)?;
    matches!(
        scene.group(id)?.kind,
        GroupKind::Node | GroupKind::Edge | GroupKind::Label
    )
    .then_some(id)
}

/// The node a click at `p` focuses: only a node does; anything else, empty space included, clears.
fn click_target(scene: &Scene, p: [f64; 2], slop: f64) -> Option<String> {
    let id = scene.hit_test(p, slop)?;
    let g = scene.group(id)?;
    (g.kind == GroupKind::Node)
        .then(|| g.node_id.clone())
        .flatten()
}

/// The layer over the picture, above the viewport's pan and zoom layer: hover tooltip, click to
/// focus, and the reach control. It consumes no press, so a drag still pans (`D14`).
fn overlay(
    key: &str,
    scene: &Arc<Scene>,
    state: &crate::state::archify::Interaction,
    handle: &FocusHandle,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let content = content_of(scene);
    let (down_key, up_key, move_key) = (key.to_string(), key.to_string(), key.to_string());
    let (up_scene, move_scene) = (scene.clone(), scene.clone());
    let handle = handle.clone();

    let mut layer = div()
        .absolute()
        .inset_0()
        .size_full()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, e: &MouseDownEvent, window, cx| {
                window.focus(&handle, cx);
                ui(cx).view(&down_key).down =
                    Some([f32::from(e.position.x), f32::from(e.position.y)]);
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseUpEvent, _, cx| {
                let Some(down) = ui(cx).view(&up_key).down.take() else {
                    return;
                };
                let moved =
                    (f32::from(e.position.x) - down[0]).hypot(f32::from(e.position.y) - down[1]);
                if moved > CLICK_SLOP_PX {
                    return;
                }
                let Some((p, zoom)) = vu_at(&this.viewport(&up_key), content, e.position) else {
                    return;
                };
                let target = click_target(&up_scene, p, HIT_SLOP_PX / f64::from(zoom));
                let focus = ui(cx).peek_view(&up_key).focus;
                let route = ui(cx).motions.route_active(&up_key);
                match focus::click_plan(target, e.modifiers.shift, focus.as_deref(), route) {
                    Click::Route(node) => {
                        animate::route_key(this, &up_key, RouteKey::Target(node), cx)
                    }
                    Click::Relate { from, to } => animate::relate(this, &up_key, &from, &to, cx),
                    Click::Nothing => {}
                    Click::Focus(target) => {
                        animate::click_ends_interaction(&up_key, cx);
                        let view = ui(cx).view(&up_key);
                        if view.focus != target {
                            view.focus = target;
                        }
                        cx.notify();
                    }
                }
            }),
        )
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            let vp = this.viewport(&move_key);
            let group = if e.dragging() {
                None
            } else {
                vu_at(&vp, content, e.position)
                    .and_then(|(p, zoom)| tip_target(&move_scene, p, HIT_SLOP_PX / f64::from(zoom)))
            };
            let view = ui(cx).view(&move_key);
            if view.hover.map(|h| h.group) == group {
                return;
            }
            view.hover = group.map(|group| Hover {
                group,
                x: f32::from(e.position.x) - vp.origin_x,
                y: f32::from(e.position.y) - vp.origin_y,
            });
            cx.notify();
        }));

    if let Some(hover) = state.hover
        && let Some(text) = scene.tooltip(hover.group)
    {
        layer = layer.child(
            // Zero-size anchor at the pointer: the popover hangs above its asker's top-left.
            div()
                .absolute()
                .left(px(hover.x + 12.0))
                .top(px(hover.y - 6.0))
                .child(popover(
                    "archify-tooltip".into(),
                    px(0.),
                    None,
                    None,
                    [mono(text, ubiq_theme::text()).into_any_element()],
                )),
        );
    }
    layer
        .child(reach_control(key, state.reach))
        .child(controls_row(key, cx))
        .into_any_element()
}

/// The picture's top-left: Find and the lens pill, and while a relationship, a lens or a route is
/// on, the chip that says what it is, with the route's Prev, Play and Next. Esc ends the chip's
/// interaction.
fn controls_row(key: &str, cx: &mut Context<AppState>) -> AnyElement {
    let hud = ui(cx).motions.hud(key);
    let lens_on = ui(cx).motions.lens_label(key);
    let entity = cx.entity();
    let find_key = key.to_string();
    let find_entity = entity.clone();
    let mut row = div()
        .absolute()
        .top_0()
        .left_0()
        .flex()
        .items_center()
        // The controls take their own presses; they must not focus or clear behind them.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(choice_pill(
            ElementId::Name("archify-find".into()),
            "Find /",
            false,
            move |_, window, cx| {
                find_entity.update(cx, |this, cx| finder::open(this, &find_key, window, cx));
            },
        ))
        .child(lens::pill(key, lens_on, cx));
    if let Some(hud) = hud {
        row = row.child(pill(ubiq_theme::accent()).child(mono(hud.text, ubiq_theme::text())));
        if hud.route {
            let play = if hud.playing { "Pause" } else { "Play" };
            for (id, label, act) in [
                ("archify-route-prev", "Prev", RouteKey::Back),
                ("archify-route-play", play, RouteKey::Play),
                ("archify-route-next", "Next", RouteKey::Step),
            ] {
                let (entity, key) = (entity.clone(), key.to_string());
                row = row.child(choice_pill(
                    ElementId::Name(id.into()),
                    label,
                    false,
                    move |_, _, cx| {
                        let act = act.clone();
                        entity.update(cx, |this, cx| animate::route_key(this, &key, act, cx));
                    },
                ));
            }
        }
    }
    row.into_any_element()
}

/// Near, Upstream, Downstream: a small segmented control at the picture's top-right.
fn reach_control(key: &str, current: Reach) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .right_0()
        .flex()
        // The control takes its own presses; they must not focus or clear behind it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(Reach::ALL.map(|reach| {
            let key = key.to_string();
            choice_pill(
                ElementId::Name(format!("archify-reach-{}", reach.label()).into()),
                reach.label(),
                reach == current,
                move |_, _window, cx| {
                    ui(cx).view(&key).reach = reach;
                    cx.refresh_windows();
                },
            )
        }))
        .into_any_element()
}

// --------------------------------------------------------------------------------------------- //
// The transform
// --------------------------------------------------------------------------------------------- //

/// vu to window pixels for one frame, and what the frame needs to colour and cull.
struct Frame {
    panel: Bounds<Pixels>,
    camera: Camera,
    tier: Tier,
    /// The detail visibility while a tier fade runs; `None` outside one (the tier decides).
    fade: Option<DetailAlpha>,
    mode: Mode,
    /// The opacity of the item being painted: its group's multiplier in the motion overlay.
    alpha: Cell<f64>,
    /// Whether the item being painted shows its text whatever the tier.
    reveal: Cell<bool>,
}

impl Frame {
    fn at(&self, p: [f64; 2]) -> Point<Pixels> {
        self.panel.origin
            + point(
                px(self.camera.offset_x + p[0] as f32 * self.camera.scale),
                px(self.camera.offset_y + p[1] as f32 * self.camera.scale),
            )
    }

    /// A length in vu as window pixels.
    fn len(&self, vu: f64) -> f32 {
        vu as f32 * self.camera.scale
    }

    /// `token` at `alpha`, dimmed when the item being painted is outside the focus. The mask plate
    /// is the ground itself and is not dimmed: a dimmed node or halo must still hide what it
    /// covers, and faded it would show edges through it.
    fn colour(&self, token: Token, alpha: f64) -> Rgba {
        let dim = if token == Token::Mask {
            1.0
        } else {
            self.alpha.get()
        };
        ubiq_theme::fade(theme::resolve(token, self.mode), (alpha * dim) as f32)
    }

    /// The painted width of a stroke, in pixels.
    fn stroke_px(&self, stroke: &Stroke, non_scaling: bool) -> f32 {
        if non_scaling {
            stroke.width as f32
        } else {
            self.len(stroke.width).max(HAIRLINE)
        }
    }

    /// A stroke pen of `width` pixels with the stroke's dash pattern, scaled with the camera. Dashes
    /// never fall under a pixel: a zero-length dash would make the dasher spin.
    fn pen(&self, stroke: &Stroke, width: f32) -> PathBuilder {
        let pen = PathBuilder::stroke(px(width));
        if stroke.dash.is_empty() {
            return pen;
        }
        let dash: Vec<Pixels> = stroke
            .dash
            .iter()
            .map(|d| px(self.len(*d).max(1.0)))
            .collect();
        pen.dash_array(&dash)
    }

    /// Whether a window-space box (min, max) is clear of the panel.
    fn clear_of_panel(&self, min: Point<Pixels>, max: Point<Pixels>) -> bool {
        let p = self.panel;
        max.x < p.origin.x
            || max.y < p.origin.y
            || min.x > p.origin.x + p.size.width
            || min.y > p.origin.y + p.size.height
    }
}

// --------------------------------------------------------------------------------------------- //
// The walk
// --------------------------------------------------------------------------------------------- //

fn paint_scene(
    scene: &Scene,
    lit: Option<&Lit>,
    revealed: &[GroupId],
    motion: &Overlay,
    frame: &Frame,
    window: &mut Window,
    cx: &mut App,
) {
    // The motion layers are painted before the first item above them, or after the last item.
    let (mut strokes_due, mut marks_due) = (true, true);
    for (i, item) in scene.items.iter().enumerate() {
        if strokes_due && item.layer > Layer::Overlay {
            strokes_due = false;
            overlay_strokes(motion, frame, window);
        }
        if marks_due && item.layer > Layer::Nodes {
            marks_due = false;
            overlay_highlights(motion, frame, window);
        }
        frame
            .alpha
            .set(item.group.map_or(1.0, |group| motion.alpha_of(group)));
        frame
            .reveal
            .set(item.group.is_some_and(|g| revealed.contains(&g)));
        // A label plate (a mask directly before its text, same group and layer) stays opaque while
        // its text is legible and is dropped once the text fades under `PLATE_CUTOFF`.
        if let Shape::Rect(r) = &item.shape
            && r.fill.is_some_and(|f| f.token == Token::Mask)
        {
            let next = scene.items.get(i + 1).filter(|n| n.layer == item.layer && n.group == item.group);
            if let Some(Shape::Text(t)) = next.map(|n| &n.shape)
                && detail_factor(t.detail, frame.tier, frame.fade, frame.reveal.get()) < PLATE_CUTOFF
            {
                continue;
            }
        }
        match &item.shape {
            Shape::Rect(r) => rect(r, frame, window),
            Shape::Polyline(p) => polyline(p, frame, window),
            Shape::Text(t) => text(t, frame, window, cx),
            // The preset's grid is painted by `grid` over the whole panel, not the baked tiles.
            Shape::Path(p) if is_grid(p) => {}
            Shape::Path(p) => sigil(p, frame, window),
        }
    }
    frame.alpha.set(1.0);
    if strokes_due {
        overlay_strokes(motion, frame, window);
    }
    if marks_due {
        overlay_highlights(motion, frame, window);
    }
    if let Some(lit) = lit {
        // The selected glow in the accent; what a reach walk found, outlined in a tone for its
        // direction (the base has no violet/green: info is upstream, success downstream).
        let reach = match lit.reach {
            Reach::Near => None,
            Reach::Up => Some(ubiq_theme::info()),
            Reach::Down => Some(ubiq_theme::success()),
        };
        if let Some(colour) = reach {
            for group in &lit.others {
                glow(scene, *group, colour, frame, window);
            }
        }
        glow(scene, lit.focus, ubiq_theme::accent(), frame, window);
    }
}

/// The baked background grid of `tokens::restyle`, which only covers the diagram's own bounds.
fn is_grid(p: &PathShape) -> bool {
    p.stroke.as_ref().is_some_and(|s| s.token == Token::Grid)
}

/// The preset's background grid over the whole panel at any pan and zoom: lines on every
/// `GRID_TILE` vu multiple that falls in the visible rect, so panning moves them with the picture
/// instead of resampling the mesh. Classic has none. A step under a few pixels doubles until it is
/// not, which keeps a far zoom-out cheap and readable.
fn grid(preset: Preset, frame: &Frame, window: &mut Window) {
    let Some(style) = tokens::frame_style(preset).grid else {
        return;
    };
    let (w, h) = (f32::from(frame.panel.size.width), f32::from(frame.panel.size.height));
    let scale = frame.camera.scale;
    if scale <= 0.0 || !scale.is_finite() {
        return;
    }
    let mut step = tokens::GRID_TILE as f32;
    while step * scale < 6.0 {
        step *= 2.0;
    }
    let stroke = Stroke { token: Token::Grid, width: tokens::GRID_STROKE, dash: style.dash.to_vec() };
    let colour = frame.colour(Token::Grid, style.opacity);
    let px_width = frame.stroke_px(&stroke, false);
    let mut pen = frame.pen(&stroke, px_width);
    // The visible rect in vu, then the first multiple of `step` at or after each edge.
    let (x0, x1) = (-frame.camera.offset_x / scale, (w - frame.camera.offset_x) / scale);
    let (y0, y1) = (-frame.camera.offset_y / scale, (h - frame.camera.offset_y) / scale);
    let mut x = (x0 / step).ceil() * step;
    while x <= x1 {
        pen.move_to(frame.at([f64::from(x), f64::from(y0)]));
        pen.line_to(frame.at([f64::from(x), f64::from(y1)]));
        x += step;
    }
    let mut y = (y0 / step).ceil() * step;
    while y <= y1 {
        pen.move_to(frame.at([f64::from(x0), f64::from(y)]));
        pen.line_to(frame.at([f64::from(x1), f64::from(y)]));
        y += step;
    }
    if let Ok(path) = pen.build() {
        window.paint_path(path, colour);
    }
}

/// An outline around a group's bounds, in screen px, on top of the picture.
fn glow(scene: &Scene, group: GroupId, colour: Rgba, frame: &Frame, window: &mut Window) {
    let Some(g) = scene.group(group) else {
        return;
    };
    let radius = scene
        .items_of(group)
        .find_map(|i| match &i.shape {
            Shape::Rect(r) if r.stroke.is_some() => Some(r.radius),
            _ => None,
        })
        .unwrap_or(0.0);
    let grow = GLOW_PX / 2.0 + 1.0;
    let top_left = frame.at([g.bounds.x, g.bounds.y]) - point(px(grow), px(grow));
    let extent = size(
        px(frame.len(g.bounds.width) + grow * 2.0),
        px(frame.len(g.bounds.height) + grow * 2.0),
    );
    window.paint_quad(
        fill(Bounds::new(top_left, extent), ubiq_theme::transparent())
            .corner_radii(px(if radius > 0.0 {
                frame.len(radius) + grow
            } else {
                0.0
            }))
            .border_widths(px(GLOW_PX))
            .border_color(colour),
    );
}

// --------------------------------------------------------------------------------------------- //
// The motion overlay
// --------------------------------------------------------------------------------------------- //

/// One pen of an overlay stroke: a width in pixels and an opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pen {
    width: f32,
    alpha: f64,
}

/// The pens of `stroke` under the group's multiplier `group_alpha`, glow first so the core lies on
/// it. A `non_scaling` stroke (a comet) is in screen px, any other in vu with the hairline floor;
/// a glow of `g` px is a copy `g` px wider at [`GLOW_ALPHA`] of the opacity.
fn stroke_pens(stroke: &OverlayStroke, scale: f32, group_alpha: f64) -> Vec<Pen> {
    let alpha = stroke.alpha * group_alpha;
    if alpha <= 0.0 {
        return Vec::new();
    }
    let width = if stroke.non_scaling {
        stroke.width as f32
    } else {
        (stroke.width as f32 * scale).max(HAIRLINE)
    };
    let mut pens = Vec::with_capacity(2);
    if stroke.glow > 0.0 {
        pens.push(Pen {
            width: width + stroke.glow as f32,
            alpha: alpha * GLOW_ALPHA,
        });
    }
    pens.push(Pen { width, alpha });
    pens
}

/// One ring of a highlight: the node's rect grown by `grow` px on every side, with a border of
/// `border` px drawn inwards from that edge.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ring {
    grow: f32,
    border: f32,
    alpha: f64,
}

/// The rings of `highlight`, glow first. The outline (`width` in vu, a hairline floor, none when 0)
/// is centred on the rect's edge as an SVG stroke is; the glow is a ring `glow` px wide just
/// outside it, at [`GLOW_ALPHA`] of the opacity.
fn highlight_rings(highlight: &Highlight, scale: f32, group_alpha: f64) -> Vec<Ring> {
    let alpha = highlight.alpha * group_alpha;
    if alpha <= 0.0 {
        return Vec::new();
    }
    let width = if highlight.width > 0.0 {
        (highlight.width as f32 * scale).max(HAIRLINE)
    } else {
        0.0
    };
    let mut rings = Vec::with_capacity(2);
    if highlight.glow > 0.0 {
        let glow = highlight.glow as f32;
        rings.push(Ring {
            grow: width / 2.0 + glow,
            border: glow,
            alpha: alpha * GLOW_ALPHA,
        });
    }
    if width > 0.0 {
        rings.push(Ring {
            grow: width / 2.0,
            border: width,
            alpha,
        });
    }
    rings
}

/// The overlay's strokes, above the edges and under the nodes (`Layer::Overlay`).
fn overlay_strokes(motion: &Overlay, frame: &Frame, window: &mut Window) {
    for stroke in &motion.strokes {
        let [first, rest @ ..] = stroke.points.as_slice() else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let colour = theme::resolve(stroke.token, frame.mode);
        for pen in stroke_pens(stroke, frame.camera.scale, motion.alpha_of(stroke.group)) {
            let mut path = PathBuilder::stroke(px(pen.width));
            path.move_to(frame.at(*first));
            for point in rest {
                path.line_to(frame.at(*point));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, ubiq_theme::fade(colour, pen.alpha as f32));
            }
        }
    }
}

/// The overlay's highlights: rounded outlines and glows over the nodes they belong to.
fn overlay_highlights(motion: &Overlay, frame: &Frame, window: &mut Window) {
    for highlight in &motion.highlights {
        let colour = theme::resolve(highlight.token, frame.mode);
        let rect = highlight.rect;
        let radius = frame.len(highlight.radius);
        for ring in highlight_rings(
            highlight,
            frame.camera.scale,
            motion.alpha_of(highlight.group),
        ) {
            let top_left = frame.at([rect.x, rect.y]) - point(px(ring.grow), px(ring.grow));
            let extent = size(
                px(frame.len(rect.width) + ring.grow * 2.0),
                px(frame.len(rect.height) + ring.grow * 2.0),
            );
            window.paint_quad(
                fill(Bounds::new(top_left, extent), ubiq_theme::transparent())
                    .corner_radii(px(if radius > 0.5 {
                        radius + ring.grow
                    } else {
                        0.0
                    }))
                    .border_widths(px(ring.border))
                    .border_color(ubiq_theme::fade(colour, ring.alpha as f32)),
            );
        }
    }
}

/// The opacity of a text of reading depth `detail`: full when the tier shows it or the text is
/// revealed (the focused and hovered node's), and while a tier fade runs the fade's value for its
/// depth (anchors never fade).
fn detail_factor(detail: Detail, tier: Tier, fade: Option<DetailAlpha>, revealed: bool) -> f64 {
    if revealed {
        return 1.0;
    }
    match (fade, detail) {
        (_, Detail::Anchor) => 1.0,
        (Some(f), Detail::Context) => f.context,
        (Some(f), Detail::Fine) => f.fine,
        (None, d) => f64::from(d.visible(tier)),
    }
}

/// A rect. A solid stroke is the quad's own border, grown by half its width so it is centred on the
/// rect's edge as an SVG stroke is. A dashed one is a path over a plain quad.
fn rect(r: &RectShape, frame: &Frame, window: &mut Window) {
    let top_left = frame.at([r.rect.x, r.rect.y]);
    let extent = size(px(frame.len(r.rect.width)), px(frame.len(r.rect.height)));
    if frame.clear_of_panel(top_left, top_left + point(extent.width, extent.height)) {
        return;
    }
    let radius = frame
        .len(r.radius)
        .min(f32::from(extent.width) / 2.0)
        .min(f32::from(extent.height) / 2.0);
    let background = r.fill.map(|f| frame.colour(f.token, f.alpha));

    let dashed = r.stroke.as_ref().filter(|s| !s.dash.is_empty());
    let border = r.stroke.as_ref().filter(|s| s.dash.is_empty());

    let (bounds, corner, width) = match border {
        Some(s) => {
            let w = frame.stroke_px(s, false);
            let grow = w / 2.0;
            (
                Bounds::new(
                    top_left - point(px(grow), px(grow)),
                    extent + size(px(w), px(w)),
                ),
                if radius > 0.5 { radius + grow } else { 0.0 },
                w,
            )
        }
        None => (
            Bounds::new(top_left, extent),
            if radius > 0.5 { radius } else { 0.0 },
            0.0,
        ),
    };
    if background.is_some() || border.is_some() {
        let mut quad = fill(
            bounds,
            background.unwrap_or(Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }),
        )
        .corner_radii(px(corner));
        if let Some(s) = border {
            quad = quad
                .border_widths(px(width))
                .border_color(frame.colour(s.token, 1.0));
        }
        window.paint_quad(quad);
    }

    if let Some(s) = dashed {
        let mut pen = frame.pen(s, frame.stroke_px(s, false));
        outline(&mut pen, top_left, extent, radius);
        if let Ok(path) = pen.build() {
            window.paint_path(path, frame.colour(s.token, 1.0));
        }
    }
}

/// A closed rounded-rect outline (corners are quadratic curves with the sharp corner as the
/// control point, which is what an SVG `rx` draws).
fn outline(pen: &mut PathBuilder, at: Point<Pixels>, extent: gpui::Size<Pixels>, radius: f32) {
    let (w, h) = (f32::from(extent.width), f32::from(extent.height));
    let p = |x: f32, y: f32| at + point(px(x), px(y));
    if radius <= 0.5 {
        pen.add_polygon(&[p(0.0, 0.0), p(w, 0.0), p(w, h), p(0.0, h)], true);
        return;
    }
    let r = radius;
    pen.move_to(p(r, 0.0));
    pen.line_to(p(w - r, 0.0));
    pen.curve_to(p(w, r), p(w, 0.0));
    pen.line_to(p(w, h - r));
    pen.curve_to(p(w - r, h), p(w, h));
    pen.line_to(p(r, h));
    pen.curve_to(p(0.0, h - r), p(0.0, h));
    pen.line_to(p(0.0, r));
    pen.curve_to(p(r, 0.0), p(0.0, 0.0));
    pen.close();
}

/// An edge: the halo underlay (the mask colour, wider than the stroke), the stroke, the arrowhead.
fn polyline(p: &PolylineShape, frame: &Frame, window: &mut Window) {
    if p.points.len() < 2 {
        return;
    }
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for pt in &p.points {
        lo = [lo[0].min(pt[0]), lo[1].min(pt[1])];
        hi = [hi[0].max(pt[0]), hi[1].max(pt[1])];
    }
    if frame.clear_of_panel(frame.at(lo), frame.at(hi)) {
        return;
    }
    let cmds = p.cmds();
    let map = |pt: [f64; 2]| frame.at(pt);
    let width = frame.stroke_px(&p.stroke, false);

    if p.halo {
        let mut halo = PathBuilder::stroke(px(frame.len(p.halo_width()).max(width)));
        trace(&mut halo, &cmds, &map);
        if let Ok(path) = halo.build() {
            window.paint_path(path, frame.colour(Token::Mask, 1.0));
        }
    }

    let mut pen = frame.pen(&p.stroke, width);
    trace(&mut pen, &cmds, &map);
    if let Ok(path) = pen.build() {
        window.paint_path(path, frame.colour(p.stroke.token, 1.0));
    }

    if let (Some(marker), Some(arrow)) = (p.marker, p.arrow()) {
        let [a, b, c] = arrow.triangle();
        let mut head = PathBuilder::fill();
        head.add_polygon(&[frame.at(a), frame.at(b), frame.at(c)], true);
        if let Ok(path) = head.build() {
            window.paint_path(path, frame.colour(marker.fill, 1.0));
        }
    }
}

/// A sigil or glyph: commands in glyph space through the shape's SVG matrix, then the camera.
fn sigil(p: &PathShape, frame: &Frame, window: &mut Window) {
    let [a, b, c, d, e, f] = p.transform;
    let map = |pt: [f64; 2]| frame.at([a * pt[0] + c * pt[1] + e, b * pt[0] + d * pt[1] + f]);

    if let Some(f) = p.fill {
        let mut pen = PathBuilder::fill();
        trace(&mut pen, &p.cmds, &map);
        if let Ok(path) = pen.build() {
            window.paint_path(path, frame.colour(f.token, f.alpha * p.opacity));
        }
    }
    if let Some(s) = &p.stroke {
        let mut pen = frame.pen(s, frame.stroke_px(s, p.non_scaling));
        trace(&mut pen, &p.cmds, &map);
        if let Ok(path) = pen.build() {
            window.paint_path(path, frame.colour(s.token, p.opacity));
        }
    }
}

/// Path commands onto a builder through `map`.
fn trace(pen: &mut PathBuilder, cmds: &[Cmd], map: &dyn Fn([f64; 2]) -> Point<Pixels>) {
    for cmd in cmds {
        match *cmd {
            Cmd::M(p) => pen.move_to(map(p)),
            Cmd::L(p) => pen.line_to(map(p)),
            Cmd::Q(ctrl, to) => pen.curve_to(map(to), map(ctrl)),
            Cmd::C(c1, c2, to) => pen.cubic_bezier_to(map(to), map(c1), map(c2)),
            Cmd::Z => pen.close(),
        }
    }
}

/// A text run at `size * zoom`, on the SVG baseline, in the base monospace (D13). `Middle` centres
/// on `at.x`. Hidden when its reading depth is not shown at this tier.
fn text(t: &TextShape, frame: &Frame, window: &mut Window, cx: &mut App) {
    let shown = detail_factor(t.detail, frame.tier, frame.fade, frame.reveal.get());
    if t.text.is_empty() || shown <= 0.0 {
        return;
    }
    let font_px = frame.len(t.size);
    if font_px < MIN_TEXT_PX {
        return;
    }
    let at = frame.at(t.at);
    let reach = px(font_px * (t.text.chars().count() as f32 + 2.0));
    if frame.clear_of_panel(
        at - point(reach, px(font_px * 2.0)),
        at + point(reach, px(font_px * 2.0)),
    ) {
        return;
    }

    let mut font = window.text_style().font();
    font.family = ubiq_theme::MONO_FONT.into();
    font.weight = FontWeight(f32::from(t.weight));
    let run = TextRun {
        len: t.text.len(),
        font,
        color: frame.colour(t.token, shown).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(t.text.clone().into(), px(font_px), &[run], None);
    let left = match t.anchor {
        Anchor::Start => at.x,
        Anchor::Middle => at.x - shaped.width() / 2.0,
    };
    // A line box exactly as tall as its glyphs has its baseline `ascent` below the top.
    let (ascent, line) = (shaped.ascent, shaped.ascent + shaped.descent);
    let _ = shaped.paint(
        point(left, at.y - ascent),
        line,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::motion::{CometKind, StrokeKind};
    use ubiq_archify::scene::{Bounds as SceneBounds, EdgeRef, Group, SceneBuilder};
    use ubiq_archify::tokens::Kind;

    #[test]
    fn the_tier_is_zoom_over_fit_and_tags_wait_for_full() {
        let fit = 0.8;
        // At the fit (the camera's default) the reader is at `Read`: context shows, tags do not.
        let at_fit = tier_of(fit, fit);
        assert_eq!(at_fit, Tier::Read);
        assert!(Detail::Context.visible(at_fit));
        assert!(!Detail::Fine.visible(at_fit), "tags are hidden at fit");
        // Below the fit it is the map: only anchors.
        assert_eq!(tier_of(fit * 0.99, fit), Tier::Map);
        assert!(!Detail::Context.visible(Tier::Map));
        // Tags arrive at 1.75 times the fit, not before.
        assert_eq!(tier_of(fit * 1.74, fit), Tier::Read);
        assert_eq!(tier_of(fit * 1.75, fit), Tier::Full);
        assert!(Detail::Fine.visible(Tier::Full));
    }

    /// A node at (0,0)-(40,20), an edge from it along y = 60, and a frame around both.
    fn scene() -> Scene {
        let mut b = SceneBuilder::new(200.0, 100.0);
        b.group(Group::new(
            GroupKind::Frame,
            "frame-stage-x",
            SceneBounds::new(0.0, 0.0, 200.0, 100.0),
        ));
        b.group(Group::node(
            "api",
            Kind::Backend,
            "API",
            SceneBounds::new(0.0, 0.0, 40.0, 20.0),
        ));
        let edge = EdgeRef {
            key: 1,
            from: "api".into(),
            to: "db".into(),
            id: None,
        };
        b.group(Group::edge(edge, "", &[[0.0, 60.0], [100.0, 60.0]], 2.0));
        b.build()
    }

    fn stroke(non_scaling: bool, width: f64, alpha: f64, glow: f64) -> OverlayStroke {
        OverlayStroke {
            kind: StrokeKind::Comet(CometKind::Intent),
            group: GroupId(1),
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            token: Token::ArrowEmphasis,
            width,
            non_scaling,
            alpha,
            glow,
        }
    }

    #[test]
    fn a_comet_is_in_screen_px_and_an_ambient_dash_scales_with_the_camera() {
        let comet = stroke_pens(&stroke(true, 1.8, 1.0, 0.0), 3.0, 1.0);
        assert_eq!(
            comet,
            [Pen {
                width: 1.8,
                alpha: 1.0
            }]
        );
        let dash = stroke_pens(&stroke(false, 2.0, 1.0, 0.0), 3.0, 1.0);
        assert_eq!(
            dash,
            [Pen {
                width: 6.0,
                alpha: 1.0
            }]
        );
        let thin = stroke_pens(&stroke(false, 0.1, 1.0, 0.0), 1.0, 1.0);
        assert_eq!(thin[0].width, HAIRLINE, "the hairline floor");
    }

    #[test]
    fn a_glow_is_a_wider_fainter_pen_under_the_core_and_the_group_alpha_multiplies_both() {
        let pens = stroke_pens(&stroke(true, 2.0, 0.8, 6.0), 1.0, 0.5);
        assert_eq!(pens.len(), 2);
        assert_eq!(pens[0].width, 8.0);
        assert!((pens[0].alpha - 0.8 * 0.5 * GLOW_ALPHA).abs() < 1e-9);
        assert_eq!(
            pens[1],
            Pen {
                width: 2.0,
                alpha: 0.4
            }
        );
        assert!(stroke_pens(&stroke(true, 2.0, 0.8, 6.0), 1.0, 0.0).is_empty());
    }

    fn mark(width: f64, alpha: f64, glow: f64) -> Highlight {
        Highlight {
            group: GroupId(0),
            rect: SceneBounds::new(0.0, 0.0, 40.0, 20.0),
            radius: 8.0,
            token: Token::ArrowEmphasis,
            width,
            alpha,
            glow,
        }
    }

    #[test]
    fn a_highlight_is_an_outline_centred_on_the_edge_with_the_glow_outside_it() {
        let rings = highlight_rings(&mark(1.5, 1.0, 10.0), 2.0, 1.0);
        assert_eq!(rings.len(), 2);
        // Glow: 10 px just outside a 3 px outline (half of it is inside the rect's edge).
        assert_eq!(
            rings[0],
            Ring {
                grow: 11.5,
                border: 10.0,
                alpha: GLOW_ALPHA
            }
        );
        assert_eq!(
            rings[1],
            Ring {
                grow: 1.5,
                border: 3.0,
                alpha: 1.0
            }
        );
        // The intent trace's selected node has a glow and no outline.
        let glow_only = highlight_rings(&mark(0.0, 0.5, 10.0), 2.0, 1.0);
        assert_eq!(glow_only.len(), 1);
        assert_eq!(glow_only[0].grow, 10.0);
        assert!(highlight_rings(&mark(1.0, 1.0, 4.0), 1.0, 0.0).is_empty());
    }

    #[test]
    fn a_detail_fade_replaces_the_tier_for_context_and_fine_but_never_for_anchors() {
        let fade = Some(DetailAlpha {
            context: 0.4,
            fine: 0.0,
        });
        assert_eq!(detail_factor(Detail::Context, Tier::Map, None, false), 0.0);
        assert_eq!(detail_factor(Detail::Context, Tier::Read, None, false), 1.0);
        assert_eq!(detail_factor(Detail::Fine, Tier::Read, None, false), 0.0);
        assert_eq!(detail_factor(Detail::Context, Tier::Read, fade, false), 0.4);
        assert_eq!(detail_factor(Detail::Fine, Tier::Full, fade, false), 0.0);
        assert_eq!(detail_factor(Detail::Anchor, Tier::Map, fade, false), 1.0);
        assert_eq!(
            detail_factor(Detail::Fine, Tier::Map, fade, true),
            1.0,
            "revealed"
        );
    }

    #[test]
    fn a_click_focuses_a_node_and_anything_else_clears() {
        let s = scene();
        assert_eq!(click_target(&s, [10.0, 10.0], 1.0).as_deref(), Some("api"));
        assert_eq!(click_target(&s, [50.0, 60.0], 1.0), None, "an edge clears");
        assert_eq!(click_target(&s, [150.0, 90.0], 1.0), None, "a frame clears");
        assert_eq!(
            click_target(&s, [500.0, 500.0], 1.0),
            None,
            "empty space clears"
        );
    }

    #[test]
    fn a_tooltip_is_for_nodes_and_edges_not_frames() {
        let s = scene();
        let node = tip_target(&s, [10.0, 10.0], 1.0).unwrap();
        assert_eq!(s.tooltip(node).as_deref(), Some("API"));
        assert!(tip_target(&s, [50.0, 61.0], 1.0).is_some());
        assert!(tip_target(&s, [150.0, 90.0], 1.0).is_none());
    }

    #[test]
    fn a_window_point_maps_through_the_camera_to_the_view_box() {
        let content = Content::from_size(200.0, 100.0);
        let mut vp = Viewport::default();
        vp.set_panel(248.0, 148.0, 100.0, 50.0);
        // 248 - 2 * 24 = 200 wide: the fit scale is 1 and the picture is offset by the margin.
        let (p, zoom) = vu_at(
            &vp,
            content,
            point(px(100.0 + 24.0 + 10.0), px(50.0 + 24.0 + 5.0)),
        )
        .unwrap();
        assert_eq!(zoom, 1.0);
        assert!(
            (p[0] - 10.0).abs() < 1e-4 && (p[1] - 5.0).abs() < 1e-4,
            "{p:?}"
        );
        assert!(vu_at(&Viewport::default(), content, point(px(0.0), px(0.0))).is_none());
    }
}
