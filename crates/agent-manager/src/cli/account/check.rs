use super::*;
use crate::credentials::{Validity, credential_validity};

/// Current wall-clock time in epoch millis, for [`credential_validity`].
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Render a validity as a human-readable status suffix, e.g.
/// `valid (expires in 3d 4h)`, `expired`, `present (no expiry info)`,
/// `empty (no blobs stored)`.
fn describe_validity(v: Validity, now: i64) -> String {
    match v {
        Validity::Valid {
            expires_at_ms: Some(ms),
        } => format!("valid (expires in {})", human_duration_ms(ms - now)),
        Validity::Valid {
            expires_at_ms: None,
        } => "valid".to_string(),
        Validity::Expired { expires_at_ms: ms } => {
            format!("expired ({} ago)", human_duration_ms(now - ms))
        }
        Validity::Unknown => "present (no expiry info)".to_string(),
        Validity::Empty => "empty (no blobs stored)".to_string(),
    }
}

/// Format a non-negative millisecond span compactly as `Nd Nh` / `Nh Nm` /
/// `Nm` / `Ns`. Clamps negatives to `0s`.
fn human_duration_ms(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        format!("{secs}s")
    }
}

/// `am account check <name> --harness <id>` / `am account check --all`
pub(super) fn cmd_check(name: Option<&str>, harness: Option<&str>, all: bool) -> Result<()> {
    let store = build_secret_store()?;
    let now = now_ms();

    if all {
        if name.is_some() {
            bail!("give either a <name> --harness <id> or --all, not both");
        }
        let mut valid = 0usize;
        let mut expired = 0usize;
        let mut unknown = 0usize;
        for meta in store.list()? {
            let blobs = store.get(&meta.id)?.unwrap_or_default();
            let v = credential_validity(&blobs, now);
            match v {
                Validity::Valid { .. } => valid += 1,
                Validity::Expired { .. } => expired += 1,
                Validity::Unknown | Validity::Empty => unknown += 1,
            }
            println!(
                "({}, {}): {}",
                meta.id.harness,
                meta.id.name,
                describe_validity(v, now)
            );
        }
        println!("{valid} valid, {expired} expired, {unknown} unknown");
        return Ok(());
    }

    let name = name.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let harness = harness.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    match store.get(&id)? {
        None => println!("({}, {}): missing", id.harness, id.name),
        Some(blobs) => {
            let v = credential_validity(&blobs, now);
            println!(
                "({}, {}): {}",
                id.harness,
                id.name,
                describe_validity(v, now)
            );
        }
    }
    Ok(())
}

/// `am account renew <name> --harness <id>` / `am account renew --all`
pub(super) fn cmd_renew(name: Option<&str>, harness: Option<&str>, all: bool) -> Result<()> {
    let store = build_secret_store()?;

    if all {
        if name.is_some() {
            bail!("give either a <name> --harness <id> or --all, not both");
        }
        let mut renewed_ok = 0usize;
        let mut failed = 0usize;
        for meta in store.list()? {
            let id = &meta.id;
            let Some(h) = crate::harness::resolve(&id.harness) else {
                println!("({}, {}): skipped (unknown harness)", id.harness, id.name);
                failed += 1;
                continue;
            };
            let creds = store.get(id)?.unwrap_or_default();
            match h.renew_credentials(&creds).and_then(|renewed| {
                let count = renewed.len();
                store.set(id, &renewed).map(|()| count)
            }) {
                Ok(count) => {
                    renewed_ok += 1;
                    println!("({}, {}): renewed {count} blob(s)", id.harness, id.name);
                }
                Err(e) => {
                    failed += 1;
                    println!("({}, {}): failed: {e:#}", id.harness, id.name);
                }
            }
        }
        println!("{renewed_ok} renewed, {failed} failed");
        return Ok(());
    }

    let name = name.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let harness = harness.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    let creds = store.get(&id)?.unwrap_or_default();
    let renewed = h
        .renew_credentials(&creds)
        .with_context(|| format!("renewing credentials for ({}, {})", id.harness, id.name))?;
    let count = renewed.len();
    store.set(&id, &renewed)?;
    println!("renewed {count} blob(s) for ({}, {})", id.harness, id.name);
    Ok(())
}
