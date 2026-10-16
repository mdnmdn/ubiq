---
id: inbox-policy-registry
title: Proposal — sandbox policy registry, selector and editor
kind: proposal
status: proposal
summary: Every confined run gets the same policy because `compose_run` hardcodes an empty layer name — this proposes a Ubiq policy that is an unmodified isol8 layer, a local registry laid out exactly as isol8's own registry feature lays one out (Ubiq writing the index isol8 leaves to CI), a resolution chain from launch pick to definition to host default, a selector in the agent form, an editor with a form mode and a raw TOML mode over one buffer, a dry-run preview, and the path to isol8's online registries later.
read_when: you are making two agents run under different sandbox policies, editing or adding an isol8 layer from Ubiq, or deciding how Ubiq consumes an isol8 registry
updated: 2026-10-16
depends_on: [inbox-isolation, inbox-isol8-upstream, tech-agent-manager, tech-architecture, tech-transport, tech-ui, feat-workbench, feat-workbench-agents, backlog]
---

# Proposal — sandbox policy registry, selector and editor

**Every confined agent runs under one policy, and nothing in Ubiq can name another.** The sequel to
the isolation proposal: that one settled *whose home* a confined run gets; this one settles *what it
may reach*, per agent.

> A Ubiq policy is an isol8 layer, byte for byte. Ubiq adds a place to keep layers, a way to pick
> one, and a screen to write one — never a schema of its own.

## 1. Where it stands

`Agents::compose_run` in `crates/ubiq-host/src/agent.rs` overrides the resolved spec's isolation
with `Isolation::Sandboxed(String::new())` whenever the host's `isolate_agents` is on. The empty
payload means `isolate::plan` in `crates/agent-manager/src/isolate.rs` adds no layer of its own: the
run gets isol8's `base` and system-runtime layers, the fifteen `DEV_LAYERS`, whatever `agents/*`
layer auto-selects on the harness executable, and the grants Ubiq computes (project folder, run
dir, `PATH` dirs, `HostSettings.extra_grants`, a launch's `AgentPicks.grants`).

The mechanism for more already exists and is unused:

| Piece | What it does today | Where |
|---|---|---|
| `Isolation::Sandboxed(String)` | A non-empty payload is pushed onto `default_profiles` after `DEV_LAYERS` | `crates/agent-manager/src/spec.rs`, `isolate::plan` |
| `ProfileIsolate` | `isolate = false \| "<layer>"` in a definition's `profile.toml`; resolved by the library | `crates/agent-manager/src/profile.rs`, `resolve.rs` |
| `save_definition_replacing` | Keeps a hand-written `isolate` across a save from the form | `crates/ubiq-host/src/agent.rs` |
| `<root>/isol8/profiles` | Already a `profile_paths` entry for every run: a TOML dropped there is a selectable layer | `isolate::plan` |
| `IsolateSettings.profile` | The library's own CLI default layer; Ubiq never reads it | `crates/agent-manager/src/settings.rs` |

Then `compose_run` throws the resolved isolation away. The definition's `isolate` is resolved and
overwritten; `AgentDefinition` on the wire has no field for it (`G89`); `AgentPicks` documents
"Isolation is not here and never will be". This proposal reverses that sentence, deliberately: a
policy *is* something a start chooses, the same way a model is. The host toggle stays what it is — the
master switch.

The isolation proposal's "chat stays unconfined" is stale: structured runs are confined too. Nothing
here distinguishes the two.

## 2. The isol8 policy format

isol8 v0.4.0 at the rev `crates/agent-manager/Cargo.toml` pins, read through the `refs/isol8`
checkout: `refs/isol8/crates/isol8-core/src/profile.rs`. One struct, `Profile`, is both the authored layer
and the merged result, and every table is `deny_unknown_fields` — a typo is a load error, not a
silently ignored key.

| Key | Type | Meaning | Merge across layers (`profile::merge`) |
|---|---|---|---|
| `requires` (alias `extends`) | `[string]` | Layers pulled in transitively, deps first | `resolve_requires`: DFS topo-sort, cycle is an error |
| `filter` | `ProfileFilter {os, arch, executables}` | Layer applies only when every axis matches; a non-empty `executables` makes it **auto-selectable** | — |
| `paths` | `[PathGrant {path, access, match}]` | `access`: `none` `ro` `rw` `metadata`; `match`: `subpath` (default) `literal` `prefix` `regex` | Keyed by `(path, match)`; the highest layer wins, `none` included |
| `env` | table | Default environment | First writer wins |
| `home_replace` | `HomeReplace {enabled, auto_scratch, path, seed}` | Replacement `$HOME` | Highest layer wins; seeds unioned |
| `rewrite` | `Rewrite {ensure_args}` | Args inserted after `argv[0]` | Unioned |
| `macos` | `MacosExtra {capabilities, raw}` | `Capability` (kebab-case: `mach-lookup`, `process-fork`, `pseudo-tty`, …) and verbatim SBPL | Capabilities unioned, `raw` concatenated |
| `windows` | `WindowsExtra {capabilities}` | Parsed, not applied | Unioned |
| `[[policies]]` | `Policy {filter, paths, macos, windows}` | Conditional blocks inside one layer | Folded by `filter::apply_policies` |

Placeholders: `~` and `#HOME` expand against the *effective* home; `%VAR%` on Windows. A layer's id
is its path relative to the profile root, extension dropped — `profiles/ubiq/review.toml` under a
root is the layer `ubiq/review`. `LayerRegistry::load` reads builtins (74 embedded), then the user's
`~/.config/isol8/profiles` (`LayerSource::UserConfig`), then each `profile_paths` entry in order
(`LayerSource::ProfilePath`); a later same-named layer replaces an earlier one.

```toml
# ubiq/review — read the repo, write nothing outside the run dir
requires = ["integrations/git"]
paths = [
  { path = "~/.npmrc", access = "none", match = "literal" },
  { path = "~/src/shared-fixtures", access = "ro" },
]
env = { CI = "1" }

[macos]
capabilities = ["pasteboard"]
```

Five properties shape everything below:

- **A policy only adds.** `isolate::plan` pushes the named layer *after* `DEV_LAYERS`, so it outranks
  them on an identical `(path, match)` key and can carve a `none` out of one. It cannot remove a
  capability (unioned) or a `raw` block, and it cannot narrow a key it does not restate. Auto-selected
  `agents/*` layers are pushed after explicit ones (`select_layer_names`), so they outrank the policy.
- **Any layer with `filter.executables` joins every matching run uninvited.** A file in any profile
  path that says `executables = ["claude"]` confines every Claude run, picked or not.
- **One unparsable file fails every run.** `load_toml_tree` propagates the first parse error, so a
  broken file anywhere under a profile path is a spawn error for all confined runs.
- **`profile::format_layer` drops comments** — it is `toml::to_string_pretty`.
- **What is absent:** no network table (the `network` layer is a no-op alias; the n0–n3 tier schema
  in `refs/isol8/_docs/profile-model.md` is a target, and a `network` key fails to load); no exec
  allowlist beyond the `process-exec` capability and path grants; no JSON schema, no `schemars`.

## 3. The registry on disk

isol8's `registry` feature (`refs/isol8/crates/isol8-registry/src/lib.rs`, design in
`refs/isol8/_docs/registry.md`)
already defines a registry: a directory with `registry.toml` (`RegistryManifest`, with a
`TrustConfig`), a generated `index.json` (`RegistryIndex` of `IndexEntry`), and artifact files.
`DirSource::open` reads one. The local Ubiq registry is that layout and nothing more:

```text
<config root>/isol8/registries/local/
  registry.toml          name = "local", [trust] level = "local"
  index.json             regenerated by Ubiq on every save
  profiles/ubiq/<name>.toml
<config root>/projects/<project id>/isol8/registries/local/
  registry.toml          name = "project", [trust] level = "local"
  index.json
  profiles/ubiq/<name>.toml
```

- **The layer file is a plain `Profile`.** No Ubiq key inside it — `deny_unknown_fields` would
  reject one anyway, and the file must stay loadable by a bare `isol8 --profile-path`.
- **Metadata lives in the index.** `IndexEntry` carries `id`, `kind = "profile"`, `os`, `file`,
  `summary`, `tags`, `requires` and `sha256`. That is the whole record: the id is the name shown, the
  summary is the description. Trust is per registry (`RegistryManifest.trust`), not per entry. There is
  no `title` field and Ubiq adds none — `IndexEntry` is not `deny_unknown_fields`, so an extra key
  would load, but isol8 would never write it back, and the index must round-trip through isol8's own
  tools.
- **Ubiq is the index generator.** isol8 expects `index.json` to come out of a registry's CI. For the
  local registry Ubiq is that CI: every save rewrites the index from the files, hashing each with
  `DirSource::hash_file`. A hand edit is allowed; `ListPolicies` runs `verify_content_hashes` and
  regenerates on a mismatch, so the index is derived state and never the source of truth.
- **The `ubiq/` namespace is a directory.** Because a layer id is the relative path, every Ubiq
  policy is `ubiq/<name>` for free, never collides with a builtin (`base`, `toolchains/*`,
  `integrations/*`, `agents/*`, `apps/*`), and can never shadow one. The editor refuses an id outside
  `ubiq/` — shadowing a builtin is the one thing a profile path permits and a registry editor must not.
- **Project scope shadows global.** A project's `ubiq/review` replaces the global one inside that
  project only — the precedent `ScopedProfileStore` sets for definitions and `OverlayRegistry` sets
  for the MCP and skill catalog. Removing a project's directory takes its policies with it.
- **`<root>/isol8/profiles` stays** as the unindexed drop folder `plan` already loads; the registry
  does not replace it, and the list shows its layers as `drop-in`, read-only.

agent-manager's `_docs` has no policy-registry plan: its `registry` module is the MCP/skill catalog
(`Registry`, `CatalogStore`, `FsRegistry`, `OverlayRegistry`). The registry design here is isol8's;
the catalog is the Ubiq-side model for scoping and overlay only.

## 4. Logic

### 4.1 Resolution for one run

Highest first; the first that answers wins.

| # | Source | Where it lives | Values |
|---|---|---|---|
| 0 | Master switch | `HostSettings.isolate_agents` | Off → `Isolation::None`, the rest is not consulted |
| 1 | Launch pick | `AgentPicks.policy`, `StartConversation.policy` | `PolicyChoice` |
| 2 | Agent definition | `ProfileIsolate` in `profile.toml`, on the wire as `AgentDefinition.policy` | `Off` / `Named` |
| 3 | Host default | `HostSettings.default_policy` | layer id or none |
| 4 | Fallback | — | `Sandboxed("")`: today's behaviour |

```rust
pub enum PolicyChoice {
    /// Whatever the definition and the host default say.
    Default,
    /// Unconfined, for this run only.
    Off,
    /// A layer id: `ubiq/review`, `agents/claude-code`, a registry's `vendor/x`.
    Named(String),
}
```

`compose_run` stops hardcoding the payload and asks one function, `Agents::policy_for`, which walks
the table and returns an `Isolation`. Rank 2 is read from the library's resolved spec (it already
honours `ProfileIsolate`), so the change is "stop overwriting it" plus ranks 1 and 3. The library's
own `IsolateSettings.profile` stays the CLI's concern; Ubiq passes `Settings::default()` and keeps
doing so.

`HostSettings` gains `default_policy: Option<String>`; `HOST_SETTINGS_SCHEMA` goes from 23 to 24,
`#[serde(default)]`, so an old file reads as "no default" and nothing changes for anyone who never
opens the screen. The resolved id and the layer file's sha256 go on `ConversationRecord`
(`crates/ubiq-host/src/conversation_record.rs`), so a revive after a restart reruns under the same
policy and can say so when the file changed underneath — the same gap `G409` records for grants, and
the same fix.

### 4.2 Sources, precedence, materialisation

isol8's `LayerRegistry` does not consume registries — only builtins, the user dir and
`profile_paths`. So materialising a registry is adding its `profiles/` root to
`isol8::Config.profile_paths`, the mechanism `isolate::plan` already uses for the generated dotnet
layer and `<state_dir>/profiles`. `IsolateOptions` gains `profile_paths: Vec<PathBuf>`, appended after
those two, and the host fills it in this order (later wins on a name, so later is higher):

1. builtins (implicit)
2. installed online registries, from their pinned cache (section 6)
3. the global local registry
4. the project's local registry, when the run has a project

Because every policy id is under `ubiq/` and online ids are checked at install, the order matters
only for project-over-global, which is exactly the shadowing wanted.

Two of the format's properties from section 2 become rules here:

- **A registry policy may not carry `filter.executables`.** Validation rejects it on save, and the
  index regeneration marks an offending hand-edited file broken. Otherwise a policy picked for one
  agent auto-joins every run of that executable. `os` and `arch` filters are fine.
- **A broken file is refused at the door, not at the spawn.** Save validates; a hand-broken file
  shows red in the list. A confined run that still meets one fails with isol8's error naming the file
  — it never falls back to unconfined. The upstream fix is in section 7.

### 4.3 The library's `policy` module

agent-manager gains `src/policy.rs`, core like `isolate.rs` (isol8 is a non-optional dependency of
the crate, so there is no isol8 feature to hide it behind), and the isol8 dependency gains
`features = ["registry"]`. That feature adds `isol8-registry` and nothing heavy — serde, toml,
serde_json — and buys `DirSource`, `RegistryIndex`, `RegistryManifest`, `TrustConfig`, `Lockfile` and
`update_registry` for section 6 instead of a vendored copy. The comment on the dependency in
`crates/agent-manager/Cargo.toml` that says `registry` is not needed is the one line to rewrite.

| Function | Does |
|---|---|
| `open(root, name)` | Creates `registry.toml` and an empty index on first use, then `DirSource::open_with_trust` |
| `list(&registries)` | Index entries of every source plus `LayerRegistry::list_for_host` for builtins, each tagged with its source and whether it is editable |
| `read(id)` | The layer's text, its parsed `Profile`, and the form projection (4.5) |
| `save(registry, id, text, replaces)` | Validate, `write_atomic`, regenerate the index |
| `delete(registry, id)` | Remove the file, regenerate the index |
| `validate(text, stack)` | See 4.4 |
| `preview(...)` | See 4.4 |

`ubiq-host` already has `write_atomic` (`crates/ubiq-host/src/atomic.rs`); the library writes its own
files the way its profile store does. Ubiq never names a path inside the registry except its root,
the same posture as the definitions directory.

### 4.4 Validation and preview

Validation, in order, each failure a `PolicyError` with the message isol8 produced:

1. Parse as `isol8::profile::Profile` — serde names the bad key and the line.
2. Id is `ubiq/<name>` with a name a file can carry.
3. No `filter.executables`.
4. Load a `LayerRegistry` over the would-be `profile_paths` with the draft in place, then
   `resolve_requires` on it — a missing layer or a cycle fails here.
5. No layer in the closure is in `isolate::BROKEN_LAYERS` — those fail the whole SBPL policy at
   spawn.
6. Optionally, the registry's `TrustConfig` ceiling: `forbidden_paths` and
   `max_grant_outside_home`. isol8 applies these at install only; for the local registry they are
   advisory warnings, for an online one (section 6) they are the install gate.

Preview answers "what would this agent actually get": the host builds the same `Launch` and
`RunSpec` a real start would (agent type, project, home, grants), with the draft's id as the
`Isolation::Sandboxed` payload, runs `isolate::plan`, then `isolate::describe` — which is
`isol8::sandbox::dry_run_in` with an explicit `Context`. `DryRun` gives the layer stack with each
layer's `LayerOrigin` (explicit, auto, required), the merged `Profile`, the env, the rewritten
command, the rendered native policy (`policy`, labelled by `policy_label` — SBPL on macOS) and the
home plan. Nothing is written or spawned; the preview is exact rather than a re-implementation,
which is the rule isol8's `resolve::effective_policy` comment states about itself.

### 4.5 Wire messages

In the style of the definition family. The UI never reads, parses or writes a policy file; it holds
text and a form projection, and the host does the TOML.

| Message | Carries | Answered by |
|---|---|---|
| `ListPolicies { project }` | — | `Policies { policies: Vec<PolicyInfo>, capabilities: Vec<String> }` |
| `PreviewPolicy { source, agent_type, project }` | `PolicyBody` or an id | `PolicyPreview` |
| `SavePolicy { policy, body, replaces }` | `PolicyInfo` (id, scope, summary, tags), `PolicyBody` | `Policies` or `PolicyError` |
| `ClonePolicy { id, new_id, project }` | — | `Policies` or `PolicyError` |
| `DeletePolicy { id, project }` | — | `Policies` or `PolicyError` |

- `PolicyInfo`: id, `PolicySource` (`Builtin`, `DropIn`, `Local`, `Project(ProjectId)`,
  `Registry { name, trust }`), summary, tags, requires, sha256, `editable`, `broken: Option<String>`.
  Builtins are in the list because the `requires` picker needs them.
- `PolicyBody`: `Text(String)`, or `Form { base: String, draft: PolicyDraft }`.
- `PolicyDraft`: the form-editable subset — `requires`, `paths` (path, access, match as strings),
  `env`, `home_replace`, `macos.capabilities`, `filter.os`. Capability names come from
  `Policies.capabilities`, so the proto names no isol8 enum.
- `PolicyPreview`: the text, the `PolicyDraft` projection plus `lossless: bool` (false when the file
  holds `[[policies]]`, `raw`, `rewrite` or comments the form cannot carry), validation findings, and
  the `DryRun` fields flattened to strings.

`AgentPicks` and `StartConversation` gain `policy: PolicyChoice` (`Default` by serde default);
`AgentDefinition` gains `policy: Option<PolicyChoice>` and `save_definition_replacing` writes it into
`profile.toml` as `isolate`, which closes the isolation part of `G89`. `tech/transport-contract.md`
owns these shapes once they land.

## 5. The interface

Built from the kit as it stands. The defaults do the work: a user who never opens any of this keeps
today's policy and never sees the word "layer".

### 5.1 Picking a policy

`ui/new_agent.rs` `body` draws both the launch form and the definition form, so one row serves both:
a `Picker` (`PickerStyle` of its sibling rows, through `picker_row`) labelled **Sandbox policy**,
items grouped by `separators` — *Default (…)* naming what it resolves to, *Off*, then Ubiq
policies, then registry policies, then builtins worth picking (`agents/*` excluded: they auto-apply).
Each item's `details` is the summary; a broken one is `disabled`. With `isolate_agents` off the row is
disabled with the tooltip `policies_button` already uses.

The naming clash: `policies_button` opens `policies_modal` (`Layer::NewAgentPolicies`) and says
**Custom policies**, but it edits extra folders (`AgentPicks.grants`), not a policy. It becomes
**Extra folders**; `NewAgentForm.policies` (the modal-open flag) becomes `folders`. "Policy" then
means one thing on screen.

```text
 Agent       [ claude-code     v]   Account [ work      v]
 Model       [ sonnet          v]   Mode    [ default   v]
 Sandbox     [ ubiq/review     v]   Extra folders · 2
```

### 5.2 The Policies list in Isolation settings

`ui/settings.rs` `isolation` keeps the master toggle and `home_cards`/`grant_chips`, and gains two
rows: a `setting_row` **Default policy** with a `Picker` (writes `HostSettings.default_policy`
through `remember_host_settings`, beside `AppState::toggle_isolate_agents`), and a **Policies**
block drawn with `settings_split` — list left, detail right — the shape `agent_definitions` uses.

```text
 Policies                                   [+ New]
 ┌──────────────────────┬──────────────────────────────────────┐
 │ ● ubiq/review  local │ ubiq/review              [Duplicate] │
 │ ● ubiq/web     proj  │ Read the repo, write nothing else.   │
 │ ○ vendor/ci   comm.  │ requires  integrations/git           │
 │ ─ builtins ───────── │ [ Form | TOML | Preview ]            │
 │   base               │ ...                                  │
 │   toolchains/rust    │                     [Delete] [Save]  │
 └──────────────────────┴──────────────────────────────────────┘
```

Rows carry a `status_dot` (ok, broken) and a `state_chip` for the source. Builtins, drop-ins and
registry entries are read-only and offer **Duplicate to edit**, which is `ClonePolicy` into
`ubiq/<name>`. Delete goes through `confirm_modal` with `danger: true`; a refusal is `error_banner`,
as for definitions. When the detail outgrows the pane it opens in `modal_sized`.

### 5.3 The editor: one buffer, two modes

A `tab_strip` over **Form**, **TOML** and **Preview**. The buffer is the text; the form is a view of
it.

- **Form.** `requires` as a `MultiPicker` over the `Policies` list; `paths` as rows of path field,
  access `Picker` (`none` `ro` `rw` `metadata`) and match `Picker`, each removable; `env` as key/value
  pairs with `removable_tag`; `home_replace` as a `check_box` plus fields; macOS capabilities as a
  `check_box` column; `filter.os` as `choice_pill`s. Saving sends `PolicyBody::Form { base, draft }`
  and the host applies the draft onto the base text with `toml_edit`, the way isol8's
  `wizard::rewrite_managed` patches a config while keeping unknown keys, so comments and the keys the form does not show survive.
  When `PolicyPreview.lossless` is false the form opens read-only above a banner pointing at TOML.
- **TOML.** A `gpui_component` `InputState` with `ui::editor::highlighter_language(FileLanguage::Toml)`,
  as `app/editor.rs` sets up a file editor. Errors from `PreviewPolicy` (debounced on edit) show
  under it with the line serde named.
- **Preview.** Pick an agent type and, optionally, a project; shows the layer stack with origins, the
  merged path table, the env and the rendered SBPL in a read-only text view.

### 5.4 Seeing it afterwards

A pane's footer gains a `state_chip` with the active policy id — `default` faint, `off` in the
warning group, a named one plain — with the full stack in its tooltip, beside the harness·account
chip the isolation proposal placed. It answers "which question to ask" when a harness reports a
denial; it explains nothing.

## 6. Online registries, later

isol8's design, adopted unchanged; Ubiq adds a screen and a place to cache.

- **Order of backends.** `RegistrySpec::Path` (a folder or checkout the user names — works today),
  then `RegistrySpec::Git` (`update_registry` clones into `<cache>/<name>/<sha>/`; Ubiq passes
  `<config root>/isol8/cache` as the cache root so nothing lands in `~/.cache/isol8`), then
  `RegistrySpec::Http` once isol8 implements it (`refs/isol8/_docs/inbox/evo-repo.md`: static tarball plus
  index, `registry:ns/id@version`).
- **Declaration.** A `[registries.<name>]` table in Ubiq's own settings, parsed by
  `parse_registry_spec`, not isol8's config file — Ubiq never reads `~/.config/isol8/isol8.toml`.
- **Pinning.** An `isol8.lock` (`Lockfile`, `LockRegistry`, `LockEntry`) under
  `<config root>/isol8/`. Runs read only the pinned cache through `open_offline`; nothing fetches at
  spawn. `verify_lock_against_disk` runs at list time.
- **Install, update, diff.** **Update** fetches, `diff_index` against the lock, and shows the diff;
  **Install** writes the lock. The diff is the trust gate: a profile that grants outside the
  `TrustConfig` ceiling, carries `filter.executables`, carries `raw` SBPL, or whose id collides with a
  builtin or another registry is flagged, and a `community` or `untrusted` registry's flagged entry
  needs an explicit accept.
- **Trust.** `TrustLevel` per registry, from the manifest or the config override. No signing exists
  upstream; until it does, a git pin is the integrity guarantee and the UI says "pinned", never
  "verified".
- **Editing.** Registry entries are read-only; **Duplicate to edit** copies one into `ubiq/`, and
  `requires` lets a local policy extend a registry one without copying it.

## 7. What isol8 owes upstream

Short; the evidence format belongs in the isol8 upstream note.

1. `LayerRegistry` taking a `ProfileSource` (or `LayeredSource`) so registries are first-class
   instead of `profile_paths` entries, with registry-qualified ids.
2. Per-file load errors reported and skipped for layers that are not selected, as the recipe loader
   already does, instead of one bad file failing every run.
3. A way to mark a layer not auto-selectable from a profile path, or to load a path explicit-only.
4. `format_layer` preserving comments, or a `toml_edit`-based layer writer in core.
5. The capability ceiling at resolve time (isol8's registry design lists it as a limit).
6. A JSON schema for `Profile`, so the form and any external editor share one definition.

## 8. Staged delivery

Each stage ships alone and leaves the tree consistent.

1. **The chain.** `Agents::policy_for`, `compose_run` honouring the definition's `isolate`,
   `HostSettings.default_policy` (schema 24), `AgentDefinition.policy` on the wire, and the
   **Sandbox policy** picker in the definition form over builtins and the drop-in folder. No editor.
2. **The local registry.** `policy.rs`, the `registry` feature, the five messages, the Policies
   list with the TOML mode and validation. The picker lists `ubiq/*`.
3. **Preview.** `PreviewPolicy` through `isolate::describe`, the Preview tab.
4. **Form mode.** `PolicyDraft`, the `toml_edit` patch, the lossless flag.
5. **Launch pick and provenance.** `AgentPicks.policy`, `StartConversation.policy`, the footer chip,
   the policy id and sha on `ConversationRecord`, revive reading them. The **Extra folders** rename.
6. **Project scope.** The project registry and its place in `profile_paths`.
7. **Online registries.** Path, then git, then HTTP when isol8 has it; lockfile, update, diff.

## 9. Source references

| Concept | File | Items |
|---|---|---|
| Layer format | `refs/isol8/crates/isol8-core/src/profile.rs` | `Profile`, `PathGrant`, `Access`, `MatchKind`, `ProfileFilter`, `HomeReplace`, `Rewrite`, `MacosExtra`, `Capability`, `WindowsExtra`, `Policy`, `LayerSource`, `LayerRegistry::load` / `list_for_host`, `select_layer_names`, `resolve_requires`, `merge`, `format_layer` |
| Filters | `refs/isol8/crates/isol8-core/src/filter.rs` | `RunContext`, `apply_policies`, `apply_layer_filter`, `is_auto_selectable` |
| Dry run | `refs/isol8/crates/isol8-core/src/sandbox.rs`, `resolve.rs` | `DryRun`, `dry_run_in`, `effective_policy_in`, `LayerOrigin` |
| isol8 config | `refs/isol8/crates/isol8-core/src/config.rs` | `Config.profile_paths`, `Config.default_profiles`, `Config::builtin_defaults` |
| Registry | `refs/isol8/crates/isol8-registry/src/lib.rs` | `TrustLevel`, `ArtifactKind`, `IndexEntry`, `RegistryIndex`, `RegistryManifest`, `TrustConfig`, `RegistrySpec`, `ProfileSource`, `DirSource`, `LayeredSource`, `open_offline`, `update_registry`, `diff_index`, `Lockfile`, `verify_lock_against_disk`, `parse_registry_spec` |
| Comment-preserving edit | `refs/isol8/crates/isol8-cli/src/wizard.rs` | `rewrite_managed` |
| Run isolation | `crates/agent-manager/src/isolate.rs` | `plan`, `describe`, `IsolateOptions`, `HomeMode`, `DEV_LAYERS`, `BROKEN_LAYERS` |
| Isolation model | `crates/agent-manager/src/spec.rs`, `profile.rs`, `resolve.rs` | `Isolation`, `ProfileIsolate`, `ScopedProfileStore` |
| Catalog precedent | `crates/agent-manager/src/registry/` | `Registry`, `CatalogStore`, `FsRegistry`, `OverlayRegistry` |
| Dependency | `crates/agent-manager/Cargo.toml` | the `isol8` entry |
| Host composition | `crates/ubiq-host/src/agent.rs` | `Agents`, `compose_run`, `isolate_options`, `set_isolate`, `save_definition_replacing` |
| Revive record | `crates/ubiq-host/src/conversation_record.rs` | `ConversationRecord` |
| Atomic write | `crates/ubiq-host/src/atomic.rs` | `write_atomic` |
| Wire | `crates/ubiq-proto/src/messages.rs` | `AgentPicks`, `StartConversation`, `SpawnWorkspace`, `AgentDefinition`, the `*AgentDefinition*` family |
| Host settings | `crates/ubiq-proto/src/settings.rs` | `HostSettings`, `HOST_SETTINGS_SCHEMA` |
| Settings screen | `crates/ubiq/src/ui/settings.rs`, `app/settings.rs` | `isolation`, `agent_definitions`, `definition_form`, `error_banner`, `toggle_isolate_agents`, `remember_host_settings` |
| Agent form | `crates/ubiq/src/ui/new_agent.rs`, `state/new_agent.rs` | `body`, `picker_row`, `policies_button`, `policies_modal`, `NewAgentForm` |
| Kit | `crates/ubiq/src/ui/kit/` | `Picker`, `PickerStyle`, `MultiPicker`, `settings_split`, `setting_row`, `modal_sized`, `confirm_modal`, `tab_strip`, `check_box`, `removable_tag`, `state_chip`, `status_dot`, `choice_pill` |
| TOML highlighting | `crates/ubiq/src/ui/editor.rs` | `highlighter_language` |

## 10. What this asks to be decided

- **May a launch pick `Off` while the master switch is on?** Proposed: yes, visibly — the footer chip
  goes to the warning group. A user who can turn the switch off can already do this for every agent.
- **May a Ubiq policy shadow a builtin?** Proposed: no; the `ubiq/` namespace makes it impossible,
  and narrowing a builtin is done with a `none` grant in a policy that `requires` it.
- **Should the definition form show builtins at all?** Proposed: only `apps/*` and the
  `integrations/*` not in `BROKEN_LAYERS`; the rest are already in every run or auto-apply.
- **Where is a policy's display name?** Proposed: the id; `IndexEntry` has no title and adding one
  breaks the round-trip through isol8's tools.
- **Does a changed policy file invalidate a revive?** Proposed: no; the revive runs under the current
  file and the chip's tooltip notes the sha changed.
- **Should Ubiq load the user's `~/.config/isol8/profiles`?** isol8 always does, so a layer there
  with an `executables` filter confines Ubiq runs today. Proposed: list those layers as a read-only
  source with a warning on auto-selectable ones, and ask upstream for a switch (section 7).

## Related docs

- [`./isolation-proposal.md`](./isolation-proposal.md) — the prequel: homes, the Isolation section,
  the footer chip.
- [`./isol8-upstream.md`](./isol8-upstream.md) — where section 7's asks get their evidence.
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary: the library owns isol8, Ubiq
  names no layer path.
- [`../tech/architecture.md`](../tech/architecture.md) — why the UI holds text and never a file.
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the new messages land.
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the kit and the token rule.
- [`../features/workbench-agents.md`](../features/workbench-agents.md) — the agent form and the
  definitions list.
- [`../backlog.md`](../backlog.md) — `G89`, `G91`, `G409`.
