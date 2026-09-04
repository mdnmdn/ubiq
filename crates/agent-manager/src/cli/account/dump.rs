use super::*;

/// `am account dump <name> --harness <id> [--json] [--show-secrets] [--path <p>]`
pub(super) fn cmd_dump(
    name: &str,
    harness: &str,
    json: bool,
    show_secrets: bool,
    path_filter: Option<&Path>,
) -> Result<()> {
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    let store = build_secret_store()?;
    let all_blobs = store
        .get(&id)?
        .ok_or_else(|| anyhow!("no stored credential ({}, {})", id.harness, id.name))?;

    let blobs: Vec<CredentialBlob> = all_blobs
        .into_iter()
        .filter(|b| path_filter.is_none_or(|p| b.rel_path == p))
        .collect();
    if blobs.is_empty() {
        if let Some(p) = path_filter {
            bail!(
                "no blob at path {} for ({}, {})",
                p.display(),
                id.harness,
                id.name
            );
        }
        println!("(no blobs)");
        return Ok(());
    }

    if show_secrets {
        let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
        let env_allow = std::env::var("AM_ALLOW_SECRET_DUMP").as_deref() == Ok("1");
        if !secret_dump_allowed(show_secrets, is_tty, env_allow) {
            bail!(
                "refusing to print secrets outside a TTY; re-run interactively or set \
                 AM_ALLOW_SECRET_DUMP=1"
            );
        }
    }

    if json {
        let entries: Vec<serde_json::Value> = blobs
            .iter()
            .map(|b| {
                serde_json::json!({
                    "rel_path": b.rel_path.to_string_lossy(),
                    "bytes_utf8": dump_display_text(b, show_secrets),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    for blob in &blobs {
        println!("{}", blob.rel_path.display());
        println!("{}", dump_display_text(blob, show_secrets));
    }
    Ok(())
}

/// Render one blob's displayable text for `am account dump`: raw UTF-8
/// (lossy) when `show_secrets`; otherwise a `<present, N bytes>` placeholder,
/// followed by a redacted pretty-JSON rendering when the bytes parse as JSON.
fn dump_display_text(blob: &CredentialBlob, show_secrets: bool) -> String {
    if show_secrets {
        return String::from_utf8_lossy(&blob.bytes).into_owned();
    }
    let mut out = format!("<present, {} bytes>", blob.bytes.len());
    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&blob.bytes) {
        redact_json(&mut v);
        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            out.push('\n');
            out.push_str(&pretty);
        }
    }
    out
}

/// Recursively replace object string values whose key case-insensitively
/// contains `token`, `secret`, `key`, `password`, or `auth` with
/// `"<redacted:LEN>"` (`LEN` = the original string's byte length). Non-string
/// values under a matching key, and every value under a non-matching key, are
/// walked/left as-is. Pure — the redaction core of `am account dump`, unit
/// tested without any store or I/O.
pub(super) fn redact_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            let secret_like = ["token", "secret", "key", "password", "auth"];
            for (k, val) in map.iter_mut() {
                let key_matches = secret_like.iter().any(|pat| k.to_lowercase().contains(pat));
                if key_matches && let serde_json::Value::String(s) = val {
                    *val = serde_json::Value::String(format!("<redacted:{}>", s.len()));
                } else {
                    redact_json(val);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_json(item);
            }
        }
        _ => {}
    }
}

/// Gate for `am account dump --show-secrets`: only allowed on a real TTY (so
/// secrets can't land silently in a captured log/pipe) or when the caller
/// explicitly opts in via `AM_ALLOW_SECRET_DUMP=1`. Pure — factored out of
/// [`cmd_dump`] so the gating logic is unit-testable without a real terminal.
pub(super) fn secret_dump_allowed(show_secrets: bool, is_tty: bool, env_allow: bool) -> bool {
    !show_secrets || is_tty || env_allow
}
