//! The quota ring: an account's rolling windows as concentric bands, each with a pace tick.
//!
//! One component for the conversation footer and the harness settings, so the two surfaces can
//! never disagree about what the glyph or its tooltip say.

use gpui::{
    AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div,
};
use ubiq_proto::quota::QuotaSnapshot;

use super::controls::{Band, progress_rings};
use crate::state::settings::{quota_ring_tip, window_paces};
use crate::theme;

/// The ring for `snapshot` at `diameter`, with the tooltip that names every window and its pace.
/// The first two windows that state a percentage are drawn, the shortest outside; each carries a
/// tick at the share of its window that has elapsed (see [`crate::state::settings::pace`]).
/// `None` when no window states a percentage — no ring beats a wrong one.
pub fn quota_ring(
    id: impl Into<ElementId>,
    snapshot: &QuotaSnapshot,
    diameter: f32,
    now_ms: i64,
) -> Option<AnyElement> {
    let paces = window_paces(snapshot, now_ms);
    let bands: Vec<Band> = snapshot
        .windows()
        .iter()
        .take(2)
        .zip(paces)
        .filter_map(|(gauge, pace)| {
            let pct = gauge.reading.used_pct()?;
            let mut band = Band::new(pct, theme::usage_tone(pct));
            band.tick = pace.map(|pace| (pace.elapsed / 100.0, theme::pace_tick(pace.ahead())));
            Some(band)
        })
        .collect();
    if bands.is_empty() {
        return None;
    }
    let tip = quota_ring_tip(snapshot, now_ms);
    Some(
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .child(progress_rings(bands, diameter))
            .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
            .into_any_element(),
    )
}
