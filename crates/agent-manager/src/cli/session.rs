//! `am session` subcommands: `ls`, `show`, `resume`.
//!
//! Presentation layer over [`crate::session`]: this module owns argument
//! parsing and printing only, never the on-disk shape. `resume` re-launches a
//! prior run so the harness continues its own conversation, using the
//! session's retained config dir + the harness's native resume flag (see
//! [`rebuild_spec`]); the actual provision/run dispatch is shared with
//! `am <harness>` via `crate::cli::run::run_provisioned`.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use crate::session::{self, SessionMeta};
use crate::spec::{ConfigStrategy, IoModes, RunSpec};

/// `am session` subcommand dispatcher.
#[derive(Debug, Parser)]
#[command(name = "am-session", disable_help_flag = false)]
struct SessionArgs {
    #[command(subcommand)]
    command: SessionCommand,
}

/// Subcommands for `am session`.
#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// List recorded sessions, newest first.
    #[command(name = "ls")]
    List,
    /// Show one session's metadata and transcript.
    Show {
        /// Session id (as printed by `am session ls`).
        id: String,
    },
    /// Resume a previous session.
    Resume {
        /// Session id to resume (as printed by `am session ls`).
        id: String,
    },
}

/// Run a session subcommand, given argv AFTER the `session` word.
pub(super) fn run(args: &[String]) -> Result<()> {
    // If no args, default to 'ls'.
    let args = if args.is_empty() {
        vec!["ls".to_string()]
    } else {
        args.to_vec()
    };

    let args = SessionArgs::try_parse_from(
        std::iter::once("am-session".to_string()).chain(args.iter().cloned()),
    )?;

    match args.command {
        SessionCommand::List => cmd_list(),
        SessionCommand::Show { id } => cmd_show(&id),
        SessionCommand::Resume { id } => cmd_resume(&id),
    }
}

/// Resolve the sessions root the same way `am <harness>` recorded into
/// (`AM_SESSIONS` / the default state dir).
fn sessions_root() -> Option<std::path::PathBuf> {
    session::sessions_root(None)
}

/// `am session ls`
fn cmd_list() -> Result<()> {
    let Some(root) = sessions_root() else {
        println!("no sessions recorded");
        return Ok(());
    };

    let sessions = session::list(&root)?;
    if sessions.is_empty() {
        println!("no sessions recorded");
        return Ok(());
    }

    for meta in sessions {
        println!("{}", format_summary_line(&meta));
    }

    Ok(())
}

/// `am session show <id>`
fn cmd_show(id: &str) -> Result<()> {
    let Some(root) = sessions_root() else {
        bail!("no sessions root configured for this OS");
    };

    let meta = session::load(&root, id)?;

    println!("id:                  {}", meta.id);
    println!("harness:             {}", meta.harness);
    println!("cwd:                 {}", meta.cwd.display());
    println!("argv:                {}", meta.argv.join(" "));
    println!(
        "account:             {}",
        meta.account.as_deref().unwrap_or("-")
    );
    println!("io:                  {}", meta.io);
    println!("config dir:          {}", meta.config_dir.display());
    println!("created at:          {}", format_millis(meta.created_at));
    println!(
        "finished at:         {}",
        meta.finished_at.map(format_millis).unwrap_or_else(|| "-".to_string())
    );
    println!(
        "exit code:           {}",
        meta.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "harness session id:  {}",
        meta.harness_session_id.as_deref().unwrap_or("-")
    );

    let transcript_path = session::transcript_path(&root, id);
    if !transcript_path.exists() {
        println!();
        println!("(no transcript recorded — passthrough run)");
        return Ok(());
    }

    let events = session::read_transcript(&root, id)?;
    println!();
    println!("transcript: {} event(s)", events.len());
    if let Some(first) = events.first() {
        println!("  first: {}", serde_json::to_string(first)?);
    }
    if events.len() > 1
        && let Some(last) = events.last()
    {
        println!("  last:  {}", serde_json::to_string(last)?);
    }

    Ok(())
}

/// `am session resume <id>`: reconstruct a [`RunSpec`] from the recorded
/// session (see [`rebuild_spec`]), re-provision it (writes into the
/// session's own retained config dir — see [`ConfigStrategy::Fixed`]), and
/// run it through the same provision-is-done tail `am <harness>` uses
/// (`crate::cli::run::run_provisioned`), so a resumed run is recorded as its
/// own new session just like any other.
fn cmd_resume(id: &str) -> Result<()> {
    let Some(root) = sessions_root() else {
        bail!("no sessions root configured for this OS");
    };

    let meta = session::load(&root, id)?;
    let spec = rebuild_spec(&meta)?;

    let Some(harness) = crate::harness::resolve(&meta.harness) else {
        bail!(
            "unknown harness '{}' recorded in session '{id}'; known: {}",
            meta.harness,
            crate::harness::known_ids().join(", ")
        );
    };

    let templates = crate::harness::FsTemplateStore::from_default();
    let provisioned = crate::provision::provision(harness.as_ref(), &spec, &templates)?;
    let new_sessions_root = session::sessions_root(None);

    // `output` only matters for `--io structured`; there's no `--output`
    // flag on `am session resume` (yet), so this always uses the raw
    // `AgentEvent` NDJSON projection. `keep_config = true`: a resumed run's
    // config dir is `Fixed` already (never auto-deleted regardless — see
    // `run::cleanup`'s `ephemeral` check), but passing `true` makes the
    // intent explicit.
    super::run::run_provisioned(
        harness.as_ref(),
        &spec,
        &provisioned,
        &meta.cwd,
        super::OutputMode::Events,
        new_sessions_root,
        true,
    )
}

/// Reconstruct a minimal [`RunSpec`] to resume `meta`'s harness-native
/// session: `config = Fixed(meta.config_dir)` (so re-provisioning writes
/// into — and the launch points at — the dir the original run already
/// populated), `resume = meta.harness_session_id` (the harness-native id the
/// provisioner turns into its native resume flag), and the same `io` mode
/// the original run used.
///
/// Per-run mcps/skills/hooks/account from the original run are NOT
/// re-applied: resume restores the *conversation* via the retained config
/// dir (which still holds the original run's mcp.json/settings.json/skills
/// on disk), not the full original `RunSpec` — re-injecting tools from
/// scratch would need that original spec, and `SessionMeta` doesn't persist
/// it (see `src/session.rs`).
///
/// Pure aside from a `Path::exists` check — no provisioning, no launch — so
/// it's fully unit-testable without spawning a real harness.
fn rebuild_spec(meta: &SessionMeta) -> Result<RunSpec> {
    let Some(harness_session_id) = meta.harness_session_id.clone() else {
        bail!(
            "session '{}' has no recorded harness session id (only structured runs capture one); cannot resume",
            meta.id
        );
    };

    if !meta.config_dir.exists() {
        bail!(
            "config dir for session '{}' was not retained ({}); cannot resume",
            meta.id,
            meta.config_dir.display()
        );
    }

    let mut spec = RunSpec::new(meta.harness.clone(), meta.cwd.clone());
    spec.config = ConfigStrategy::Fixed(meta.config_dir.clone());
    spec.io = match meta.io.as_str() {
        "structured" => IoModes::Structured,
        _ => IoModes::Passthrough,
    };
    spec.resume = Some(harness_session_id);

    Ok(spec)
}

/// One `am session ls` line: id, harness, io, created time, exit code.
fn format_summary_line(meta: &SessionMeta) -> String {
    let exit = meta
        .exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "{}  {:<12}  {:<11}  {}  exit={}",
        meta.id,
        meta.harness,
        meta.io,
        format_millis(meta.created_at),
        exit
    )
}

/// Render a unix-millis timestamp as a compact human-readable form. Kept
/// dependency-free (no `chrono`/`time` crate) — good enough for `ls`/`show`;
/// callers wanting real date math should read `meta.json` directly.
fn format_millis(millis: u64) -> String {
    let secs = millis / 1000;
    // Days since epoch + time-of-day, using plain arithmetic — accurate
    // (proleptic Gregorian, UTC) without pulling in a date/time dependency.
    let days_since_epoch = secs / 86_400;
    let secs_of_day = secs % 86_400;
    let (h, m, s) = (secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60);

    let (y, mo, d) = civil_from_days(days_since_epoch as i64);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`: convert a day count since
/// 1970-01-01 into a proleptic-Gregorian (year, month, day). Public-domain
/// algorithm, chosen to avoid a date/time dependency for a display-only
/// timestamp.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn format_millis_renders_a_known_timestamp() {
        // 2024-01-02T03:04:05Z
        assert_eq!(format_millis(1_704_164_645_000), "2024-01-02 03:04:05Z");
    }

    #[test]
    fn format_millis_renders_the_epoch() {
        assert_eq!(format_millis(0), "1970-01-01 00:00:00Z");
    }

    fn sample_meta(config_dir: PathBuf) -> SessionMeta {
        SessionMeta {
            id: "111-222".to_string(),
            harness: "claude-code".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            argv: vec!["claude".to_string()],
            account: None,
            io: "structured".to_string(),
            config_dir,
            created_at: 0,
            finished_at: Some(1),
            exit_code: Some(0),
            harness_session_id: Some("harness-abc".to_string()),
        }
    }

    #[test]
    fn rebuild_spec_sets_fixed_config_resume_and_io() {
        let temp = tempfile::TempDir::new().unwrap();
        let meta = sample_meta(temp.path().to_path_buf());

        let spec = rebuild_spec(&meta).unwrap();

        assert_eq!(spec.config, ConfigStrategy::Fixed(temp.path().to_path_buf()));
        assert_eq!(spec.resume.as_deref(), Some("harness-abc"));
        assert_eq!(spec.io, IoModes::Structured);
        assert_eq!(spec.harness, "claude-code");
        assert_eq!(spec.cwd, PathBuf::from("/tmp/project"));
    }

    #[test]
    fn rebuild_spec_passthrough_io_round_trips() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut meta = sample_meta(temp.path().to_path_buf());
        meta.io = "passthrough".to_string();

        let spec = rebuild_spec(&meta).unwrap();
        assert_eq!(spec.io, IoModes::Passthrough);
    }

    #[test]
    fn rebuild_spec_missing_harness_session_id_is_an_error() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut meta = sample_meta(temp.path().to_path_buf());
        meta.harness_session_id = None;

        let err = rebuild_spec(&meta).unwrap_err();
        assert!(err.to_string().contains("no recorded harness session id"));
    }

    #[test]
    fn rebuild_spec_missing_config_dir_is_an_error() {
        let meta = sample_meta(PathBuf::from("/definitely/does/not/exist/anywhere"));

        let err = rebuild_spec(&meta).unwrap_err();
        assert!(err.to_string().contains("was not retained"));
    }
}
