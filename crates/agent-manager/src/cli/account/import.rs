use super::*;

/// `am account import [--write]`: discover credential *locations* (env names
/// / file paths — never values) and, on macOS, optionally import the live
/// Claude Keychain OAuth session as account [`account::DEFAULT_ACCOUNT_ID`].
pub(super) fn cmd_import(write: bool) -> Result<()> {
    let root = account::resolve_accounts_root(None)
        .ok_or_else(|| anyhow!("could not determine the accounts root for this OS"))?;
    let settings = effective_settings();

    // --- macOS Keychain → accounts/default (primary path for bare `am claude`) ---
    let mut keychain_imported = false;
    // Track whether we stored any credential (Keychain or harness file-login),
    // so the "nothing found" branch below doesn't fire after a real import.
    let mut captured_any = false;
    match try_claude_keychain_import(&root, write, &settings) {
        Ok(KeychainImport::Written(location)) => {
            keychain_imported = true;
            captured_any = true;
            println!(
                "imported Claude Keychain → account '{}' ({location})",
                account::DEFAULT_ACCOUNT_ID,
            );
            // Always wire bare `am claude` to this account id (`default`).
            set_defaults_account(account::DEFAULT_ACCOUNT_ID, /*force*/ true)?;
        }
        Ok(KeychainImport::WouldWrite(location)) => {
            println!(
                "found macOS Keychain: {} — would materialize account id '{}' \
                 ({location}) and set [defaults].account = '{}' (pass --write)",
                account::CLAUDE_KEYCHAIN_SERVICE,
                account::DEFAULT_ACCOUNT_ID,
                account::DEFAULT_ACCOUNT_ID,
            );
        }
        Ok(KeychainImport::Unavailable(reason)) => {
            println!("Claude Keychain import skipped: {reason}");
        }
        Err(e) => {
            // Don't fail the whole import if Keychain is missing; continue
            // with path/env discovery. Print the error so the user can act.
            println!("Claude Keychain import failed: {e:#}");
        }
    }

    let mut suggestions: Vec<Account> = Vec::new();

    for env_name in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"] {
        if std::env::var(env_name).is_ok() {
            println!("found env var: {env_name}");
            suggestions.push(Account {
                id: env_name.to_lowercase().replace('_', "-"),
                api_key_env: Some(env_name.to_string()),
                ..Default::default()
            });
        }
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        let home = base_dirs.home_dir();
        // Path existence only — never read credential file contents here.
        // (Claude's usable macOS tokens live in Keychain; the on-disk
        // `.credentials.json` is often an empty stub — handled above.)
        let candidates: [(&str, PathBuf); 5] = [
            ("claude-credentials", home.join(".claude/.credentials.json")),
            ("claude-json", home.join(".claude.json")),
            ("codex-auth", home.join(".codex/auth.json")),
            (
                "opencode-auth",
                home.join(".local/share/opencode/auth.json"),
            ),
            ("copilot-config", home.join(".copilot/config.json")),
        ];

        for (label, path) in candidates {
            if !path.exists() {
                continue;
            }
            println!("found credential file ({label}): {}", path.display());
            // Skip suggesting a claude file-home when Keychain import
            // already owns `default` — pointing home at ~/.claude would
            // re-introduce the empty-stub problem under CLAUDE_CONFIG_DIR.
            if label.starts_with("claude") && keychain_imported {
                println!(
                    "  (not suggesting a separate account — Keychain `default` covers Claude)"
                );
                continue;
            }
            // Harness file-logins → capture into the credentials store as
            // `(harness, "default")`, harness-scoped (so codex/opencode/copilot
            // each get a `default`, matching the Keychain-imported claude one),
            // honoring the configured engine. Replaces the old flat
            // `{label}-home` reference suggestion for these.
            if let Some(harness_id) = harness_for_label(label) {
                if write {
                    match capture_harness_default_from_file(&settings, harness_id, &path) {
                        Ok(()) => {
                            captured_any = true;
                            println!(
                                "  → imported ({harness_id}, {}) into the credentials store",
                                account::DEFAULT_ACCOUNT_ID
                            );
                        }
                        Err(e) => println!(
                            "  ! could not import ({harness_id}, {}): {e:#}",
                            account::DEFAULT_ACCOUNT_ID
                        ),
                    }
                } else {
                    println!(
                        "  would import ({harness_id}, {}) into the credentials store (pass --write)",
                        account::DEFAULT_ACCOUNT_ID
                    );
                }
                continue;
            }
            let home_dir = path.parent().unwrap_or(&path).to_path_buf();
            suggestions.push(Account {
                id: format!("{label}-home"),
                home: Some(home_dir),
                ..Default::default()
            });
        }
    }

    if suggestions.is_empty() && !keychain_imported && !captured_any {
        // Keychain dry-run already printed; only claim "nothing found" when
        // we truly have no leads.
        if !cfg!(target_os = "macos") {
            println!("no known credential locations found");
        }
        if !write {
            println!();
            println!("(dry run — nothing written; pass --write to apply)");
        }
        return Ok(());
    }

    // Idempotency for reference-only suggestions: never append an id that
    // already exists. Keychain `default` is handled separately (refreshable).
    let existing_ids: BTreeSet<String> = if root.is_dir() {
        FsAccountStore::new(&root)
            .accounts()?
            .into_iter()
            .map(|a| a.id)
            .collect()
    } else {
        BTreeSet::new()
    };

    let (to_add, skipped) = partition_new(&existing_ids, suggestions);
    for id in &skipped {
        println!("skip (already present): {id}");
    }

    if to_add.is_empty() {
        println!();
        if keychain_imported {
            println!("no additional reference-only accounts to add.");
        } else {
            println!("all suggested accounts already present — nothing to add (idempotent).");
        }
        if !write && !keychain_imported {
            println!("(dry run — nothing written; pass --write to append to accounts.toml)");
        }
        return Ok(());
    }

    println!();
    println!("new suggested account(s) (references only; edit ids/env names as needed):");
    for acct in &to_add {
        print!("{}", account_toml_snippet(acct));
    }

    if write {
        std::fs::create_dir_all(&root)?;
        let toml_path = root.join("accounts.toml");
        let mut existing = if toml_path.exists() {
            std::fs::read_to_string(&toml_path)?
        } else {
            String::new()
        };
        for acct in &to_add {
            // Keep array-of-tables well-separated even if the prior content
            // didn't end in a newline.
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push('\n');
            existing.push_str(&account_toml_snippet(acct));
        }
        std::fs::write(&toml_path, existing)?;
        println!();
        println!(
            "appended {} new account(s) to {}",
            to_add.len(),
            toml_path.display()
        );
    } else {
        println!();
        println!("(dry run — nothing written; pass --write to append to accounts.toml)");
    }

    Ok(())
}

/// Outcome of the optional Claude Keychain import step.
enum KeychainImport {
    /// `--write`: credential stored + account record written. Carries a
    /// human-readable description of where the secret bytes landed.
    Written(String),
    /// Dry-run: Keychain is readable and would be written to `location`.
    WouldWrite(String),
    /// Not applicable (non-macOS) or no entry — not an error.
    Unavailable(String),
}

/// Probe / materialize the ambient Claude Keychain session as account
/// `default`, honoring the configured credentials engine.
///
/// Under a **secure** engine (`os`/`keychain`) the secret bytes go into the
/// [`crate::credentials::SecretStore`] and **no plaintext credential files are
/// written** — only a references-only index entry (`home = None`). Under
/// `files` the plaintext file home stays the store (legacy layout), with a
/// best-effort dual-write into the file secret store for the run path.
fn try_claude_keychain_import(
    accounts_root: &Path,
    write: bool,
    settings: &crate::settings::Settings,
) -> Result<KeychainImport> {
    if !cfg!(target_os = "macos") {
        return Ok(KeychainImport::Unavailable(
            "only supported on macOS".into(),
        ));
    }
    // Probe first so dry-run and --write share the same validity checks
    // (empty token stub → Unavailable with a clear reason).
    let creds = match account::read_claude_keychain_credentials() {
        Ok(creds) => creds,
        Err(e) => return Ok(KeychainImport::Unavailable(format!("{e:#}"))),
    };

    let engine = crate::credentials::resolve_engine(settings);
    let claude = crate::harness::resolve("claude").expect("claude harness exists");
    let id = CredentialId {
        harness: claude.id(),
        name: account::DEFAULT_ACCOUNT_ID.to_string(),
    };

    if engine != "files" {
        // Secure engine: the secret store holds the bytes; never leave
        // plaintext `.credentials.json` / `.claude.json` on disk.
        let location = format!("credentials store (engine '{engine}')");
        if !write {
            return Ok(KeychainImport::WouldWrite(location));
        }
        let seed = claude.config_anchor().login_seed;
        let blobs = blobs_from_seed(&claude_keychain_login_source(&creds)?, &seed)?;
        let store = crate::credentials::build_secret_store(settings)?;
        store.set(&id, &blobs)?; // primary write — surface failures
        account::record_default_claude_from_keychain(accounts_root, &creds)?;
        return Ok(KeychainImport::Written(location));
    }

    // Files engine: plaintext file home is the store (legacy behavior).
    let home = accounts_root.join(account::DEFAULT_ACCOUNT_ID);
    if !write {
        return Ok(KeychainImport::WouldWrite(format!(
            "home {}",
            home.display()
        )));
    }
    let acct = account::import_default_claude_from_keychain(accounts_root)?;
    let home = acct.home.unwrap_or(home);
    // Best-effort dual-write into the file secret store for the run path; the
    // file home above stays authoritative, so a store failure isn't fatal.
    let home_source = crate::source::Source::Dir(home.clone());
    let blobs = blobs_from_seed(&home_source, &claude.config_anchor().login_seed)?;
    if !blobs.is_empty()
        && let Ok(store) = build_secret_store()
    {
        let _ = store.set(&id, &blobs);
    }
    Ok(KeychainImport::Written(format!("home {}", home.display())))
}

/// A [`crate::source::Source`] holding a Claude Keychain login's bytes, keyed
/// by the paths [`crate::harness::Claude`]'s `login_seed` reads (so
/// [`blobs_from_seed`] picks them up) — without touching disk. `.claude.json`
/// (identity, non-secret) is included from the real `$HOME` when present.
fn claude_keychain_login_source(creds: &[u8]) -> Result<crate::source::Source> {
    let mut files = vec![(PathBuf::from(".claude/.credentials.json"), creds.to_vec())];
    if let Some(home) = std::env::var_os("HOME") {
        let json = PathBuf::from(home).join(".claude.json");
        if json.is_file() {
            files.push((PathBuf::from(".claude.json"), std::fs::read(&json)?));
        }
    }
    Ok(crate::source::Source::Files(files))
}

/// Map a discovery `label` (from `cmd_import`'s candidate list) to the harness
/// whose on-disk login it is, for harnesses `am account import` captures into
/// the credentials store as `(harness, "default")`. `None` for labels that
/// stay reference-only suggestions.
fn harness_for_label(label: &str) -> Option<&'static str> {
    match label {
        "codex-auth" => Some("codex"),
        "opencode-auth" => Some("opencode"),
        "copilot-config" => Some("copilot"),
        _ => None,
    }
}

/// Capture a harness's existing on-disk login file into the credentials store
/// as `(harness, "default")`, honoring the configured engine. The blob's
/// `rel_path` is the harness's `login_seed` source (what
/// [`crate::harness::seed_login`] reads back at run time), so the file's real
/// on-disk location can differ from that seed path (e.g. `~/.codex/auth.json`
/// on disk → seed `auth.json`).
fn capture_harness_default_from_file(
    settings: &crate::settings::Settings,
    harness_id: &str,
    real_file: &Path,
) -> Result<()> {
    let harness = crate::harness::resolve(harness_id)
        .ok_or_else(|| anyhow!("unknown harness '{harness_id}'"))?;
    let seed = harness.config_anchor().login_seed;
    // These harnesses each have a single login file; take the first seed slot.
    let rel = seed
        .first()
        .map(|s| s.src.clone())
        .ok_or_else(|| anyhow!("{harness_id} declares no login_seed to capture"))?;
    let bytes =
        std::fs::read(real_file).with_context(|| format!("reading {}", real_file.display()))?;
    let name = rel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel.to_string_lossy().into_owned());
    let blob = CredentialBlob {
        name,
        rel_path: rel,
        bytes,
    };
    let store = crate::credentials::build_secret_store(settings)?;
    let id = CredentialId {
        harness: harness.id(),
        name: account::DEFAULT_ACCOUNT_ID.to_string(),
    };
    store.set(&id, &[blob])
}

/// Set `[defaults].account = id` in the global settings file.
///
/// When `force` is false, only fills the key if unset (prints a note if
/// another id is already configured). When `force` is true (Keychain
/// import of the ambient session as account [`account::DEFAULT_ACCOUNT_ID`]),
/// always writes `id` so bare `am claude` uses the imported credentials.
pub(super) fn set_defaults_account(id: &str, force: bool) -> Result<()> {
    let config_path = global_config_path()?;
    let mut table: toml::Table = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow!("reading {}: {e}", config_path.display()))?;
        toml::from_str(&content).map_err(|e| anyhow!("parsing {}: {e}", config_path.display()))?
    } else {
        toml::Table::new()
    };

    let defaults = table
        .entry("defaults")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let defaults_table = defaults
        .as_table_mut()
        .ok_or_else(|| anyhow!("'defaults' in {} is not a table", config_path.display()))?;

    let existing = defaults_table
        .get("account")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    match (existing.as_deref(), force) {
        (Some(e), false) if e == id => {
            println!("[defaults].account already '{id}'");
            return Ok(());
        }
        (Some(e), false) => {
            println!(
                "note: [defaults].account is '{e}' (not overwritten); \
                 run `am account use {id}` to make bare `am claude` use account '{id}'"
            );
            return Ok(());
        }
        (Some(e), true) if e == id => {
            println!("[defaults].account already '{id}'");
            return Ok(());
        }
        (Some(e), true) => {
            println!(
                "updating [defaults].account '{e}' → '{id}' (Keychain import is the ambient default)"
            );
        }
        (None, _) => {}
    }

    defaults_table.insert("account".to_string(), toml::Value::String(id.to_string()));
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&table)?)
        .map_err(|e| anyhow!("writing {}: {e}", config_path.display()))?;
    println!(
        "set [defaults].account = '{id}' ({})",
        config_path.display()
    );
    Ok(())
}

/// Split `suggestions` into `(to_add, skipped_ids)`: entries whose id is not
/// already in `existing_ids`, de-duplicated by id within the batch (order
/// preserved), plus the ids skipped as already-present or intra-batch
/// duplicates. Pure — the idempotency core of `am account import`, unit-tested
/// without touching the filesystem.
pub(super) fn partition_new(
    existing_ids: &BTreeSet<String>,
    suggestions: Vec<Account>,
) -> (Vec<Account>, Vec<String>) {
    let mut to_add = Vec::new();
    let mut skipped = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for acct in suggestions {
        if existing_ids.contains(&acct.id) || !seen.insert(acct.id.clone()) {
            skipped.push(acct.id);
            continue;
        }
        to_add.push(acct);
    }
    (to_add, skipped)
}
