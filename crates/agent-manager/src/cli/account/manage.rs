use super::*;

/// `am account delete <name> --harness <id> [--yes]`
pub(super) fn cmd_delete(name: &str, harness: &str, yes: bool) -> Result<()> {
    let h = resolve_harness(harness)?;
    let default_account = configured_default_account()?;
    if default_account.as_deref() == Some(name) && !yes {
        bail!("refusing to delete the default account '{name}' without --yes");
    }

    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    let store = build_secret_store()?;
    store.delete(&id)?;
    println!(
        "deleted stored credential ({}, {}) — the account index entry, if any, is untouched",
        id.harness, id.name
    );
    Ok(())
}

/// `am account rename <old> <new> --harness <id>`
pub(super) fn cmd_rename(old: &str, new: &str, harness: &str) -> Result<()> {
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: old.to_string(),
    };
    let store = build_secret_store()?;
    store.rename(&id, new)?;
    println!("renamed credential ({}, {old}) -> {new}", id.harness);

    if configured_default_account()?.as_deref() == Some(old) {
        set_defaults_account(new, true)?;
    }
    Ok(())
}

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
    if acct.home.is_some() {
        parts.push("home");
    }
    if parts.is_empty() {
        "(no references set)".to_string()
    } else {
        format!("({})", parts.join(", "))
    }
}

/// `am account ls`
pub(super) fn cmd_list() -> Result<()> {
    // Harness-scoped credentials in the secret store (the primary view — one
    // row per `(harness, name)`, so each harness's `default` shows up). Best
    // effort: if no store is configured, just skip this section.
    let creds = crate::credentials::build_secret_store(&effective_settings())
        .and_then(|s| s.list())
        .unwrap_or_default();

    // Reference-only accounts (env keys, base URLs, legacy file homes).
    let store = build_store();
    let accounts = store.accounts()?;

    if creds.is_empty() && accounts.is_empty() {
        println!("no accounts configured");
        return Ok(());
    }

    if !creds.is_empty() {
        println!("stored credentials:");
        for m in &creds {
            println!(
                "  {:<14} {:<10} [engine: {}]",
                m.id.harness, m.id.name, m.engine
            );
        }
    }

    if !accounts.is_empty() {
        if !creds.is_empty() {
            println!();
        }
        println!("accounts (references):");
        for acct in accounts {
            let mut line = format!("  {}  {}", acct.id, describe_refs(&acct));
            if let Some(home) = &acct.home {
                let captured = effective_harnesses(home);
                if !captured.is_empty() {
                    line.push_str(&format!("  [captured: {}]", captured.join(", ")));
                }
            }
            println!("{line}");
        }
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

/// Render an [`Account`] as an inline `[[account]]` TOML snippet.
pub(super) fn account_toml_snippet(acct: &Account) -> String {
    let mut s = String::new();
    s.push_str("[[account]]\n");
    s.push_str(&format!("id = \"{}\"\n", acct.id));
    if let Some(v) = &acct.api_key_env {
        s.push_str(&format!("api_key_env = \"{v}\"\n"));
    }
    if let Some(v) = &acct.auth_token_env {
        s.push_str(&format!("auth_token_env = \"{v}\"\n"));
    }
    if let Some(v) = &acct.base_url {
        s.push_str(&format!("base_url = \"{v}\"\n"));
    }
    if let Some(v) = &acct.helper {
        s.push_str(&format!("helper = \"{v}\"\n"));
    }
    if let Some(v) = &acct.home {
        s.push_str(&format!("home = \"{}\"\n", v.display()));
    }
    s
}
