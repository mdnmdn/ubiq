//! What every harness impl says the same way.
//!
//! The five provisioners differ in ~75% of their body (native config
//! serialization, model discovery, argv shape). This module holds the
//! remainder: the identity block each `impl Harness` opens with, the account
//! env-var resolution, and the conformance tests every harness owes.

/// Emit the five identity methods of [`super::Harness`].
///
/// Every harness answers these with string literals and three booleans; the
/// bodies are otherwise identical. Invoke it as the first item inside
/// `impl Harness for X`.
///
/// `acp:` is an optional trailing field defaulting to `false` — the short form
/// forwards to the long one with `acp: false`, so a harness that does not
/// speak ACP says nothing about it.
macro_rules! harness_identity {
    (
        id: $id:literal,
        display_name: $display_name:literal,
        command: $command:literal,
        aliases: [$($alias:literal),* $(,)?],
        passthrough: $passthrough:literal,
        structured: $structured:literal,
        multi_turn: $multi_turn:literal $(,)?
    ) => {
        $crate::harness::shared::harness_identity! {
            id: $id,
            display_name: $display_name,
            command: $command,
            aliases: [$($alias),*],
            passthrough: $passthrough,
            structured: $structured,
            multi_turn: $multi_turn,
            acp: false,
        }
    };
    (
        id: $id:literal,
        display_name: $display_name:literal,
        command: $command:literal,
        aliases: [$($alias:literal),* $(,)?],
        passthrough: $passthrough:literal,
        structured: $structured:literal,
        multi_turn: $multi_turn:literal,
        acp: $acp:literal $(,)?
    ) => {
        fn id(&self) -> $crate::spec::HarnessId {
            $id.to_string()
        }

        fn display_name(&self) -> &str {
            $display_name
        }

        fn command(&self) -> &str {
            $command
        }

        fn aliases(&self) -> &[&str] {
            &[$($alias),*]
        }

        fn io_support(&self) -> $crate::harness::IoSupport {
            $crate::harness::IoSupport {
                passthrough: $passthrough,
                structured: $structured,
                multi_turn: $multi_turn,
                acp: $acp,
                // Every harness that identifies itself through this macro — Copilot, opencode,
                // Grok — has a provider that publishes a plan ceiling as prose and nothing
                // queryable behind it. The macro grows a parameter when one of them gains a
                // source; until then a literal is the honest declaration.
                quota: $crate::quota::QuotaSource::None,
            }
        }
    };
}
pub(crate) use harness_identity;

/// Every harness's version/discovery probes are headless: a `--version` or `models` shell-out
/// whose only output anyone reads is its stdout. Windows would still pop a console window for one
/// — the embedding app frees its own console at boot (`ubiq_app::detach_console`), and a
/// console-subsystem child with no console to inherit gets a brand-new one — so every probe spawn
/// asks for none.
#[cfg(windows)]
pub(crate) fn no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Where a version/discovery probe's `cwd` sits: `~/.config/agent-manager/probe` — sibling to
/// `runs/`, `accounts/`, `catalog/` under [`crate::settings::default_config_dir`] — created on
/// first use. Overridable by `AM_PROBE_DIR`, mirroring `AM_RUNS`.
///
/// A probe is never handed the caller's own `cwd`. Left to inherit it, a `claude --version` or a
/// `claude -p "/model"` shell-out starts life *inside* whatever folder the embedding app happens
/// to be running from — typically the user's home directory for a GUI app — and two things follow
/// from that: on macOS, the OS treats the child's mere presence there as this process touching a
/// TCC-protected folder and prompts for access; and a harness that reads its ambient cwd (project
/// config, `.git`, `CLAUDE.md`) picks up whatever unrelated project sits there. Neither is a
/// concern for a real session: [`crate::spec::RunSpec::cwd`] is the project folder a caller
/// chose on purpose, and this function is never consulted on that path.
///
/// Falls back to the OS temp dir on the rare host with no resolvable home, and again if the
/// directory cannot be created — a `cwd` that does not exist fails the spawn itself with `ENOENT`,
/// so an unwritable config dir would take every probe down with it rather than costing a prompt.
pub(crate) fn probe_cwd() -> std::path::PathBuf {
    probe_cwd_from(std::env::var("AM_PROBE_DIR").ok().filter(|s| !s.is_empty()))
}

/// [`probe_cwd`]'s body, taking the `AM_PROBE_DIR` reading as a parameter so a test can supply one
/// without `std::env::set_var` — `unsafe` as of edition 2024, and this crate forbids unsafe code
/// (`#![forbid(unsafe_code)]` in `lib.rs`).
fn probe_cwd_from(explicit: Option<String>) -> std::path::PathBuf {
    let dir = explicit
        .map(std::path::PathBuf::from)
        .or_else(|| crate::settings::default_config_dir().map(|d| d.join("probe")))
        .unwrap_or_else(std::env::temp_dir);
    if std::fs::create_dir_all(&dir).is_ok() {
        return dir;
    }
    let fallback = std::env::temp_dir();
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// Read the secret an account's env-var *reference* names.
///
/// `am`'s account store never holds secret material — only env-var NAMES, a
/// base URL, a helper command, and/or a home dir path. The only place a
/// secret value is ever touched is the transient `std::env::var` read here;
/// it lands in `Launch.env` (in-memory, passed to the child process) and is
/// never written to disk.
pub(crate) fn account_env(account: &crate::account::Account, name: &str) -> crate::Result<String> {
    std::env::var(name).map_err(|_| {
        anyhow::anyhow!(
            "account '{}' references env var '{}' which is not set",
            account.id,
            name
        )
    })
}

/// Walk everything `provision()` wrote and fail if `secret` appears in any of
/// it — the no-secret-on-disk invariant, asserted the same way for every
/// harness.
#[cfg(test)]
pub(crate) fn assert_no_secret_on_disk(dir: &std::path::Path, secret: &str) {
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
        assert!(
            !content.contains(secret),
            "secret value leaked into {}",
            entry.path().display()
        );
    }
}

/// Emit the test items every harness's `mod tests` owes: the `write_skill`
/// fixture and the two conformance tests that differ only by harness id and
/// constructor.
#[cfg(test)]
macro_rules! harness_conformance_tests {
    ($ty:ty, $id:literal) => {
        fn write_skill(dir: &std::path::Path, id: &str) -> std::path::PathBuf {
            let skill_dir = dir.join(id);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {id}\ndescription: test skill\n---\nBody."),
            )
            .unwrap();
            skill_dir
        }

        #[test]
        fn provision_missing_skill_path_is_an_error() {
            let config_dir = tempfile::TempDir::new().unwrap();
            let mut spec =
                $crate::spec::RunSpec::new($id.to_string(), std::path::PathBuf::from("."));
            spec.config = $crate::spec::ConfigStrategy::Fixed(config_dir.path().to_path_buf());
            spec.skills.push($crate::spec::SkillRef {
                id: "missing".to_string(),
                source: $crate::source::Source::Dir(std::path::PathBuf::from(
                    "/definitely/does/not/exist/anywhere",
                )),
            });

            let harness = <$ty>::new();
            let err = harness.provision(&spec, config_dir.path()).unwrap_err();
            assert!(err.to_string().contains("missing"));
        }

        #[test]
        fn provision_account_unset_api_key_env_is_an_error_naming_the_var() {
            let config_dir = tempfile::TempDir::new().unwrap();
            let mut spec =
                $crate::spec::RunSpec::new($id.to_string(), std::path::PathBuf::from("."));
            spec.config = $crate::spec::ConfigStrategy::Fixed(config_dir.path().to_path_buf());
            spec.account = Some($crate::account::Account {
                id: "broken".to_string(),
                api_key_env: Some("__AM_DEFINITELY_UNSET_VAR__".to_string()),
                ..Default::default()
            });

            let harness = <$ty>::new();
            let err = harness.provision(&spec, config_dir.path()).unwrap_err();
            assert!(err.to_string().contains("__AM_DEFINITELY_UNSET_VAR__"));
        }
    };
}
#[cfg(test)]
pub(crate) use harness_conformance_tests;

#[cfg(test)]
mod probe_cwd_tests {
    use super::probe_cwd_from;

    /// The scratch dir is created on demand at the given root, never left for the caller to
    /// create — a probe must not fail just because nothing has run one yet.
    #[test]
    fn probe_cwd_is_created_on_demand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("does-not-exist-yet");

        let dir = probe_cwd_from(Some(root.display().to_string()));

        assert_eq!(dir, root);
        assert!(dir.is_dir(), "probe_cwd should create the scratch dir");
        assert_ne!(
            dir,
            std::env::current_dir().unwrap(),
            "probe cwd must never be the ambient process cwd"
        );
    }
}
