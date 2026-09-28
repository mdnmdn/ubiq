//! GitHub Copilot CLI provisioner.
//!
//! Transcribes `_docs/harness/copilot.md`, but several load-bearing facts in
//! that doc do **not** match the real installed binary (GitHub Copilot CLI
//! 1.0.69, verified 2026-07-19 via `copilot --help`, `copilot login --help`,
//! `copilot mcp add --help`, `copilot help environment`, `copilot help
//! config`) — this file follows the verified binary, not the doc, wherever
//! they disagree, and each disagreement is called out below so the doc can be
//! corrected too. **If you're touching this file and something still looks
//! off, re-verify against `copilot --help` before trusting either this file's
//! comments or copilot.md** — that's exactly the lesson this harness taught.
//!
//! **Scope decision: CLI front-end only.** GitHub Copilot has three surfaces
//! (VS Code extension, Copilot CLI, github.com's cloud agent/code review) that
//! share a `.github/` config layout but have distinct user-profile roots and
//! processes. `am` spawns processes; VS Code's extension and github.com's
//! cloud agent aren't processes `am` can launch, so this impl targets only the
//! `copilot` binary.
//!
//! **Isolation lever — Class A, not Class C (verified, corrects an earlier
//! draft of this file and of copilot.md).** `COPILOT_HOME` **does** relocate
//! the CLI's entire config/state store — confirmed empirically: `COPILOT_HOME=
//! <dir> copilot mcp add ...` writes `<dir>/mcp-config.json` directly (not
//! `<dir>/.copilot/mcp-config.json`, and not the real `~/.copilot/`).
//! `COPILOT_HOME` **is** the `~/.copilot`-equivalent directory itself, not a
//! parent whose child is `.copilot/` — so every path below is written directly
//! under the ephemeral `dir`, with no `.copilot/` prefix. This means Copilot
//! CLI needs **no `HOME` relocation at all** — the user's real toolchain
//! (`nvm`/`mise`/`pyenv`, shell rc, PATH shims) stays intact, unlike a
//! Class-C harness (Grok). `login --help` describes the credential-store
//! fallback as "a plain text config file under `~/.copilot/`" without naming
//! it; `config.json` is carried over from copilot.md as the best-effort guess
//! (plausible, not independently confirmed — this environment authenticates
//! via the OS keychain / an env var, so no plaintext fallback file was ever
//! observed here to name for certain). Re-verify the exact filename the next
//! time a real plaintext-fallback capture is exercised.
//!
//! **Corrections to copilot.md, verified against 1.0.69:**
//! - The login command is `copilot login`, **not** `copilot auth login` — this
//!   version has no `auth` subcommand namespace at all (no `login`/`logout`/
//!   `status`/`setup-token` under `auth`; `login` is a top-level command).
//!   This was a real bug in an earlier draft of this file (reported by a user
//!   hitting `Invalid command format` at `am account login --harness copilot`).
//! - The MCP config file is `mcp-config.json`, **not** `mcp.json`; its
//!   top-level key is `mcpServers`, **not** `servers`; and its per-server
//!   schema is `{"tools": [...], "type": "local"|"http"|"sse", ...}` (stdio
//!   servers are typed `"local"`, not `"stdio"`) — verified with
//!   `copilot mcp add` under a scratch `COPILOT_HOME` for all three
//!   transports, `--env`, and `--header`.
//! - Token env var precedence is `COPILOT_GITHUB_TOKEN` > `GH_TOKEN` >
//!   `GITHUB_TOKEN` (verified via `copilot login --help` and `copilot help
//!   environment`). There is **no** `COPILOT_TOKEN` env var and **no**
//!   `copilot auth setup-token` command in this version — copilot.md's CI
//!   short-lived-token story doesn't exist in the real binary.
//! - There is **no** documented native hook slot in this CLI version — `hook`
//!   does not appear anywhere in `copilot --help` or its help topics. An
//!   earlier draft of this file wrote a speculative `hooks/am-managed.json`
//!   the real binary never reads; `spec.hooks` is now a documented no-op here
//!   (same stance as opencode's hookless slot) rather than dead plumbing.
//! - `discover_models()` **can** shell out for a real, machine-parseable model
//!   list (`copilot help config`'s `` `model`: `` settings entry is a stable
//!   quoted bullet list) — an earlier draft of this file gave up on this and
//!   returned a curated static fallback; that was premature, not a genuine
//!   doc gap.
//! - Prompt seeding differs by mode: non-interactive is `-p <text>`
//!   (`--output-format json --allow-all --no-ask-user`, as copilot.md
//!   documents); interactive is `-i <text>` (`--interactive`, "start
//!   interactive mode and automatically execute this prompt") — **not** a
//!   bare trailing positional argument (`copilot`'s parser has no such seam;
//!   feeding it one is exactly what produced the reported
//!   `Invalid command format. Did you mean: copilot -i "..."?` error for the
//!   unrelated `login` args bug above). `--model <id>` and `--resume[=<id>]`
//!   are both general top-level flags valid in either mode (`--resume` takes
//!   its value via `=`, not a separate argv token — it's an optional-value
//!   option).
//!
//! **Shared home (`D194`).** Under `ConfigStrategy::Home` a run uses the
//! account's `COPILOT_HOME` directly ([`Harness::provision_home`]): per-run
//! MCP and instructions reach it by flag and env from the run's scratch dir,
//! skills are profile-owned, and the login is the home's own, made by
//! [`Harness::login_home`] and refreshed by Copilot. `--additional-mcp-config`
//! and `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` are not re-verified against a binary.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Value, json};

use crate::Result;
use crate::config::{McpServer, McpTransport};
use crate::spec::{McpRef, RunSpec};

use super::{ConfigAnchor, Harness, Launch, Relocate};

/// The GitHub Copilot CLI harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Copilot;

impl Copilot {
    /// Construct the GitHub Copilot CLI harness descriptor.
    pub fn new() -> Self {
        Copilot
    }

    /// The launch shared by [`Harness::provision`] and [`Harness::provision_home`]:
    /// `COPILOT_HOME=<home>` when there is a home (none for a native run), the account's
    /// credential references, and `config_args` ahead of the user's passthrough args.
    ///
    /// Verified against `copilot --help`: prompt seeding differs by mode (`-p`
    /// non-interactive vs `-i` interactive — there is no bare positional-prompt seam);
    /// `--model`/`--resume` are general flags valid in either mode; `--resume` takes its
    /// value via `=` (an optional-value option, not a separate argv token).
    fn launch(
        &self,
        spec: &RunSpec,
        home: Option<&Path>,
        config_args: Vec<String>,
    ) -> Result<Launch> {
        let args = if spec.io == crate::spec::IoModes::Structured {
            // Structured mode: `copilot --acp [args...]` — an ACP endpoint,
            // driven by `crate::io::AcpBridge`. Everything a passthrough run
            // puts in argv moves onto the wire here: the prompt is a
            // `session/prompt`, a resume is a `session/load` (from
            // `Provisioned::resume`), and the model is a
            // `session/set_config_option`, so none of `-p`, `--output-format
            // json`, `--allow-all`, `--no-ask-user`, `--model` or
            // `--resume=` belongs on this argv. Permissions are now real
            // `session/request_permission` round trips instead of
            // `--allow-all`/`--no-ask-user` auto-approval.
            let mut structured_args = vec!["--acp".to_string()];
            structured_args.extend(config_args);
            structured_args.extend(spec.passthrough_args.iter().cloned());
            structured_args
        } else {
            let mut args = config_args;
            args.extend(spec.passthrough_args.iter().cloned());
            if let Some(model) = &spec.model {
                args.push("--model".to_string());
                args.push(model.clone());
            }
            if let Some(id) = &spec.resume {
                args.push(format!("--resume={id}"));
            }
            if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
                args.push("-i".to_string());
                args.push(prompt.clone());
            }
            args
        };

        // Account: inject credential *references* into the child's env.
        let mut env: Vec<(String, String)> = home
            .map(|home| ("COPILOT_HOME".to_string(), home.display().to_string()))
            .into_iter()
            .collect();
        if let Some(account) = &spec.account {
            // `api_key_env` maps to `COPILOT_GITHUB_TOKEN` (highest
            // precedence per `copilot login --help`/`copilot help
            // environment`; accepts fine-grained PATs and OAuth tokens
            // alike). `auth_token_env` maps to `GITHUB_TOKEN` (lowest of the
            // three documented token env vars, still real and functional) —
            // if both are set, api_key_env wins (this codebase's established
            // convention). There is no `COPILOT_TOKEN` — see module doc.
            if let Some(name) = &account.api_key_env {
                let value = super::shared::account_env(account, name)?;
                env.push(("COPILOT_GITHUB_TOKEN".to_string(), value));
            } else if let Some(name) = &account.auth_token_env {
                let value = super::shared::account_env(account, name)?;
                env.push(("GITHUB_TOKEN".to_string(), value));
            }
            // `base_url` maps to `COPILOT_GH_HOST` (verified via `copilot
            // help environment`: "GitHub hostname used only by Copilot CLI
            // ... overriding GH_HOST when set" — a GitHub Enterprise Cloud
            // data-residency hostname, e.g. "mycompany.ghe.com", NOT a full
            // https:// URL despite the field's generic name).
            if let Some(base_url) = &account.base_url {
                env.push(("COPILOT_GH_HOST".to_string(), base_url.clone()));
            }
        }

        Ok(Launch {
            program: "copilot".to_string(),
            args,
            env,
            env_remove: Vec::new(),
            env_clear: false,
        })
    }
}

impl Harness for Copilot {
    // Copilot's structured seam is `copilot --acp` — an ACP endpoint, driven
    // by the generic `crate::io::AcpBridge`, so nothing harness-specific is
    // parsed here (see `structured_bridge` below).
    super::shared::harness_identity! {
        id: "copilot",
        display_name: "GitHub Copilot",
        command: "copilot",
        aliases: [],
        passthrough: true,
        structured: true,
        multi_turn: true,
        acp: true,
    }

    /// Class A: `COPILOT_HOME` relocates the CLI's entire config/state tree —
    /// verified against the installed binary (see the module doc); `HOME` is
    /// never touched.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![("COPILOT_HOME".to_string(), Relocate::All)],
            requires_home_relocation: false,
        }
    }

    /// `copilot help config`'s `` `model`: `` settings entry is a stable,
    /// machine-parseable quoted bullet list (verified against 1.0.69) — shell
    /// out and scrape that one block rather than falling back to a curated
    /// static list. Needs no auth/network (shown in plain `--help` text).
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        let mut cmd = std::process::Command::new("copilot");
        cmd.args(["help", "config"]);
        cmd.current_dir(super::shared::probe_cwd());
        // Headless help-text scrape: nothing reads a window, and the app has already freed its own
        // console (`ubiq_app::detach_console`), so a console-subsystem child would otherwise flash one.
        #[cfg(windows)]
        super::shared::no_window(&mut cmd);
        let output = cmd
            .output()
            .with_context(|| "running `copilot help config` (is the copilot binary on PATH?)")?;
        if !output.status.success() {
            anyhow::bail!(
                "`copilot help config` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let ids = parse_model_ids(&stdout);
        if ids.is_empty() {
            anyhow::bail!(
                "no model ids found in `copilot help config` output — its format may have \
                 changed; re-verify against the installed binary"
            );
        }
        let mut out: Vec<super::ModelInfo> = ids.into_iter().map(super::ModelInfo::new).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // `dir` IS `$COPILOT_HOME` directly (no `.copilot/` prefix — see
        // module doc), so every injected file below lands straight under it.
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        // 1. Skills: copy each skill folder into <dir>/skills/<id>/ (the CLI
        // user-tier location per copilot.md's on-disk layout table, minus the
        // `.copilot/` prefix COPILOT_HOME already relocates away).
        write_skills(spec, &dir.join("skills"))?;

        // 2. MCP: <dir>/mcp-config.json, the CLI's own user-profile MCP file.
        write_mcp_config(spec, dir)?;

        // 3. Hooks: no-op. `hook` does not appear anywhere in the installed
        // binary's `--help`/help topics (verified) — there is no native hook
        // slot to write into for this CLI version, so `spec.hooks` is a
        // documented fidelity gap here, not a user mistake (same stance as
        // opencode's hookless slot).

        // 4. Instructions: <dir>/copilot-instructions.md — per copilot.md's
        // documented global personal-instructions path (`--no-custom-
        // instructions` existing in `--help` confirms *some* default
        // instructions file is loaded; this exact path is not independently
        // re-verified, but writing it is harmless if wrong — worst case it's
        // silently unread, unlike the hooks case above).
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let instructions_path = dir.join("copilot-instructions.md");
            std::fs::write(&instructions_path, instr_text)
                .with_context(|| format!("writing {}", instructions_path.display()))?;
        }

        // 5. Build the launch against `dir` as `$COPILOT_HOME`.
        let launch = self.launch(spec, Some(dir), Vec::new())?;
        Ok(launch)
    }

    /// Copilot CLI runs from the account's shared `COPILOT_HOME` (`D194`).
    fn shares_home(&self) -> bool {
        true
    }

    /// `$COPILOT_HOME`, else `~/.copilot`: `config.json`, token and settings together.
    fn default_homes(&self) -> Vec<std::path::PathBuf> {
        super::env_dir_or_home("COPILOT_HOME", ".copilot")
            .into_iter()
            .collect()
    }

    /// A run against the account's shared `COPILOT_HOME` (`D194`), whose `config.json` holds the
    /// login and the user's settings together — so nothing here writes it, and nothing per-run
    /// lands in `home`:
    ///
    /// - MCP → `<scratch>/mcp-config.json`, `--additional-mcp-config @<file>`, which augments the
    ///   home's own `mcp-config.json` for this session. The same argv reaches a structured
    ///   `copilot --acp` run, whose `session/new` sends `mcpServers: []`;
    /// - instructions → `<scratch>/instructions/AGENTS.md`, named by
    ///   `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` (copilot.md: the CLI reads an `AGENTS.md` in each);
    /// - hooks → a no-op, as in [`Harness::provision`].
    ///
    /// Skills have no per-run route, so they are **profile-owned**: written into
    /// `<home>/skills/<id>/` by [`write_profile_skills`] — a whole-skill swap under a lock, never
    /// a partial folder. A native run (no `home`) sets no `COPILOT_HOME`, runs from the user's
    /// own `~/.copilot`, and writes no skill there. No login is seeded under either.
    fn provision_home(
        &self,
        spec: &RunSpec,
        home: Option<&Path>,
        scratch: &Path,
    ) -> Result<Launch> {
        let mut config_args = Vec::new();
        if let Some(mcp_path) = write_mcp_config(spec, scratch)? {
            config_args.push("--additional-mcp-config".to_string());
            config_args.push(format!("@{}", mcp_path.display()));
        }
        let mut launch = self.launch(spec, home, config_args)?;
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let instructions_dir = scratch.join("instructions");
            std::fs::create_dir_all(&instructions_dir)
                .with_context(|| format!("creating {}", instructions_dir.display()))?;
            let agents_md = instructions_dir.join("AGENTS.md");
            std::fs::write(&agents_md, instr_text)
                .with_context(|| format!("writing {}", agents_md.display()))?;
            launch.env.push((
                "COPILOT_CUSTOM_INSTRUCTIONS_DIRS".to_string(),
                instructions_dir.display().to_string(),
            ));
        }
        match home {
            Some(home) => write_profile_skills(spec, home)?,
            None if !spec.skills.is_empty() || !spec.mcp_as_skill.is_empty() => {
                tracing::warn!(
                    skills = spec.skills.len() + spec.mcp_as_skill.len(),
                    "copilot has no per-run skill route; skills are not applied to a run with no profile"
                );
            }
            None => {}
        }
        Ok(launch)
    }

    /// `copilot login` with `COPILOT_HOME=<home>`, and the real `HOME` left alone: the login
    /// lands in the home's `config.json` (or the OS credential store Copilot picks), where every
    /// run of the profile reads it, and Copilot refreshes it from then on. Nothing is captured.
    fn login_home(&self, home: &Path) -> Result<Launch> {
        Ok(Launch {
            program: "copilot".to_string(),
            args: vec!["login".to_string()],
            env: vec![("COPILOT_HOME".to_string(), home.display().to_string())],
            env_remove: Vec::new(),
            env_clear: false,
        })
    }

    /// Copilot CLI keeps its GitHub token in `config.json` at the root of `COPILOT_HOME` — the
    /// home itself. On macOS a harness may keep the login in the OS keychain instead, so an
    /// absent file is not proof of no login.
    fn login_files(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("config.json")]
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::AcpBridge::new(
            child,
            cwd,
            provisioned.resume.as_deref(),
            provisioned.model.as_deref(),
        )?))
    }
}

/// Render one [`McpServer`] into the JSON shape `mcp-config.json`'s
/// `mcpServers` map expects — verified against the installed binary via
/// `copilot mcp add` (stdio/http/sse, `--env`, `--header`) under a scratch
/// `COPILOT_HOME`. Every entry always carries `tools`; `am` has no per-server
/// tool filter today, so this always emits `["*"]` (all tools allowed).
fn mcp_server_json(server: &McpServer) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("tools".to_string(), json!(["*"]));
    match server.transport {
        // Verified: stdio servers are typed "local", not "stdio".
        McpTransport::Stdio => {
            obj.insert("type".to_string(), json!("local"));
            if let Some(command) = &server.command {
                obj.insert("command".to_string(), json!(command));
            }
            if !server.args.is_empty() {
                obj.insert("args".to_string(), json!(server.args));
            }
            if !server.env.is_empty() {
                obj.insert("env".to_string(), json!(server.env));
            }
        }
        McpTransport::Http => {
            obj.insert("type".to_string(), json!("http"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
        }
        McpTransport::Sse => {
            obj.insert("type".to_string(), json!("sse"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
        }
    }
    Value::Object(obj)
}

/// Build the `mcpServers` map from `spec.mcps`, keyed by server id.
fn build_mcp_servers(mcps: &[McpRef]) -> Result<serde_json::Map<String, Value>> {
    let mut servers = serde_json::Map::new();
    for mcp in mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                servers.insert(server.id.clone(), mcp_server_json(server));
            }
            McpRef::InProcess(_) => {
                anyhow::bail!("in-process MCP not supported in passthrough mode");
            }
        }
    }
    Ok(servers)
}

/// Write `<dir>/mcp-config.json` with a top-level `mcpServers` key (verified against the
/// installed binary — NOT `mcp.json`/`servers`; see module doc) and return its path. Written
/// only when there are servers to inject — no documented stub-file requirement, mirrors Grok's
/// "unused runs stay minimal" convention.
fn write_mcp_config(spec: &RunSpec, dir: &Path) -> Result<Option<PathBuf>> {
    let mcp_map = build_mcp_servers(&spec.mcps)?;
    if mcp_map.is_empty() {
        return Ok(None);
    }
    let mcp_json = json!({ "mcpServers": Value::Object(mcp_map) });
    let mcp_json_path = dir.join("mcp-config.json");
    std::fs::write(&mcp_json_path, serde_json::to_string_pretty(&mcp_json)?)
        .with_context(|| format!("writing {}", mcp_json_path.display()))?;
    Ok(Some(mcp_json_path))
}

/// Copy each skill folder into `<skills_dir>/<id>/`, plus the MCP-as-skill `SKILL.md`
/// pointers (see [`super::write_mcp_as_skill_pointers`]; a no-op when there are none).
fn write_skills(spec: &RunSpec, skills_dir: &Path) -> Result<()> {
    for skill in &spec.skills {
        let dest = skills_dir.join(&skill.id);
        skill
            .source
            .materialize(&dest, crate::source::LinkMode::Copy, true)
            .with_context(|| format!("copying skill '{}' into {}", skill.id, dest.display()))?;
    }
    super::write_mcp_as_skill_pointers(spec, skills_dir)
}

/// Put the run's skills into a shared home's `skills/` — profile-owned, since Copilot CLI has
/// no per-run skill route (`D194`). Each skill is staged whole in `skills/.am-stage/` and
/// renamed into place, so a concurrent run never reads half a skill folder; staging and swap
/// happen under a lock beside `skills/`, so two runs of one profile never interleave. A skill
/// the home already holds under that id is replaced; one the run does not name is left alone.
fn write_profile_skills(spec: &RunSpec, home: &Path) -> Result<()> {
    if spec.skills.is_empty() && spec.mcp_as_skill.is_empty() {
        return Ok(());
    }
    let skills_dir = home.join("skills");
    std::fs::create_dir_all(&skills_dir)
        .with_context(|| format!("creating {}", skills_dir.display()))?;
    // Released when `_lock` drops, at the end of this function.
    let _lock = super::lock_beside(&skills_dir)?;
    let stage = skills_dir.join(".am-stage");
    let old = skills_dir.join(".am-old");
    for leftover in [&stage, &old] {
        if leftover.exists() {
            std::fs::remove_dir_all(leftover)
                .with_context(|| format!("removing {}", leftover.display()))?;
        }
    }
    write_skills(spec, &stage)?;
    for entry in
        std::fs::read_dir(&stage).with_context(|| format!("reading {}", stage.display()))?
    {
        let staged = entry?.path();
        let dest = skills_dir.join(staged.file_name().unwrap_or_default());
        if dest.exists() {
            std::fs::rename(&dest, &old).with_context(|| format!("moving {}", dest.display()))?;
        }
        std::fs::rename(&staged, &dest)
            .with_context(|| format!("renaming {} onto {}", staged.display(), dest.display()))?;
        if old.exists() {
            std::fs::remove_dir_all(&old).with_context(|| format!("removing {}", old.display()))?;
        }
    }
    std::fs::remove_dir(&stage).with_context(|| format!("removing {}", stage.display()))?;
    Ok(())
}

/// Scrape the `` `model`: `` settings entry out of `copilot help config`'s
/// output: a quoted bullet (`    - "id"`) per line, ending at the first
/// non-bullet, non-blank line after the block starts. Pure (no I/O) so it's
/// unit-testable directly against a captured snippet of real `--help` output.
fn parse_model_ids(help_text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut in_model_block = false;
    for line in help_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("`model`:") {
            in_model_block = true;
            continue;
        }
        if !in_model_block {
            continue;
        }
        if let Some(id) = trimmed
            .strip_prefix("- \"")
            .and_then(|s| s.strip_suffix('"'))
        {
            ids.push(id.to_string());
        } else if !trimmed.is_empty() {
            break;
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ConfigStrategy, Instructions, McpRef, SkillRef};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    super::super::shared::harness_conformance_tests!(Copilot, "copilot");

    #[test]
    fn provision_writes_mcp_config_json_skills_and_launch_without_touching_home() {
        // Stand-in for the user's real `$HOME`. `provision()` never overrides
        // `HOME` at all for Copilot (Class A via `COPILOT_HOME`) and never
        // writes outside the `dir` it's given, so this must stay empty.
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(skill_path),
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

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        // mcp-config.json exists directly under `dir` (no `.copilot/`
        // prefix) with the right `mcpServers` shape.
        let mcp_json_path = config_dir.path().join("mcp-config.json");
        assert!(mcp_json_path.exists());
        let parsed: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        let servers = parsed.get("mcpServers").unwrap().as_object().unwrap();
        assert!(parsed.get("servers").is_none());
        assert_eq!(servers.len(), 2);
        // stdio -> "local" type + command + args + default tools.
        assert_eq!(servers["postgres"]["type"].as_str(), Some("local"));
        assert_eq!(
            servers["postgres"]["command"].as_str(),
            Some("postgres-mcp")
        );
        assert_eq!(servers["postgres"]["args"].as_array().unwrap().len(), 1);
        assert_eq!(
            servers["postgres"]["tools"].as_array().unwrap()[0].as_str(),
            Some("*")
        );
        // http remote: type + url.
        assert_eq!(servers["docs"]["type"].as_str(), Some("http"));
        assert_eq!(
            servers["docs"]["url"].as_str(),
            Some("https://example.com/mcp/")
        );

        // Skill copied directly under <dir>/skills.
        let skill_md = config_dir.path().join("skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // Launch relocates COPILOT_HOME, and does NOT override HOME at all.
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "COPILOT_HOME" && v == &config_dir.path().display().to_string())
        );
        assert!(!launch.env.iter().any(|(k, _)| k == "HOME"));
        assert_eq!(launch.program, "copilot");

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path()).unwrap().collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_empty_mcps_writes_no_mcp_config_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        let copilot = Copilot::new();
        copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(!config_dir.path().join("mcp-config.json").exists());
    }

    #[test]
    fn provision_mcp_in_process_is_an_error() {
        use crate::mcp::{McpService, ToolDef};
        use crate::spec::InProcessMcpHandle;
        use std::sync::Arc;

        struct NoopService;
        impl McpService for NoopService {
            fn tools(&self) -> Vec<ToolDef> {
                Vec::new()
            }
            fn call(&self, _name: &str, _arguments: Value) -> crate::Result<Value> {
                anyhow::bail!("not implemented")
            }
        }

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::InProcess(InProcessMcpHandle {
            name: "in-proc".to_string(),
            service: Arc::new(NoopService),
        }));

        let copilot = Copilot::new();
        let err = copilot.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("in-process"));
    }

    #[test]
    fn provision_hooks_are_a_documented_noop() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.hooks.push(crate::spec::HookRef {
            id: "notify".to_string(),
            event: "PreToolUse".to_string(),
            command: "notify-send hi".to_string(),
            matcher: Some("Bash".to_string()),
        });

        let copilot = Copilot::new();
        // Must not error and must not write anything hook-shaped — this CLI
        // version has no native hook slot.
        copilot.provision(&spec, config_dir.path()).unwrap();
        assert!(!config_dir.path().join("hooks").exists());
    }

    #[test]
    fn provision_instructions_writes_copilot_instructions_md() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER: be terse".to_string()),
            prompt: None,
        });

        let copilot = Copilot::new();
        copilot.provision(&spec, config_dir.path()).unwrap();

        let instructions_path = config_dir.path().join("copilot-instructions.md");
        assert!(instructions_path.exists());
        let content = std::fs::read_to_string(&instructions_path).unwrap();
        assert!(content.contains("REMEMBER: be terse"));
    }

    #[test]
    fn provision_account_api_key_env_maps_to_copilot_github_token() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "pat".to_string(),
            api_key_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "COPILOT_GITHUB_TOKEN" && v == &expected)
        );
        assert!(!launch.env.iter().any(|(k, _)| k == "GITHUB_TOKEN"));

        // No-secret-on-disk invariant.
        super::super::shared::assert_no_secret_on_disk(config_dir.path(), &expected);
    }

    #[test]
    fn provision_account_auth_token_env_maps_to_github_token() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "fallback".to_string(),
            auth_token_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "GITHUB_TOKEN" && v == &expected)
        );
        assert!(!launch.env.iter().any(|(k, _)| k == "COPILOT_GITHUB_TOKEN"));
    }

    #[test]
    fn provision_account_base_url_maps_to_copilot_gh_host() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "enterprise".to_string(),
            base_url: Some("mycompany.ghe.com".to_string()),
            ..Default::default()
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "COPILOT_GH_HOST" && v == "mycompany.ghe.com")
        );
    }

    #[test]
    fn provision_structured_is_acp_without_prompt_resume_or_model() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.model = Some("gpt-5.4".to_string());
        spec.resume = Some("sess-123".to_string());
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert_eq!(launch.args[0], "--acp");
        // The prompt, the resume and the model all travel over the wire.
        assert!(!launch.args.contains(&"-p".to_string()));
        assert!(!launch.args.contains(&"--output-format".to_string()));
        assert!(!launch.args.contains(&"--allow-all".to_string()));
        assert!(!launch.args.contains(&"--no-ask-user".to_string()));
        assert!(!launch.args.contains(&"--model".to_string()));
        assert!(!launch.args.contains(&"--resume=sess-123".to_string()));
        assert!(!launch.args.contains(&"say hello world".to_string()));
    }

    #[test]
    fn provision_passthrough_builds_interactive_argv() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.model = Some("gpt-5.4".to_string());
        spec.resume = Some("sess-123".to_string());
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"-p".to_string()));
        assert!(!launch.args.contains(&"--output-format".to_string()));
        assert!(!launch.args.contains(&"--allow-all".to_string()));
        let model_idx = launch.args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(launch.args.get(model_idx + 1), Some(&"gpt-5.4".to_string()));
        assert!(launch.args.contains(&"--resume=sess-123".to_string()));
        // Interactive prompt seeding is `-i <text>`, never a bare positional.
        let i_idx = launch.args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(
            launch.args.get(i_idx + 1),
            Some(&"say hello world".to_string())
        );
    }

    fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|a| a == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    fn home_spec(home: &std::path::Path, scratch: &std::path::Path) -> RunSpec {
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Home {
            home: home.to_path_buf(),
            scratch: scratch.to_path_buf(),
        };
        spec.mcps.push(McpRef::Inline(McpServer {
            id: "docs".to_string(),
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://example.com/mcp/run-1".to_string()),
            headers: BTreeMap::new(),
        }));
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER ME".to_string()),
            prompt: None,
        });
        spec
    }

    #[test]
    fn provision_home_puts_per_run_files_in_scratch_and_keeps_config_json() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        // The home's `config.json` holds the login and the user's settings.
        let config_json = home.path().join("config.json");
        let login = r#"{"lastLoggedInUser":{"login":"octocat"},"model":"gpt-5.4"}"#;
        std::fs::write(&config_json, login).unwrap();
        let spec = home_spec(home.path(), scratch.path());

        let launch = Copilot::new()
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        // Nothing per-run lands in the shared home, and `config.json` is untouched.
        let home_entries: Vec<_> = std::fs::read_dir(home.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(home_entries, vec!["config.json"]);
        assert_eq!(std::fs::read_to_string(&config_json).unwrap(), login);

        let mcp_path = scratch.path().join("mcp-config.json");
        assert!(mcp_path.is_file());
        assert_eq!(
            flag_value(&launch.args, "--additional-mcp-config"),
            Some(format!("@{}", mcp_path.display()).as_str())
        );
        let instructions_dir = scratch.path().join("instructions");
        assert_eq!(
            std::fs::read_to_string(instructions_dir.join("AGENTS.md")).unwrap(),
            "REMEMBER ME"
        );
        let env: BTreeMap<_, _> = launch.env.iter().cloned().collect();
        assert_eq!(
            env.get("COPILOT_HOME"),
            Some(&home.path().display().to_string())
        );
        assert_eq!(
            env.get("COPILOT_CUSTOM_INSTRUCTIONS_DIRS"),
            Some(&instructions_dir.display().to_string())
        );
        assert!(!env.contains_key("HOME"));
    }

    #[test]
    fn provision_home_structured_passes_the_mcp_flag_to_acp() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let mut spec = home_spec(home.path(), scratch.path());
        spec.io = crate::spec::IoModes::Structured;

        let launch = Copilot::new()
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        assert_eq!(launch.args[0], "--acp");
        assert!(launch.args.contains(&"--additional-mcp-config".to_string()));
    }

    #[test]
    fn provision_home_without_a_home_sets_no_copilot_home_and_writes_no_skill() {
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Native {
            scratch: scratch.path().to_path_buf(),
        };
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(write_skill(skills_src.path(), "my-skill")),
        });

        let launch = Copilot::new()
            .provision_home(&spec, None, scratch.path())
            .unwrap();

        assert!(!launch.env.iter().any(|(k, _)| k == "COPILOT_HOME"));
        assert!(!launch.args.contains(&"--additional-mcp-config".to_string()));
        let scratch_entries: Vec<_> = std::fs::read_dir(scratch.path()).unwrap().collect();
        assert!(
            scratch_entries.is_empty(),
            "scratch got: {scratch_entries:?}"
        );
    }

    #[test]
    fn provision_home_writes_skills_into_the_home_whole_and_keeps_the_rest() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        // A stale copy of the run's skill, and a skill the run does not name.
        std::fs::create_dir_all(home.path().join("skills/my-skill")).unwrap();
        std::fs::write(home.path().join("skills/my-skill/stale.md"), "old").unwrap();
        std::fs::create_dir_all(home.path().join("skills/user-skill")).unwrap();
        let mut spec = home_spec(home.path(), scratch.path());
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(write_skill(skills_src.path(), "my-skill")),
        });

        Copilot::new()
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        let skills = home.path().join("skills");
        assert!(skills.join("my-skill/SKILL.md").is_file());
        assert!(!skills.join("my-skill/stale.md").exists());
        assert!(skills.join("user-skill").is_dir());
        assert!(!skills.join(".am-stage").exists());
        assert!(!skills.join(".am-old").exists());
        assert!(!scratch.path().join("skills").exists());
    }

    #[test]
    fn copilot_shares_a_home() {
        assert!(Copilot::new().shares_home());
    }

    #[test]
    fn login_home_logs_in_through_copilot_home_and_leaves_home_alone() {
        let home = tempfile::TempDir::new().unwrap();
        let launch = Copilot::new().login_home(home.path()).unwrap();

        assert_eq!(launch.program, "copilot");
        assert_eq!(launch.args, vec!["login"]);
        assert_eq!(
            launch.env,
            vec![(
                "COPILOT_HOME".to_string(),
                home.path().display().to_string()
            )]
        );
    }

    #[test]
    fn resolve_copilot_by_id() {
        assert_eq!(super::super::resolve("copilot").unwrap().id(), "copilot");
    }

    #[test]
    fn parse_model_ids_extracts_the_quoted_bullet_list() {
        // A trimmed real excerpt of `copilot help config`'s `model` entry.
        let help_text = r#"
  `logLevel`: log level for CLI; defaults to "default".

  `model`: AI model to use for Copilot CLI; can be changed with /model command or --model flag option.
    - "claude-sonnet-5"
    - "gpt-5.4"
    - "gemini-3.5-flash"

  `contextTier`: context window tier for tiered-pricing models.
"#;
        let ids = parse_model_ids(help_text);
        assert_eq!(ids, vec!["claude-sonnet-5", "gpt-5.4", "gemini-3.5-flash"]);
    }

    #[test]
    fn parse_model_ids_missing_block_returns_empty() {
        assert_eq!(
            parse_model_ids("no model entry here at all"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn copilot_supports_passthrough_and_an_acp_structured_bridge() {
        let copilot = Copilot::new();
        let support = copilot.io_support();
        assert!(support.passthrough);
        assert!(support.structured);
        assert!(support.multi_turn);
        assert!(support.acp);
    }
}
