# `am` as a library — storage extension points

This doc is for an embedder linking `agent-manager` as a Rust crate (`use
agent_manager::…`) rather than shelling out to the `am` binary — e.g. Ubiq, a
web UI, or a CI job. It focuses on what makes embedding worthwhile: **every
persistent store `am` reads from is a trait**, so you can back accounts,
profiles, skills, MCPs, templates, and sessions with a database or in-memory
map instead of the filesystem. The CLI is just one implementation of those
traits — the filesystem one.

See also [overview](./overview.md) / [architecture](./architecture.md) for
the pipeline in general, [registry](./registry.md) and
[profiles](./profiles.md) for the filesystem-backed stores in detail, and
[open points](./open-points.md) for what's unfinished.

## 1. Two modes, one front-end-agnostic core

`am` ships as both a standalone CLI and a library, controlled by Cargo
features:

| Feature | Pulls in | Needed for |
|---|---|---|
| `cli` | `clap`, `tracing`, `pty` | the `am` binary's command-line surface |
| `tui` | `ratatui`, `crossterm` | the interactive TUI front end |
| `pty` | `portable-pty`, `crossterm`, `signal-hook` | spawning the harness in a real PTY and forwarding a tty (`src/run.rs`) |
| `inproc-mcp` | `tiny_http` | hosting an embedder-registered in-process MCP server over loopback HTTP |
| `frontend` (default) | `cli` + `tui` | the standalone binary — **not** what a lib-mode embedder wants |

An embedder depends on the crate with `default-features = false`:

```toml
[dependencies]
agent-manager = { path = "...", default-features = false, features = ["inproc-mcp"] }
```

That gets you the core — `spec`, `resolve`, `registry`, `account`, `profile`,
`harness`, `provision`, `session`, `source`, `config`, `settings`,
`isolate`, `overlay`, and the neutral `io` model — no `clap` types, no
terminal assumptions below `cli`/`tui`. This is the "front-end-agnostic
core" invariant from [architecture.md](./architecture.md#invariants): lib
mode is why it has to hold.

In CLI mode, flags parse into a `RunFlags`, a `Settings` file loads, and both
feed `resolve::resolve(...)`. In lib mode you can do the same, or skip
resolution entirely and build a `spec::RunSpec` directly via
`RunSpec::new(harness, cwd)` plus field assignment — full programmatic
control, no flags or settings file involved.

## 2. The pipeline an embedder drives

```rust
// 1. Resolve flags + settings + stores into a self-contained RunSpec.
let spec = agent_manager::resolve::resolve(
    &flags, &settings,
    &registry,   // &dyn registry::Registry
    &accounts,   // &dyn account::AccountStore
    &profiles,   // &dyn profile::ProfileStore
)?;

// 2. Provision: turn the spec into a populated, real on-disk config dir + launch argv/env.
let provisioned = agent_manager::provision::provision(
    &harness,    // &dyn harness::Harness (e.g. harness::Claude::new())
    &spec,
    &templates,  // &dyn harness::TemplateStore
)?;

// 3a. With `pty`: spawn + supervise in a real terminal.
let exit_code = agent_manager::run::run(&provisioned, &spec.cwd, /* keep_config */ false)?;

// 3b. Without `pty`: bridge I/O yourself (spawn_piped + your own event loop,
//     see io::structured), recording via a SessionRecorder.
let mut recorder = sessions.start(meta)?;   // &dyn session::SessionStore
recorder.record_event(&event)?;
recorder.finish(Some(exit_code))?;
```

There's no `Stores` struct bundling these — `resolve` and `provision` take
the trait objects they need as plain arguments. Bundle them yourself if
convenient; the crate doesn't impose a shape.

**The invariant that makes this composable: `RunSpec` is self-contained.**
`resolve` is the *only* stage that talks to `Registry`/`AccountStore`/
`ProfileStore`. It bakes their content into the spec — a skill's folder
becomes `SkillRef.source`, an account's captured login becomes
`RunSpec.account_login`, a profile chain's overlays become
`RunSpec.config_bases` — so `provision` never calls back into a store. A
`RunSpec` (mostly `Serialize`) can be shipped elsewhere and provisioned there
with zero store access.

## 3. The `Source` content seam

The abstraction the whole embedding story rests on (`src/source.rs`):

```rust
pub enum Source {
    Dir(PathBuf),                    // an existing directory — the FS stores' zero-cost path
    Files(Vec<(PathBuf, Vec<u8>)>),  // relative path -> bytes — database/memory-backed stores
}

impl Source {
    pub fn materialize(&self, dest: &Path, mode: LinkMode, clobber: bool) -> Result<()>;
    pub fn read(&self, rel: &Path) -> Result<Option<Vec<u8>>>;
}

pub enum LinkMode {
    Copy,          // credentials and anything the harness rewrites in place
    LinkElseCopy,  // symlink when possible; read-only overlay config
}
```

Every persistent store hands out `Source` instead of a raw path. A
filesystem store returns `Source::Dir(path)`; `materialize` copies or
symlinks from it. A database/memory store returns `Source::Files(...)`;
`materialize` writes each entry as a real file. The provisioner calls
`materialize`/`read` identically either way — it never needs to know which
kind of store produced the content.

This is what replaced the old `PathBuf` leaks stores used to hand out
directly: `SkillEntry.path`, an `Account`'s `home`, a profile's
`base/<harness>` dir. Each is now reachable through a `Source`, so a
database-backed embedder never needs a real directory on disk just to
satisfy the trait — it hands over bytes, and the filesystem only enters the
picture inside `materialize`.

## 4. The physical boundary — the run dir stays filesystem

`Source` abstracts where content *comes from*, not where it *ends up*. The
harness is a real OS subprocess reading real files — that's never
abstracted. So the ephemeral run dir `provision` creates (default
`~/.config/agent-manager/runs/<run-id>/`, overridable via `AM_RUNS`) is
always on disk, in every mode.

The rule: **persistent stores yield content; the provisioner materializes it
into a real on-disk run dir.** A database-backed embedder replaces the
*persistent* stores (`Registry`, `AccountStore`, `ProfileStore`,
`TemplateStore`, `SessionStore`) — it never replaces the run dir itself.
Everything upstream of `provision` can live anywhere; from `provision`
onward it's real files on real disk, because that's what the harness binary
expects.

## 5. Each store trait as an extension point

**`registry::Registry`** — skills and MCP servers:

```rust
fn skills(&self) -> Result<Vec<SkillEntry>>;
fn mcps(&self) -> Result<Vec<McpEntry>>;
fn skill(&self, id: &str) -> Result<Option<SkillEntry>>;  // default: filter skills()
fn mcp(&self, id: &str) -> Result<Option<McpEntry>>;      // default: filter mcps()
// SkillEntry { id, source: Source, meta: SkillMeta }
```

FS impl: `registry::FsRegistry`, rooted at a catalog dir (`AM_CATALOG` /
`~/.config/agent-manager/catalog`; see [registry.md](./registry.md)). A
database `Registry` returns `SkillEntry { source: Source::Files(...), .. }`
instead of reading a folder. `registry::OverlayRegistry<G, P>` composes two
`Registry`s (global + project, project wins on collision) — reusable over
any pair of stores.

**`account::AccountStore`** — credential references + the login write seam:

```rust
fn accounts(&self) -> Result<Vec<Account>>;
fn account(&self, id: &str) -> Result<Option<Account>>;                     // default: filter accounts()
fn login_source(&self, id: &str) -> Result<Option<Source>>;                 // default: Source::Dir(account.home)
fn login_home(&self, id: &str) -> Result<PathBuf>;                          // default: read-only error
fn capture_login(&self, id: &str, from: &Path, files: &[PathBuf]) -> Result<()>; // default: read-only error
```

`Account` never holds a secret value, only references (env-var names, a base
URL, a helper command, a `home` path). `login_source`'s default derives a
`Source::Dir` from `Account.home` — correct for any filesystem store; a
database store overrides it to return `Source::Files` built from stored
bytes. `login_home`/`capture_login` are the interactive-login write seam
(`am account login`): a login is a real subprocess that must write to a real
dir, so `login_home` returns one (`FsAccountStore` returns the persistent
per-account home; a DB store would return a scratch dir it reads back), and
`capture_login` persists what got written there. Both default to a
read-only error, so a read-only store implements neither.

FS impl: `account::FsAccountStore` (`accounts.toml` + per-file `<id>.toml`,
rooted at `AM_ACCOUNTS`); `account::EmptyAccountStore` is the zero-accounts
default.

**`profile::ProfileStore`** — persistent bases + inheritance:

```rust
fn profiles(&self) -> Result<Vec<Profile>>;
fn profile(&self, id: &str) -> Result<Option<Profile>>;                       // default: filter profiles()
fn base_source(&self, id: &str, harness: &str) -> Option<Source>;             // default: None
fn put_base(&self, id: &str, harness: &str, from: &Path) -> Result<()>;       // default: read-only error
```

A `Profile` ties an account id, composition defaults (mcps/skills/model/
hooks/instructions), an optional harness pin, and an isolation default
together; `extends` chains profiles (see [profiles.md](./profiles.md) §6).
`resolve_chain`/`flatten` walk and fold that chain (replace-by-default,
root→leaf); `resolve_flattened` does both. `base_source` is the
config-overlay content seam (extra settings/memory/skills, not credentials)
— `resolve` collects it across the `extends` chain into
`RunSpec.config_bases`, so `provision` layers it via `overlay::materialize`
without touching `ProfileStore` again. `put_base` is the copy-back write
seam, mirroring `capture_login`.

FS impl: `profile::FsProfileStore` — profiles are *directories*
(`<root>/<name>/profile.toml` + `base/<harness>/`), unlike accounts' flat
files, because a profile owns persistent per-harness state.
`profile::EmptyProfileStore` is the zero-profiles default.

**`harness::TemplateStore`** — editable preference defaults:

```rust
fn template(&self, harness_id: &str, name: &str, default: fn() -> serde_json::Value) -> Result<serde_json::Value>;
```

Some harness config is neither identity nor composition — it's a
first-run/cosmetic preference (theme, TUI mode) the harness's own onboarding
wizard would normally set, which an always-fresh ephemeral dir never runs.
`Harness::templates()` declares which JSON files need which keys gap-filled;
`provision` calls `harness::apply_templates`, which reads each value via
`template()` (seeding it from `default()` on first read) and shallow-merges
it in — anything the run already generated wins. FS impl:
`harness::FsTemplateStore` (`from_default()`, rooted at `AM_TEMPLATES`). See
[profiles.md](./profiles.md) §14.

**`session::SessionStore`** — run history:

```rust
fn start(&self, meta: SessionMeta) -> Result<Box<dyn SessionRecorder>>;
fn list(&self) -> Result<Vec<SessionMeta>>;
fn load(&self, id: &str) -> Result<SessionMeta>;
fn read_transcript(&self, id: &str) -> Result<Vec<AgentEvent>>;
// SessionRecorder: id(), record_event(&mut self, &AgentEvent), finish(self: Box<Self>, exit_code)
```

FS impl: `session::FsSessionStore`/`FsSessionRecorder`, writing
`<sessions-root>/<id>/{meta.json,transcript.jsonl}` (rooted at
`AM_SESSIONS`). A database `SessionStore` persists the same `SessionMeta` +
`AgentEvent` stream to rows instead of files.

## 6. Other embedder seams

- **`mcp::McpService`** (feature `inproc-mcp`) — implement `tools()` +
  `call(name, arguments)`, wrap it in an `InProcessMcpHandle`, and register
  it as `McpRef::InProcess` to expose a custom, embedder-hosted MCP server —
  not a subprocess from the catalog. `provision` starts a loopback HTTP
  server backed by it and rewrites the entry to a normal inline MCP before
  handing the spec to the harness. See `src/mcp/mod.rs` and
  [architecture.md](./architecture.md).
- **`io::IoBridge`** — structured I/O: drive the agent via input events
  (ACP/JSONL) and receive neutral `AgentEvent`s instead of a raw tty. See
  [io-modes.md](./io-modes.md).
- **`harness::Harness`** — the trait for adding a new harness entirely
  (`id`, `command`, `provision`, `config_anchor`, `templates`, `io_support`).
  Not a storage seam, but the other axis to extend along. See
  [architecture.md](./architecture.md) §"The `Harness` trait" and
  `_docs/harness/`.

## 7. The credential copy-in / copy-back lifecycle

At launch, after `harness.provision(spec, dir)` writes native config,
`provision::provision` runs two more steps: `overlay::materialize(dir,
&spec.config_bases)` symlinks-else-copies the profile's non-credential
overlay on top (leaf wins, never clobbers an `am`-written file), then
`harness::seed_login` **copies** — never symlinks — the captured-login files
from the account's `Source` into the relocated config dir, because the
harness rewrites some of them in place (OAuth refresh).

That copy is one-directional. There is **no copy-back yet**: a token
refreshed inside the run dir is discarded on cleanup, and the next run
re-seeds the older token, forcing another refresh. `AccountStore::
capture_login` and `ProfileStore::put_base` are the intended write seams to
close this loop — a database-backed store would persist a refresh through
the same trait method a filesystem store uses. See
[open-points.md](./open-points.md) §9.

## 8. A concrete embedder example

A minimal in-memory `AccountStore` returning credential bytes instead of a
home directory, driven through `resolve` + `provision`:

```rust
use agent_manager::account::{Account, AccountStore};
use agent_manager::{Result, Source};

struct MemAccountStore { creds: Vec<u8> } // e.g. loaded from your own DB row

impl AccountStore for MemAccountStore {
    fn accounts(&self) -> Result<Vec<Account>> {
        Ok(vec![Account { id: "work".into(), ..Default::default() }])
    }
    fn login_source(&self, id: &str) -> Result<Option<Source>> {
        if id != "work" { return Ok(None); }
        Ok(Some(Source::Files(vec![(".credentials.json".into(), self.creds.clone())])))
        // login_home/capture_login stay at their read-only-error defaults.
    }
}

let flags = agent_manager::resolve::RunFlags {
    harness: "claude-code".into(),
    account: Some("work".into()),
    cwd: std::env::current_dir()?,
    ..Default::default()
};
let settings = agent_manager::settings::Settings::default();
let registry = agent_manager::registry::FsRegistry::new("/path/to/catalog");
let accounts = MemAccountStore { creds: b"{...}".to_vec() };
let profiles = agent_manager::profile::EmptyProfileStore;

let spec = agent_manager::resolve::resolve(&flags, &settings, &registry, &accounts, &profiles)?;

let harness = agent_manager::harness::Claude::new();
let templates = agent_manager::harness::FsTemplateStore::from_default();
let provisioned = agent_manager::provision::provision(&harness, &spec, &templates)?;
// provisioned.dir now has .credentials.json written from MemAccountStore's bytes,
// exactly as if it had come from a real ~/.claude directory.
```

`spec.account_login` already holds the `Source::Files` `MemAccountStore`
produced by this point — `provision` never asks it anything again.
