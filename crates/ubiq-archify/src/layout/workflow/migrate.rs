//! The v1 to v2 projections the `column-capacity` fix check needs
//! (`workflow-migration-geometry.mjs`): [`intrinsic_workflow`], [`planning_workflow`] and
//! [`mapped_candidate`], whose piecewise-linear [`rank_mapper`] carries every absolute X pin of a
//! schema-1 document over to the readable rank plan.
//!
//! `migrations/workflow-v2.mjs`, Archify's `migrate` command, is not ported: nothing here writes a
//! migrated file (D19). `gates::workflow_v1` only asks whether the migrated document compiles.

use crate::model::common::SchemaVersion;
use crate::model::workflow::{Edge, EdgeRoute, Workflow};

/// `LEGACY_COLUMN_CENTERS`, the fixed v1 column centres.
pub const LEGACY_COLUMN_CENTERS: [f64; 6] = [88.0, 220.0, 300.0, 430.0, 500.0, 625.0];

/// `intrinsicWorkflow`: the document as schema 2, without its authored `meta.viewBox`.
pub fn intrinsic_workflow(w: &Workflow) -> Workflow {
    let mut out = w.clone();
    out.schema_version = SchemaVersion::V2;
    out.meta.view_box = None;
    out
}

/// `planningWorkflow`: [`intrinsic_workflow`] without the edges whose authored route geometry may
/// only become valid once its X pins are remapped; the rest keep what moves ranks (identity,
/// endpoints, variant, role, width, `straight`, the label unless it is pinned).
pub fn planning_workflow(w: &Workflow) -> Workflow {
    let mut planned = intrinsic_workflow(w);
    planned.edges = planned
        .edges
        .iter()
        .filter(|e| {
            let routed = e.via.is_some()
                || e.route.is_some_and(|r| !matches!(r, EdgeRoute::Auto | EdgeRoute::Straight))
                || e.channel_x.is_some()
                || e.channel_y.is_some();
            !routed
        })
        .map(|e| Edge {
            id: e.id.clone(),
            from: e.from.clone(),
            to: e.to.clone(),
            label: if e.label_at.is_none() { e.label.clone() } else { None },
            variant: e.variant,
            role: e.role,
            from_side: None,
            to_side: None,
            route: (e.route == Some(EdgeRoute::Straight)).then_some(EdgeRoute::Straight),
            via: None,
            label_at: None,
            label_dx: None,
            label_dy: None,
            label_segment: None,
            channel_x: None,
            channel_y: None,
            bias: None,
            width: e.width,
        })
        .collect();
    if let Some(path) = &planned.main_path {
        let pairs: Vec<(&str, &str)> = planned.edges.iter().map(|e| (e.from.as_str(), e.to.as_str())).collect();
        let breaks = path.windows(2).any(|p| !pairs.contains(&(p[0].as_str(), p[1].as_str())));
        if breaks {
            planned.main_path = None;
        }
    }
    planned
}

/// `createHorizontalRankMapper`; `None` where Archify throws (mismatched, non-finite or not
/// strictly increasing columns).
pub fn rank_mapper(old: &[f64], new: &[f64]) -> Option<impl Fn(f64) -> f64 + use<>> {
    let valid = old.len() == new.len()
        && old.len() >= 2
        && old.iter().chain(new).all(|c| c.is_finite())
        && old.windows(2).all(|p| p[1] > p[0])
        && new.windows(2).all(|p| p[1] > p[0]);
    if !valid {
        return None;
    }
    let (old, new) = (old.to_vec(), new.to_vec());
    Some(move |x: f64| {
        let mut segment = old.len() - 2;
        if x <= old[0] {
            segment = 0;
        } else if let Some(i) = (0..old.len() - 1).find(|&i| x <= old[i + 1]) {
            segment = i;
        }
        let ratio = (x - old[segment]) / (old[segment + 1] - old[segment]);
        let mapped = new[segment] + ratio * (new[segment + 1] - new[segment]);
        // `Number(value.toFixed(6))`.
        format!("{mapped:.6}").parse().unwrap_or(mapped)
    })
}

/// `createMappedWorkflowCandidate`: a schema-2 copy with every absolute X pin (`via` points,
/// `labelAt`, `channelX`) mapped from `old` columns to `new` ones; `None` when the mapping throws.
pub fn mapped_candidate(w: &Workflow, old: &[f64], new: &[f64]) -> Option<Workflow> {
    let map = rank_mapper(old, new)?;
    let mut out = w.clone();
    out.schema_version = SchemaVersion::V2;
    for e in &mut out.edges {
        for p in e.via.iter_mut().flatten().filter(|p| p[0].is_finite()) {
            p[0] = map(p[0]);
        }
        if let Some(p) = e.label_at.as_mut().filter(|p| p[0].is_finite()) {
            p[0] = map(p[0]);
        }
        if let Some(x) = e.channel_x.as_mut().filter(|x| x.is_finite()) {
            *x = map(*x);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mapper_is_piecewise_linear_and_extrapolates() {
        let old = LEGACY_COLUMN_CENTERS;
        let new = [94.0, 214.0, 334.0, 454.0, 574.0, 694.0];
        let map = rank_mapper(&old, &new).unwrap();
        assert_eq!(map(88.0), 94.0);
        assert_eq!(map(625.0), 694.0);
        assert_eq!(map(260.0), 274.0);
        // Before the first and after the last column, the nearest segment continues.
        assert_eq!(map(88.0 - 132.0), 94.0 - 120.0);
        assert!(rank_mapper(&old, &new[..5]).is_none());
        assert!(rank_mapper(&old, &[1.0, 1.0, 2.0, 3.0, 4.0, 5.0]).is_none());
    }
}
