//! Sync engine: project a [`UnifiedConfig`](crate::config::UnifiedConfig) onto
//! one or more concrete harnesses.

use anyhow::Result;

use crate::config::UnifiedConfig;
use crate::harness::Harness;

/// Result of a sync attempt against a single harness.
#[derive(Debug, Clone)]
pub struct SyncReport {
    /// Harness id.
    pub harness: String,
    /// Files written.
    pub written: Vec<String>,
    /// Files skipped (e.g. feature not supported by this harness).
    pub skipped: Vec<String>,
}

/// Sync the given config to every harness in `targets` (or all known harnesses
/// if `targets` is empty).
pub fn sync(_config: &UnifiedConfig, targets: &[String]) -> Result<Vec<SyncReport>> {
    let harnesses: Vec<Harness> = if targets.is_empty() {
        Harness::all()
    } else {
        targets
            .iter()
            .filter_map(|id| Harness::by_id(id))
            .collect()
    };

    let mut reports = Vec::with_capacity(harnesses.len());
    for h in harnesses {
        reports.push(SyncReport {
            harness: h.id.clone(),
            written: Vec::new(),
            skipped: Vec::new(),
        });
    }
    Ok(reports)
}
