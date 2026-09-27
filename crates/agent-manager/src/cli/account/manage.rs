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
