//! `am agent <name>` — run a profile as a **frozen** agent.
//!
//! An agent is a profile whose composition is pinned: `am agent <name>` resolves
//! the (flattened) profile `<name>`, requires it to name a `harness`, and
//! launches that harness with the profile applied and **no** `am` composition
//! flags of its own — only harness passthrough args after `--`. Everything
//! reuses the normal resolve → provision → run spine via
//! [`super::run::run_harness`]; see `_docs/profiles.md` §10.

use anyhow::{bail, Context, Result};

use crate::harness;
use crate::profile::{resolve_profiles_root, EmptyProfileStore, FsProfileStore, ProfileStore};

/// Build the profile store from the default profiles root (honors
/// `AM_PROFILES` / the default location), mirroring `cli/run.rs` and
/// `cli/profile.rs`.
fn build_store() -> Box<dyn ProfileStore> {
    match resolve_profiles_root(None) {
        Some(root) if root.is_dir() => Box::new(FsProfileStore::new(root)),
        _ => Box::new(EmptyProfileStore),
    }
}

/// `am agent <name> [-- <harness-args>…]`.
pub fn run(args: &[String]) -> Result<()> {
    let name = match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            println!(
                "USAGE:\n    am agent <name> [-- <harness-args>…]   run a profile as a frozen agent"
            );
            return Ok(());
        }
        Some(name) => name,
    };

    let store = build_store();
    // Flatten so an inherited `harness` pin (from an `extends` parent) counts.
    let profile = crate::profile::resolve_flattened(store.as_ref(), name)
        .with_context(|| format!("resolving agent '{name}'"))?;

    let Some(harness_id) = profile.harness.clone() else {
        bail!("agent '{name}' has no `harness` pin; an agent profile must name its harness");
    };
    let Some(h) = harness::resolve(&harness_id) else {
        bail!("agent '{name}' names unknown harness '{harness_id}'");
    };

    // Reconstruct argv for `run_harness`: only `--profile <name>`, plus any
    // passthrough after the first `--`. No composition flags are offered — that
    // is what makes the agent "frozen".
    let mut forwarded: Vec<String> = vec!["--profile".to_string(), name.to_string()];
    if let Some(pos) = args.iter().position(|a| a == "--") {
        forwarded.push("--".to_string());
        forwarded.extend(args[pos + 1..].iter().cloned());
    }
    super::run::run_harness(h.as_ref(), &forwarded)
}
