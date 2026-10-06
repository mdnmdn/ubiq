//! The repair text appended to label and spacing problems (`geometry.mjs` `suggestLabelObstacleFix`,
//! `suggestLabelPairFix`, `suggestComponentSeparation`). Verbatim wording; every suggested value is a
//! replacement for the authored field, and a suggestion that would leave the canvas is dropped.

use super::{LabelRect, Obstacle, format_rect, num};
use crate::diag::js_round;
use crate::geom::{Rect, rects_overlap};

/// How far `[start, start + size)` leaves `[0, extent)` at each end.
fn overflow(start: f64, size: f64, extent: f64) -> (f64, f64) {
    if extent.is_finite() { (-start, start + size - extent) } else { (0.0, 0.0) }
}

fn anchor_fits(anchor: f64, offset: f64, size: f64, extent: f64) -> bool {
    let (a, b) = overflow(anchor + offset, size, extent);
    a <= 0.0 && b <= 0.0
}

/// `clampAnchorToCanvas`: `None` when the rect is wider than the canvas.
fn clamp_anchor(anchor: f64, offset: f64, size: f64, extent: f64) -> Option<f64> {
    let rounded = js_round(anchor);
    if !extent.is_finite() {
        return Some(rounded);
    }
    let (min, max) = ((-offset).ceil(), (extent - size - offset).floor());
    (max >= min).then(|| rounded.max(min).min(max))
}

/// `suggestLabelObstacleFix`: where to put a label that hit `obstacle`, above or below it, clear of
/// every `blockers` rect and inside the canvas.
pub fn label_obstacle_fix(
    label: &LabelRect,
    obstacle: &Obstacle,
    kind: &str,
    view_box: [f64; 2],
    blockers: &[Obstacle],
) -> String {
    let [width, height] = view_box;
    let (lx, ly) = (label.anchor[0], label.anchor[1]);
    let r = label.rect;
    let (x_offset, y_offset) = (r.x - lx, r.y - ly);
    let anchor_x = clamp_anchor(lx, x_offset, r.width, width);
    let o = obstacle.rect;
    let placements: Vec<(&str, f64)> = [
        ("below", js_round(o.y + o.height + 4.0 - y_offset)),
        ("above", js_round(o.y - 4.0 - r.height - y_offset)),
    ]
    .into_iter()
    .filter(|&(_, y)| {
        let Some(ax) = anchor_x else { return false };
        anchor_fits(y, y_offset, r.height, height)
            && blockers.iter().all(|b| {
                !rects_overlap(&Rect::new(ax + x_offset, y + y_offset, r.width, r.height), &b.rect, -2.0)
            })
    })
    .collect();
    let mut lines = vec![
        format!("  label rect: {}", format_rect(&r)),
        format!("  {kind} \"{}\" rect: {}", obstacle.id, format_rect(&o)),
    ];
    let (w, h) = (num(width), num(height));
    let Some(ax) = anchor_x.filter(|_| !placements.is_empty()) else {
        lines.push(if anchor_x.is_none() {
            format!(
                "  Suggested fix: the {}px label rect cannot fit the {w}x{h} viewBox at any anchor — shorten the label or enlarge meta.viewBox",
                num(js_round(r.width))
            )
        } else {
            format!(
                "  Suggested fix: no placement directly above or below \"{}\" stays clear inside the {w}x{h} viewBox — move the label to an open area with labelAt, shorten the label, move the {kind}, or enlarge meta.viewBox",
                obstacle.id
            )
        });
        return lines.join("\n");
    };
    let relative = |y: f64| -> Option<String> {
        if label.authored_at {
            return None;
        }
        let (adx, ady) = (label.authored_dx, label.authored_dy);
        let dx = js_round(adx + ax - lx);
        let dy = js_round(ady + y - ly);
        let moves_x = (dx - adx).abs() >= 0.5;
        let lands_x = lx - adx + if moves_x { dx } else { adx };
        let lands_y = ly - ady + dy;
        if !anchor_fits(lands_x, x_offset, r.width, width) || !anchor_fits(lands_y, y_offset, r.height, height) {
            return None;
        }
        Some(if moves_x {
            format!("set labelDx {} with labelDy {}", num(dx), num(dy))
        } else {
            format!("set labelDy {}", num(dy))
        })
    };
    let hint = placements
        .iter()
        .map(|&(name, y)| {
            let rel = relative(y).map(|r| format!(" or {r}")).unwrap_or_default();
            format!("set labelAt [{}, {}]{rel} ({name})", num(ax), num(y))
        })
        .collect::<Vec<_>>()
        .join("; or ");
    lines.push(format!("  Suggested fix: {hint}"));
    lines.join("\n")
}

/// `suggestLabelPairFix`.
pub fn label_pair_fix(a: &LabelRect, b: &LabelRect) -> String {
    [
        format!("  \"{}\" {}; \"{}\" {}", a.text, format_rect(&a.rect), b.text, format_rect(&b.rect)),
        "  Suggested fix: adjust labelDx/labelDy/labelSegment, or route one relationship through a separate corridor"
            .to_owned(),
    ]
    .join("\n")
}

/// `suggestComponentSeparation`.
pub fn component_separation(a: &Obstacle, b: &Obstacle, min_gap: f64) -> String {
    let right_x = js_round(a.rect.x + a.rect.width + min_gap);
    let below_y = js_round(a.rect.y + a.rect.height + min_gap);
    [
        format!("  \"{}\" {}; \"{}\" {}", a.id, format_rect(&a.rect), b.id, format_rect(&b.rect)),
        format!(
            "  Suggested fix: move \"{}\" pos to [{}, {}] (right of \"{}\") or [{}, {}] (below)",
            b.id,
            num(right_x),
            num(js_round(b.rect.y)),
            a.id,
            num(js_round(b.rect.x)),
            num(below_y)
        ),
    ]
    .join("\n")
}
