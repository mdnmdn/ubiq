use super::*;

/// Describe which reference fields an account carries, e.g. `(api_key_env, base_url)`.
fn describe_refs(acct: &Account) -> String {
    let mut parts = Vec::new();
    if acct.api_key_env.is_some() {
        parts.push("api_key_env");
    }
    if acct.auth_token_env.is_some() {
        parts.push("auth_token_env");
    }
    if acct.base_url.is_some() {
        parts.push("base_url");
    }
    if acct.helper.is_some() {
        parts.push("helper");
    }
    if parts.is_empty() {
        "(no references set)".to_string()
    } else {
        format!("({})", parts.join(", "))
    }
}

/// `am account ls`
pub(super) fn cmd_list() -> Result<()> {
    let accounts = build_store().accounts()?;
    if accounts.is_empty() {
        println!("no accounts configured");
        return Ok(());
    }
    println!("accounts (references):");
    for acct in accounts {
        println!("  {}  {}", acct.id, describe_refs(&acct));
    }
    Ok(())
}

/// `am account use <id>`: set `[defaults].account` in the global settings file.
pub(super) fn cmd_use(id: &str) -> Result<()> {
    let store = build_store();
    if store.account(id)?.is_none() {
        let available: Vec<String> = store.accounts()?.into_iter().map(|a| a.id).collect();
        let listing = if available.is_empty() {
            "(none configured)".to_string()
        } else {
            available.join(", ")
        };
        bail!("unknown account id '{id}'; available: {listing}");
    }

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
    defaults_table.insert("account".to_string(), toml::Value::String(id.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&table)?)
        .map_err(|e| anyhow!("writing {}: {e}", config_path.display()))?;

    println!("default account set to '{id}' ({})", config_path.display());
    Ok(())
}

/// Resolve `harness_key` to a harness that can run from a shared config home, and this
/// account's home for it. The path is computed, not created.
fn account_home(
    id: &str,
    harness_key: &str,
) -> Result<(Box<dyn crate::harness::Harness>, PathBuf)> {
    let harness = crate::harness::resolve(harness_key).ok_or_else(|| {
        anyhow!(
            "unknown harness '{harness_key}'; known: {}",
            crate::harness::known_ids().join(", ")
        )
    })?;
    if !harness.shares_home() {
        bail!(
            "harness '{}' does not run from a shared config home",
            harness.id()
        );
    }
    let home = build_homes()?
        .home(id, &harness.id())
        .ok_or_else(|| anyhow!("'{id}' is not a usable account name"))?;
    Ok((harness, home))
}

/// `am account login <id> --harness <h>`: prepare this account's config home for the harness
/// ([`crate::provision::prepare_home`]) and run the harness's own login into it, interactively
/// (`D194`).
///
/// The login stays in the home, where every run naming this account reads it and the harness
/// refreshes it; nothing is captured or read back. A first terminal run's own login screen
/// reaches the same place, so this is the explicit route, not the only one.
pub(super) fn cmd_login(id: &str, harness_key: &str) -> Result<()> {
    let (harness, home) = account_home(id, harness_key)?;
    let templates = crate::harness::FsTemplateStore::from_default();
    crate::provision::prepare_home(harness.as_ref(), &home, &templates)?;
    let launch = harness.login_home(&home)?;
    let provisioned = crate::provision::Provisioned {
        dir: home.clone(),
        launch,
        ephemeral: false, // the account's home — never removed by a run
        home: Some(home.clone()),
        resume: None,
        model: None,
        fork: false,
        mcp_servers: Vec::new(),
        #[cfg(feature = "inproc-mcp")]
        inproc_servers: Vec::new(),
    };
    let cwd = std::env::current_dir()?;
    let code = crate::run::run(&provisioned, &cwd, true, None)?;
    if code != 0 {
        bail!("harness login exited with code {code}");
    }
    println!(
        "account '{id}' signed in to {} ({})",
        harness.id(),
        home.display()
    );
    Ok(())
}

/// `am account logout <id> --harness <h>`: remove this account's config home for that harness —
/// the sign-out. Removing one that is not there is success.
pub(super) fn cmd_logout(id: &str, harness_key: &str) -> Result<()> {
    let (harness, home) = account_home(id, harness_key)?;
    build_homes()?.forget(id, &harness.id())?;
    println!(
        "account '{id}' signed out of {} ({})",
        harness.id(),
        home.display()
    );
    Ok(())
}

/// `am account check <id> --harness <h>`: say whether that home holds a login and what expiry
/// the login states about itself ([`crate::home::login_validity`]).
///
/// An absent file is not proof of no login — on macOS a harness may keep it in the Keychain.
pub(super) fn cmd_check(id: &str, harness_key: &str) -> Result<()> {
    let (harness, home) = account_home(id, harness_key)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let line = match crate::home::login_validity(harness.as_ref(), &home, now_ms) {
        crate::Validity::Valid { expires_at_ms } => match expires_at_ms {
            Some(ms) => format!("signed in (expires at {ms} epoch-ms)"),
            None => "signed in".to_string(),
        },
        crate::Validity::Expired { expires_at_ms } => {
            format!("expired (at {expires_at_ms} epoch-ms)")
        }
        crate::Validity::Unknown => "signed in (the login states no expiry)".to_string(),
        crate::Validity::Empty => {
            "no login file here (the harness may keep one in the OS keychain)".to_string()
        }
    };
    println!("{id} / {}: {line}  ({})", harness.id(), home.display());
    Ok(())
}
