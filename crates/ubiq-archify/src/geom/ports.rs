//! Sides, anchors and port spreading: `anchor`, `defaultFromSide`/`defaultToSide` (architecture,
//! dominant axis), `legacyDefaultFromSide`/`legacyDefaultToSide` (everything else, horizontal-first),
//! `chosenSide` and `automaticPortSpread` of `renderers/shared/geometry.mjs` (00 §5.6.1–2).

use std::cmp::Ordering;
use std::collections::HashMap;

use super::rect::{Pt, Rect};

/// A side of a rect. Authored `auto` (or no side) is `None` wherever a side is optional.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    /// `left|right|top|bottom`; anything else (`auto`, unknown) is `None`.
    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            "top" => Some(Side::Top),
            "bottom" => Some(Side::Bottom),
            _ => None,
        }
    }

    /// Ports on a left/right side spread along y; on top/bottom along x.
    pub fn is_vertical_edge(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }
}

/// `anchor(rect, side)`: the midpoint of that side.
pub fn anchor(rect: &Rect, side: Side) -> Pt {
    match side {
        Side::Left => [rect.x, rect.cy()],
        Side::Right => [rect.x + rect.width, rect.cy()],
        Side::Top => [rect.cx(), rect.y],
        Side::Bottom => [rect.cx(), rect.y + rect.height],
    }
}

/// `anchor` for a side that may be unknown: JS falls back to the right edge (`auto`, `undefined`).
pub fn anchor_or_right(rect: &Rect, side: Option<Side>) -> Pt {
    anchor(rect, side.unwrap_or(Side::Right))
}

/// `chosenSide(side, fallback)`: an authored side wins; `auto`/absent uses the geometric fallback.
pub fn chosen_side(side: Option<Side>, fallback: Side) -> Side {
    side.unwrap_or(fallback)
}

/// `defaultFromSide` (architecture, dominant axis): `dx != 0 && |dx| >= |dy|` selects left/right
/// (equal deltas stay horizontal); otherwise top/bottom by `dy`, with coincident centres on `top`.
pub fn default_from_side(from: &Rect, to: &Rect) -> Side {
    let dx = to.cx() - from.cx();
    let dy = to.cy() - from.cy();
    if dx != 0.0 && dx.abs() >= dy.abs() {
        return if dx < 0.0 { Side::Left } else { Side::Right };
    }
    if dy > 0.0 { Side::Bottom } else { Side::Top }
}

/// `defaultToSide`: the mirror of [`default_from_side`].
pub fn default_to_side(from: &Rect, to: &Rect) -> Side {
    let dx = to.cx() - from.cx();
    let dy = to.cy() - from.cy();
    if dx != 0.0 && dx.abs() >= dy.abs() {
        return if dx < 0.0 { Side::Right } else { Side::Left };
    }
    if dy > 0.0 { Side::Top } else { Side::Bottom }
}

/// `legacyDefaultFromSide` (every non-architecture type): compare `cx`, tie falls to `cy`.
pub fn legacy_default_from_side(from: &Rect, to: &Rect) -> Side {
    if to.cx() < from.cx() {
        Side::Left
    } else if to.cx() > from.cx() {
        Side::Right
    } else if to.cy() > from.cy() {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// `legacyDefaultToSide`.
pub fn legacy_default_to_side(from: &Rect, to: &Rect) -> Side {
    if to.cx() < from.cx() {
        Side::Right
    } else if to.cx() > from.cx() {
        Side::Left
    } else if to.cy() > from.cy() {
        Side::Top
    } else {
        Side::Bottom
    }
}

/// Which end of a relationship a side or port belongs to (`'source'`/`'target'`, `'from'`/`'to'`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    From,
    To,
}

/// A relationship as `automaticPortSpread` reads it. `auto` is the eligibility test the JS runs
/// inline: no `via`, `route` absent or `auto`, no `channelX`/`channelY`, no `labelAt`. Callers set
/// it; sequence diagrams never call the spread at all.
#[derive(Clone, Debug, Default)]
pub struct SpreadRelation {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    pub from_side: Option<Side>,
    pub to_side: Option<Side>,
    pub auto: bool,
}

/// Spread tuning: `gutter` 16, `maxSpacing` 14 (Archify's defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadOptions {
    pub gutter: f64,
    pub max_spacing: f64,
}

impl Default for SpreadOptions {
    fn default() -> Self {
        Self {
            gutter: 16.0,
            max_spacing: 14.0,
        }
    }
}

/// The spread ports of one relationship: `Some` only for an endpoint in a shared `(node, side)` group.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpreadPorts {
    pub from: Option<Pt>,
    pub to: Option<Pt>,
}

/// `automaticPortSpread(relations, boxes, { gutter, maxSpacing, sideFor, spacingFor })`.
///
/// Endpoints are grouped by `(node, side)`; a group of two or more is sorted by the counterpart's
/// centre along the side axis (ties by `id\0from\0to\0label`, UTF-16 order as JS compares), then
/// laid out at `(i - (n-1)/2) * spacing` from the side midpoint, `spacing = min(max_spacing,
/// usable / (n-1))`, `usable = max(0, extent - 2 * gutter)`. The result is indexed like `relations`.
///
/// `side_for(i, end)` overrides the inferred side when it returns `Some` (JS `sideFor(...) || default`);
/// `spacing_for(i, j)` is the width-aware gap between consecutive relations `i`, `j` in a group
/// (architecture's arrowhead rule): when any gap exceeds `max_spacing` and the span fits `usable`,
/// the whole group uses those gaps, centred.
pub fn automatic_port_spread(
    relations: &[SpreadRelation],
    boxes: &HashMap<String, Rect>,
    opts: SpreadOptions,
    side_for: Option<&dyn Fn(usize, End) -> Option<Side>>,
    spacing_for: Option<&dyn Fn(usize, usize) -> f64>,
) -> Vec<SpreadPorts> {
    struct Item {
        rel: usize,
        end: End,
        rect: Rect,
        side: Side,
        counterpart: Rect,
    }
    let mut spread = vec![SpreadPorts::default(); relations.len()];
    let mut groups: HashMap<(&str, Side), Vec<Item>> = HashMap::new();

    for (i, rel) in relations.iter().enumerate() {
        if !rel.auto {
            continue;
        }
        let (Some(from), Some(to)) = (boxes.get(&rel.from), boxes.get(&rel.to)) else {
            continue;
        };
        let from_side = chosen_side(
            rel.from_side,
            side_for
                .and_then(|f| f(i, End::From))
                .unwrap_or_else(|| default_from_side(from, to)),
        );
        let to_side = chosen_side(
            rel.to_side,
            side_for
                .and_then(|f| f(i, End::To))
                .unwrap_or_else(|| default_to_side(from, to)),
        );
        groups
            .entry((rel.from.as_str(), from_side))
            .or_default()
            .push(Item {
                rel: i,
                end: End::From,
                rect: *from,
                side: from_side,
                counterpart: *to,
            });
        groups
            .entry((rel.to.as_str(), to_side))
            .or_default()
            .push(Item {
                rel: i,
                end: End::To,
                rect: *to,
                side: to_side,
                counterpart: *from,
            });
    }

    let sort_key = |r: &SpreadRelation| format!("{}\0{}\0{}\0{}", r.id, r.from, r.to, r.label);
    for mut items in groups.into_values() {
        if items.len() < 2 {
            continue;
        }
        let vertical_side = items[0].side.is_vertical_edge();
        items.sort_by(|a, b| {
            let (ac, bc) = if vertical_side {
                (a.counterpart.cy(), b.counterpart.cy())
            } else {
                (a.counterpart.cx(), b.counterpart.cx())
            };
            if ac != bc {
                // JS `return a - b`: NaN or equal-after-subtraction compares as 0.
                let d = ac - bc;
                return if d < 0.0 {
                    Ordering::Less
                } else if d > 0.0 {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                };
            }
            let ak = sort_key(&relations[a.rel]);
            let bk = sort_key(&relations[b.rel]);
            ak.encode_utf16().cmp(bk.encode_utf16())
        });

        let extent = if vertical_side {
            items[0].rect.height
        } else {
            items[0].rect.width
        };
        let usable = (extent - opts.gutter * 2.0).max(0.0);
        let n = items.len();
        let spacing = opts.max_spacing.min(usable / (n - 1) as f64);
        if spacing.is_nan() || spacing <= 0.0 {
            continue;
        }

        // Width-aware callers reserve the whole group together.
        let mut offsets: Option<Vec<f64>> = None;
        if let Some(spacing_for) = spacing_for {
            let gaps: Vec<f64> = (1..n)
                .map(|k| {
                    opts.max_spacing
                        .max(spacing_for(items[k - 1].rel, items[k].rel))
                })
                .collect();
            let span: f64 = gaps.iter().sum();
            if span <= usable && gaps.iter().any(|g| *g > opts.max_spacing) {
                let mut offset = -span / 2.0;
                let mut v = vec![offset];
                for g in &gaps {
                    offset += g;
                    v.push(offset);
                }
                offsets = Some(v);
            }
        }

        for (k, item) in items.iter().enumerate() {
            let offset = match &offsets {
                Some(o) => o[k],
                None => (k as f64 - (n as f64 - 1.0) / 2.0) * spacing,
            };
            let mut point = anchor(&item.rect, item.side);
            if vertical_side {
                point[1] += offset;
            } else {
                point[0] += offset;
            }
            match item.end {
                End::From => spread[item.rel].from = Some(point),
                End::To => spread[item.rel].to = Some(point),
            }
        }
    }
    spread
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(cx: f64, cy: f64) -> Rect {
        Rect::from_center(cx, cy, 0.0, 0.0)
    }

    #[test]
    fn anchor_returns_the_edge_midpoint_for_each_side() {
        let r = Rect::new(100., 100., 40., 20.); // cx=120 cy=110
        assert_eq!(anchor(&r, Side::Left), [100., 110.]);
        assert_eq!(anchor(&r, Side::Right), [140., 110.]);
        assert_eq!(anchor(&r, Side::Top), [120., 100.]);
        assert_eq!(anchor(&r, Side::Bottom), [120., 120.]);
    }

    #[test]
    fn anchor_falls_back_to_the_right_edge_for_unknown_or_auto_sides() {
        let r = Rect::new(100., 100., 40., 20.);
        assert_eq!(anchor_or_right(&r, Side::parse("auto")), [140., 110.]);
        assert_eq!(anchor_or_right(&r, None), [140., 110.]);
        assert_eq!(anchor_or_right(&r, Side::parse("top")), [120., 100.]);
    }

    #[test]
    fn default_sides_are_mirror_pairs() {
        let a = at(0., 0.);
        let right = at(100., 0.);
        assert_eq!(default_from_side(&a, &right), Side::Right);
        assert_eq!(default_to_side(&a, &right), Side::Left);
        let below = at(0., 100.);
        assert_eq!(default_from_side(&a, &below), Side::Bottom);
        assert_eq!(default_to_side(&a, &below), Side::Top);
    }

    #[test]
    fn default_sides_follow_the_dominant_axis_not_any_horizontal_delta() {
        let hub = at(288., 66.);
        let spoke = at(124., 266.); // dx = -164, dy = +200
        assert_eq!(default_from_side(&hub, &spoke), Side::Bottom);
        assert_eq!(default_to_side(&hub, &spoke), Side::Top);
        assert_eq!(default_from_side(&spoke, &hub), Side::Top);
        assert_eq!(default_to_side(&spoke, &hub), Side::Bottom);

        // Equal deltas keep the horizontal side.
        let diagonal = at(100., 100.);
        assert_eq!(default_from_side(&at(0., 0.), &diagonal), Side::Right);
        assert_eq!(default_to_side(&at(0., 0.), &diagonal), Side::Left);

        // A pure vertical delta resolves vertically.
        assert_eq!(
            default_from_side(&at(50., 0.), &at(50., 100.)),
            Side::Bottom
        );
        assert_eq!(default_to_side(&at(50., 0.), &at(50., 100.)), Side::Top);
        assert_eq!(default_from_side(&at(50., 0.), &at(50., -100.)), Side::Top);
        assert_eq!(default_to_side(&at(50., 0.), &at(50., -100.)), Side::Bottom);

        // Coincident centres stay on the vertical fallback.
        assert_eq!(default_from_side(&at(7., 7.), &at(7., 7.)), Side::Top);
        assert_eq!(default_to_side(&at(7., 7.), &at(7., 7.)), Side::Bottom);
    }

    #[test]
    fn legacy_sides_are_horizontal_first() {
        // The same hub/spoke pair: the architecture rule goes vertical, the legacy rule horizontal.
        let hub = at(288., 66.);
        let spoke = at(124., 266.);
        assert_eq!(legacy_default_from_side(&hub, &spoke), Side::Left);
        assert_eq!(legacy_default_to_side(&hub, &spoke), Side::Right);
        assert_eq!(legacy_default_from_side(&spoke, &hub), Side::Right);
        assert_eq!(legacy_default_to_side(&spoke, &hub), Side::Left);
        // cx tie falls to cy; full tie is top / bottom.
        assert_eq!(
            legacy_default_from_side(&at(5., 0.), &at(5., 9.)),
            Side::Bottom
        );
        assert_eq!(legacy_default_to_side(&at(5., 0.), &at(5., 9.)), Side::Top);
        assert_eq!(
            legacy_default_from_side(&at(5., 0.), &at(5., -9.)),
            Side::Top
        );
        assert_eq!(
            legacy_default_to_side(&at(5., 0.), &at(5., 0.)),
            Side::Bottom
        );
    }

    #[test]
    fn chosen_side_treats_auto_as_use_the_geometric_fallback() {
        assert_eq!(chosen_side(Side::parse("left"), Side::Right), Side::Left);
        assert_eq!(chosen_side(Side::parse("auto"), Side::Right), Side::Right);
        assert_eq!(chosen_side(None, Side::Right), Side::Right);
    }

    fn rel(id: &str, from: &str, to: &str) -> SpreadRelation {
        SpreadRelation {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            auto: true,
            ..Default::default()
        }
    }

    fn boxes() -> HashMap<String, Rect> {
        // A hub at x 0..100, y 0..100, three targets to its right at different heights.
        HashMap::from([
            ("hub".to_string(), Rect::new(0., 0., 100., 100.)),
            ("a".to_string(), Rect::new(300., -100., 40., 40.)),
            ("b".to_string(), Rect::new(300., 30., 40., 40.)),
            ("c".to_string(), Rect::new(300., 160., 40., 40.)),
        ])
    }

    #[test]
    fn spread_orders_by_counterpart_and_centres_on_the_midpoint() {
        // Listed out of order on purpose: the spread sorts by counterpart cy (a < b < c).
        let rels = [
            rel("2", "hub", "c"),
            rel("0", "hub", "a"),
            rel("1", "hub", "b"),
        ];
        let out = automatic_port_spread(&rels, &boxes(), SpreadOptions::default(), None, None);
        // Right side of the hub: x = 100, midpoint y = 50, spacing min(14, (100-32)/2) = 14.
        assert_eq!(out[1].from, Some([100., 36.])); // a
        assert_eq!(out[2].from, Some([100., 50.])); // b
        assert_eq!(out[0].from, Some([100., 64.])); // c
        // Each target has a single port: no spread on the target side.
        assert!(out.iter().all(|p| p.to.is_none()));
    }

    #[test]
    fn spread_spacing_shrinks_to_the_usable_extent() {
        // Hub height 40: usable = 8, three ports -> spacing min(14, 4) = 4.
        let mut b = boxes();
        b.insert("hub".into(), Rect::new(0., 0., 100., 40.));
        let rels = [
            rel("0", "hub", "a"),
            rel("1", "hub", "b"),
            rel("2", "hub", "c"),
        ];
        let out = automatic_port_spread(&rels, &b, SpreadOptions::default(), None, None);
        assert_eq!(out[0].from, Some([100., 16.]));
        assert_eq!(out[1].from, Some([100., 20.]));
        assert_eq!(out[2].from, Some([100., 24.]));
    }

    #[test]
    fn spread_skips_a_side_with_no_room_and_single_ports() {
        let mut b = boxes();
        b.insert("hub".into(), Rect::new(0., 0., 100., 32.)); // usable 0 -> spacing 0
        let rels = [rel("0", "hub", "a"), rel("1", "hub", "b")];
        let out = automatic_port_spread(&rels, &b, SpreadOptions::default(), None, None);
        assert!(out.iter().all(|p| p.from.is_none() && p.to.is_none()));
        let one = automatic_port_spread(
            &[rel("0", "hub", "a")],
            &boxes(),
            SpreadOptions::default(),
            None,
            None,
        );
        assert_eq!(one[0], SpreadPorts::default());
    }

    #[test]
    fn spread_ties_break_on_the_relationship_key() {
        // b and b2 share cy 50; "x" < "y" decides.
        let mut bx = boxes();
        bx.insert("b2".into(), Rect::new(300., 30., 40., 40.));
        let rels = [rel("y", "hub", "b2"), rel("x", "hub", "b")];
        let out = automatic_port_spread(&rels, &bx, SpreadOptions::default(), None, None);
        assert_eq!(out[1].from, Some([100., 43.])); // x first: offset -7
        assert_eq!(out[0].from, Some([100., 57.]));
    }

    #[test]
    fn spread_honours_authored_sides_and_ineligible_routes() {
        let mut r0 = rel("0", "hub", "a");
        r0.from_side = Some(Side::Top);
        let mut r1 = rel("1", "hub", "b");
        r1.auto = false; // via / explicit route / channel / labelAt
        let r2 = rel("2", "hub", "c");
        let out = automatic_port_spread(
            &[r0, r1, r2],
            &boxes(),
            SpreadOptions::default(),
            None,
            None,
        );
        // r0 leaves the top, r2 leaves the right: no shared group, so nothing is spread.
        assert!(out.iter().all(|p| p.from.is_none()));
        // Two relationships forced to the same top side spread along x.
        let mut a = rel("0", "hub", "a");
        let mut c = rel("2", "hub", "c");
        a.from_side = Some(Side::Top);
        c.from_side = Some(Side::Top);
        let out = automatic_port_spread(&[a, c], &boxes(), SpreadOptions::default(), None, None);
        assert_eq!(out[0].from, Some([43., 0.]));
        assert_eq!(out[1].from, Some([57., 0.]));
    }

    #[test]
    fn spread_groups_incoming_ports_per_target_side() {
        let rels = [rel("0", "a", "hub"), rel("1", "c", "hub")];
        let out = automatic_port_spread(&rels, &boxes(), SpreadOptions::default(), None, None);
        // Both arrive on the hub's right side (centres are to the right).
        assert_eq!(out[0].to, Some([100., 43.]));
        assert_eq!(out[1].to, Some([100., 57.]));
        assert!(out.iter().all(|p| p.from.is_none()));
    }

    #[test]
    fn spread_uses_width_aware_gaps_when_wider_than_the_default_and_they_fit() {
        let rels = [
            rel("0", "hub", "a"),
            rel("1", "hub", "b"),
            rel("2", "hub", "c"),
        ];
        let wide = |_: usize, _: usize| 20.0;
        let out =
            automatic_port_spread(&rels, &boxes(), SpreadOptions::default(), None, Some(&wide));
        // span 40 <= usable 68: offsets -20, 0, 20.
        assert_eq!(out[0].from, Some([100., 30.]));
        assert_eq!(out[1].from, Some([100., 50.]));
        assert_eq!(out[2].from, Some([100., 70.]));
        // Gaps that do not exceed the legacy spacing, or do not fit, leave the legacy layout.
        let narrow = |_: usize, _: usize| 3.0;
        let out = automatic_port_spread(
            &rels,
            &boxes(),
            SpreadOptions::default(),
            None,
            Some(&narrow),
        );
        assert_eq!(out[0].from, Some([100., 36.]));
        let huge = |_: usize, _: usize| 40.0;
        let out =
            automatic_port_spread(&rels, &boxes(), SpreadOptions::default(), None, Some(&huge));
        assert_eq!(out[2].from, Some([100., 64.]));
    }

    #[test]
    fn spread_side_for_overrides_the_inferred_side() {
        let rels = [rel("0", "hub", "a"), rel("1", "hub", "b")];
        let bottom = |_: usize, end: End| (end == End::From).then_some(Side::Bottom);
        let out = automatic_port_spread(
            &rels,
            &boxes(),
            SpreadOptions::default(),
            Some(&bottom),
            None,
        );
        // Both leave the bottom (x spread around 50), counterparts a.cx == b.cx so the key decides.
        assert_eq!(out[0].from, Some([43., 100.]));
        assert_eq!(out[1].from, Some([57., 100.]));
    }
}
