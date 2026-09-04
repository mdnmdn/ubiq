//! What every harness impl says the same way.
//!
//! The five provisioners differ in ~75% of their body (native config
//! serialization, model discovery, argv shape). This module holds the
//! remainder: the identity block each `impl Harness` opens with, the account
//! env-var resolution, and the conformance tests every harness owes.

/// Emit the five identity methods of [`super::Harness`].
///
/// Every harness answers these with string literals and two booleans; the
/// bodies are otherwise identical. Invoke it as the first item inside
/// `impl Harness for X`.
macro_rules! harness_identity {
    (
        id: $id:literal,
        display_name: $display_name:literal,
        command: $command:literal,
        aliases: [$($alias:literal),* $(,)?],
        passthrough: $passthrough:literal,
        structured: $structured:literal $(,)?
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
            }
        }
    };
}
pub(crate) use harness_identity;

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
