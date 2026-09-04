use super::*;

/// The validity of a stored credential, as computed by [`credential_validity`]
/// from any embedded expiry field. Harness-agnostic (it just looks for a
/// numeric `*expire*` key anywhere in the parsed JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Validity {
    /// An expiry was found and is still in the future (or no expiry field was
    /// found but blobs are present — see [`credential_validity`], which uses
    /// [`Validity::Unknown`] for that case, so `expires_at_ms` here is always
    /// `Some`). Kept as `Option` to leave room for future "valid, no expiry".
    Valid {
        /// Epoch-millis expiry, if one was found.
        expires_at_ms: Option<i64>,
    },
    /// An expiry was found and is at/before `now_ms`.
    Expired {
        /// Epoch-millis expiry that has passed.
        expires_at_ms: i64,
    },
    /// Blobs are present but carry no recognizable expiry field.
    Unknown,
    /// No blobs are stored for this credential.
    Empty,
}

/// Compute a stored credential's [`Validity`] at `now_ms` (epoch millis).
///
/// Harness-agnostic but Claude-aware: for each blob, parse the bytes as JSON
/// and recursively search for a numeric field whose key case-insensitively
/// contains `"expire"` (covers Claude's `claudeAiOauth.expiresAt`, which is
/// epoch **millis**). Each value is normalized — numbers below `10^12` are
/// treated as **seconds** and scaled to millis — and the **maximum** expiry
/// across all blobs wins. Pure (takes `now_ms`) so it's unit-testable without
/// a clock. Returns [`Validity::Empty`] for no blobs, [`Validity::Unknown`]
/// for blobs with no expiry field, else `Valid`/`Expired`.
pub(super) fn credential_validity(blobs: &[CredentialBlob], now_ms: i64) -> Validity {
    if blobs.is_empty() {
        return Validity::Empty;
    }
    let mut max_expiry: Option<i64> = None;
    for blob in blobs {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&blob.bytes)
            && let Some(ms) = max_expiry_ms(&v)
        {
            max_expiry = Some(max_expiry.map_or(ms, |cur| cur.max(ms)));
        }
    }
    match max_expiry {
        Some(ms) if ms > now_ms => Validity::Valid {
            expires_at_ms: Some(ms),
        },
        Some(ms) => Validity::Expired { expires_at_ms: ms },
        None => Validity::Unknown,
    }
}

/// Recursively find the maximum numeric `*expire*` value in `v`, normalized to
/// epoch millis (values below `10^12` are treated as seconds and ×1000).
fn max_expiry_ms(v: &serde_json::Value) -> Option<i64> {
    fn normalize(n: i64) -> i64 {
        if n < 1_000_000_000_000 {
            n.saturating_mul(1000)
        } else {
            n
        }
    }
    match v {
        serde_json::Value::Object(map) => {
            let mut best: Option<i64> = None;
            for (k, val) in map {
                if k.to_lowercase().contains("expire")
                    && let Some(n) = val.as_i64().or_else(|| val.as_f64().map(|f| f as i64))
                {
                    let ms = normalize(n);
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
                if let Some(ms) = max_expiry_ms(val) {
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
            }
            best
        }
        serde_json::Value::Array(items) => {
            let mut best: Option<i64> = None;
            for item in items {
                if let Some(ms) = max_expiry_ms(item) {
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
            }
            best
        }
        _ => None,
    }
}

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
