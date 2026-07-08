//! Claude Code provisioner.
//!
//! Transcribes `_docs/harness/claude-code.md` (esp. "Orchestration / headless
//! invocation", "MCP at launch", "Skills at launch") into a [`Harness`] impl.
//!
//! The "custom config folder" bridge: Claude Code's user config dir can be
//! relocated with the `CLAUDE_CONFIG_DIR` environment variable. Provisioning
//! points that variable at the ephemeral dir instead of the real `~/.claude`,
//! so skills/settings/memory are injected without ever touching the user's
//! real config.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::config::{McpServer, McpTransport};
use crate::spec::{McpRef, RunSpec};
use crate::Result;

use super::{Harness, IoSupport, Launch};

/// Environment variables stripped from the child so a nested `am`/Claude Code
/// invocation doesn't inherit the parent session's identity.
const ENV_HYGIENE: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SSE_PORT",
];

/// The marker that wraps `am`-managed content in `CLAUDE.md`, so user-authored
/// content (were any to coexist in the file) is distinguishable from ours.
const MANAGED_BEGIN: &str = "<!-- agent-manager:begin -->";
const MANAGED_END: &str = "<!-- agent-manager:end -->";

/// The Claude Code harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Claude;

impl Claude {
    /// Construct the Claude Code harness descriptor.
    pub fn new() -> Self {
        Claude
    }
}

impl Harness for Claude {
    fn id(&self) -> crate::spec::HarnessId {
        "claude-code".to_string()
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    fn command(&self) -> &str {
        "claude"
    }

    fn aliases(&self) -> &[&str] {
        &["claude"]
    }

    fn io_support(&self) -> IoSupport {
        IoSupport {
            passthrough: true,
            jsonl: true,
            acp: false,
        }
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // 1. Skills: copy each skill folder into <dir>/skills/<id>/.
        let skills_dir = dir.join("skills");
        for skill in &spec.skills {
            if !skill.path.exists() {
                bail!(
                    "skill '{}' points at a path that does not exist: {}",
                    skill.id,
                    skill.path.display()
                );
            }
            let dest = skills_dir.join(&skill.id);
            copy_dir_recursive(&skill.path, &dest).with_context(|| {
                format!(
                    "copying skill '{}' from {} to {}",
                    skill.id,
                    skill.path.display(),
                    dest.display()
                )
            })?;
        }

        // 2. MCP: always write <dir>/mcp.json, even if empty, so
        // --strict-mcp-config yields a fully-controlled server set.
        let mcp_json = build_mcp_json(&spec.mcps)?;
        let mcp_path = dir.join("mcp.json");
        std::fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_json)?)
            .with_context(|| format!("writing {}", mcp_path.display()))?;

        // 3. Policy: <dir>/settings.json, only if a policy is set.
        if let Some(policy) = &spec.policy {
            let mut permissions = serde_json::Map::new();
            if let Some(mode) = &policy.permission_mode {
                permissions.insert("defaultMode".to_string(), json!(mode));
            }
            permissions.insert("allow".to_string(), json!(policy.allow));
            permissions.insert("ask".to_string(), json!(policy.ask));
            permissions.insert("deny".to_string(), json!(policy.deny));

            let settings = json!({ "permissions": Value::Object(permissions) });
            let settings_path = dir.join("settings.json");
            std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
                .with_context(|| format!("writing {}", settings_path.display()))?;
        }

        // 4. Instructions: <dir>/CLAUDE.md, wrapped in a managed block.
        if let Some(instr) = &spec.initial {
            let claude_md = format!("{MANAGED_BEGIN}\n{}\n{MANAGED_END}\n", instr.text);
            let claude_md_path = dir.join("CLAUDE.md");
            std::fs::write(&claude_md_path, claude_md)
                .with_context(|| format!("writing {}", claude_md_path.display()))?;
        }

        // 5. Build the launch.
        let mut args = vec![
            "--mcp-config".to_string(),
            mcp_path.display().to_string(),
            "--strict-mcp-config".to_string(),
        ];
        args.extend(spec.passthrough_args.iter().cloned());

        Ok(Launch {
            program: "claude".to_string(),
            args,
            env: vec![("CLAUDE_CONFIG_DIR".to_string(), dir.display().to_string())],
            env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
        })
    }
}

/// Recursively copy `src` into `dst`, creating directories as needed.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Render one [`McpServer`] into the JSON shape Claude Code's `--mcp-config`
/// file expects, keyed by transport.
fn server_json(server: &McpServer) -> Value {
    match server.transport {
        McpTransport::Stdio => {
            let mut obj = serde_json::Map::new();
            if let Some(command) = &server.command {
                obj.insert("command".to_string(), json!(command));
            }
            obj.insert("args".to_string(), json!(server.args));
            if !server.env.is_empty() {
                obj.insert("env".to_string(), json!(server.env));
            }
            Value::Object(obj)
        }
        McpTransport::Http => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("http"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
            Value::Object(obj)
        }
        McpTransport::Sse => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("sse"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
            Value::Object(obj)
        }
    }
}

/// Build the `{"mcpServers": {...}}` document from `spec.mcps`.
fn build_mcp_json(mcps: &[McpRef]) -> Result<Value> {
    let mut servers: BTreeMap<String, Value> = BTreeMap::new();
    for mcp in mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                servers.insert(server.id.clone(), server_json(server));
            }
            McpRef::InProcess(_) => {
                bail!("in-process MCP not supported in CLI/passthrough mode");
            }
        }
    }
    Ok(json!({ "mcpServers": servers }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServer, McpTransport};
    use crate::spec::{ConfigStrategy, McpRef, Policy, SkillRef};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn write_skill(dir: &Path, id: &str) -> PathBuf {
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
    fn provision_writes_mcp_json_skills_and_launch_without_touching_home() {
        // A stand-in for the user's real `$HOME`. `provision()` never reads
        // or writes an env-derived home directory (it only touches the `dir`
        // it is explicitly given), so this must stay untouched — the core
        // invariant this test protects. (We don't actually mutate the
        // process's `HOME` var here: `std::env::set_var` requires `unsafe`
        // as of edition 2024, and this crate forbids unsafe code; asserting
        // the fake dir stays empty is sufficient since nothing in the
        // provisioner ever consults `HOME`.)
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            path: skill_path,
        });
        spec.mcps.push(McpRef::Catalog(McpServer {
            id: "postgres".to_string(),
            transport: McpTransport::Stdio,
            command: Some("postgres-mcp".to_string()),
            args: vec!["--flag".to_string()],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
        }));
        spec.mcps.push(McpRef::Inline(McpServer {
            id: "docs".to_string(),
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://example.com/mcp/".to_string()),
            headers: BTreeMap::new(),
        }));

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        // mcp.json exists and has the right shape.
        let mcp_json_path = config_dir.path().join("mcp.json");
        assert!(mcp_json_path.exists());
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        let servers = mcp_json.get("mcpServers").unwrap().as_object().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(
            servers["postgres"]["command"].as_str(),
            Some("postgres-mcp")
        );
        assert_eq!(
            servers["postgres"]["args"].as_array().unwrap().len(),
            1
        );
        assert_eq!(servers["docs"]["type"].as_str(), Some("http"));
        assert_eq!(
            servers["docs"]["url"].as_str(),
            Some("https://example.com/mcp/")
        );

        // skill copied.
        let skill_md = config_dir.path().join("skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // launch shape.
        assert!(launch.args.contains(&"--strict-mcp-config".to_string()));
        assert!(launch.args.contains(&"--mcp-config".to_string()));
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &config_dir.path().display().to_string()));
        assert!(launch.env_remove.contains(&"CLAUDECODE".to_string()));

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path())
            .unwrap()
            .collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_missing_skill_path_is_an_error() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "missing".to_string(),
            path: PathBuf::from("/definitely/does/not/exist/anywhere"),
        });

        let claude = Claude::new();
        let err = claude.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn provision_policy_writes_valid_settings_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: Some("restricted".to_string()),
            allow: vec!["Read".to_string()],
            ask: vec![],
            deny: vec!["Bash(rm *)".to_string()],
        });

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let settings_path = config_dir.path().join("settings.json");
        assert!(settings_path.exists());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        let permissions = settings.get("permissions").unwrap();
        assert_eq!(
            permissions.get("defaultMode").unwrap().as_str(),
            Some("restricted")
        );
        assert_eq!(
            permissions.get("deny").unwrap().as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn provision_empty_mcps_still_writes_mcp_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let mcp_json_path = config_dir.path().join("mcp.json");
        assert!(mcp_json_path.exists());
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        assert_eq!(
            mcp_json.get("mcpServers").unwrap().as_object().unwrap().len(),
            0
        );
    }
}
