# The catalog (MCP / skill registry)

The **catalog** is where `am` finds the skills and MCP servers a run can inject.
`--skills web-designer` means "look up `web-designer` in the catalog and inject
it"; `--mcps postgres,figma` means the same for MCP servers.

## Trait first, filesystem second

The catalog is defined as a **trait** so that lib-mode embedders can back it with
whatever they like (a database, a remote service, an in-memory map), and the CLI
gets a filesystem-backed implementation.

```rust
pub trait Registry {
    fn skills(&self) -> Result<Vec<SkillEntry>>;
    fn mcps(&self) -> Result<Vec<McpEntry>>;

    fn skill(&self, id: &str) -> Result<Option<SkillEntry>>;
    fn mcp(&self, id: &str) -> Result<Option<McpEntry>>;
}

pub struct SkillEntry { pub id: String, pub source: Source, pub meta: SkillMeta, pub origin: SkillOrigin }
pub struct McpEntry   { pub id: String, pub def: McpServer /* transport, cmd, env, … */,
                        pub expose: McpExpose, pub summary: Option<String>, pub description: Option<String> }
```

`resolve` turns `--skills`/`--mcps` ids into `SkillRef`/`McpRef` by querying the
registry; a missing id (mcp, skill, account, or hook — the same rule across all four lookups) is
**dropped** rather than failing the run, and a line naming it plus its near matches lands on
`RunSpec::problems` instead. The CLI prints each as a `warning:` on stderr; a lib-mode embedder
reads `problems` off the resolved spec and decides how loud to be — Ubiq surfaces it as a
dismissable notification. `--mcp-as-skill` naming an id outside the run's own injected set, and
`--safe` naming a preset that does not exist, are still hard errors: both are a flag misused at
the call site, not a stale reference living in a saved profile.

**The permission mode degrades the same way, checked against the harness instead of the catalog.**
`resolve` finds the `Harness` impl from `flags.harness` itself — no caller passes one in — and asks
`Harness::accepts_mode`, which answers from the fixed `Harness::modes()` CLI enum and so spawns
nothing. A mode the harness does not know (from `--permission-mode` or from a profile's `mode`) is
dropped, with its near matches, onto `problems`, and the run launches on the harness's own default.
Dropping is the safe direction: a permission mode names a default stance, and every harness's
default asks more rather than less. Two answers are deliberately permissive — a harness whose
`modes()` is empty (opencode, Copilot, Grok's ACP seam) has no permission-mode concept rather than
an empty whitelist, so its value passes through unchecked, and so does the value for a harness id
no impl answers to. Codex overrides `accepts_mode` to keep its documented `restricted` alias for
`read-only`, which `map_sandbox_mode` accepts but `modes()` does not list.

**The model is not validated at all, and that is a decision rather than a gap.** A harness's model
list is not a fixed enum: `Harness::discover_models` shells out to the harness's own binary (Claude
Code's runs a one-shot `claude -p`) and may need the login and the network. There is nothing to
check against at resolve time that is both free and reliable, and a list that failed to load would
silently drop a model that works — worse than no check. An unknown model reaches the harness, which
is the only party that can say whether it is wrong.

## Filesystem-backed layout (CLI mode)

The CLI registry is a **mixed config + folder-structure** store rooted at a path
set by `--catalog` / `AM_CATALOG` / the `catalog =` key / the default
(`~/.config/agent-manager/catalog` or `~/.agent-manager/catalog`).

```
<catalog-root>/
├── catalog.toml              # folder-level config: registry metadata + inline MCPs
├── mcp/                       # one JSON file per MCP server
│   ├── postgres.json          #   { "command": "…", "args": […], "env": {…} }
│   ├── figma.json
│   └── github.json
└── skills/                    # one folder per skill (Agent Skills open standard)
    ├── web-designer/
    │   └── SKILL.md
    └── reviewer/
        └── SKILL.md
```

Two ways to declare an MCP, matching the spec:

- **Single-file MCP** — `mcp/<id>.json`, the standard `{command, args, env}` /
  `{type, url, headers}` shape (same schema the harness docs describe).
- **Inline in `catalog.toml`** — several MCPs declared together, handy for a
  small curated set:

```toml
# catalog.toml
[registry]
name = "personal"

[[mcp]]
id = "figma"
transport = "stdio"
command = "npx"
args = ["-y", "@figma/mcp"]

[[mcp]]
id = "docs"
transport = "http"
url = "https://example.com/mcp/"

[[mcp]]
id = "postgres"
transport = "stdio"
command = "postgres-mcp"
expose = "skill"        # "tools" (default) | "skill" — see mcp-as-skill.md
summary = "Query and inspect a Postgres database."   # seeds the generated skill's description
```

Two extra, optional `[[mcp]]` fields (`catalog.toml`-only — a single-file
`mcp/*.json` entry has no room for them, and always defaults to
`expose = "tools"` / `summary = None`):

- `expose` — `"tools"` (default) injects the MCP as a normal, always-on tool
  set, same as today. `"skill"` additionally causes the provisioner to
  generate a latent `SKILL.md` pointer for the MCP (see
  [`mcp-as-skill.md`](./mcp-as-skill.md)) — as of the schema+pointer pass
  that landed, the MCP still stays injected as normal either way; `expose`
  only controls whether a skill pointer is *also* written.
- `summary` — a one-line description seeding the generated skill's
  `description:` frontmatter when `expose = "skill"`. Ignored otherwise.

`mcp/<id>.json` accepts the same optional `expose`, `summary` and `description` keys next to the
server fields; `description` is the text a catalog browser shows.

Skills are **always** folders (a `SKILL.md` + supporting files), because that is
the portable on-disk shape every harness already understands. The registry
resolves a skill id to its folder and the provisioner copies/links it into the
ephemeral config dir. A layer finds skills in three places (`SkillEntry.origin` says which):

```toml
[[skill]]            # one folder referenced in place (SkillOrigin::Linked)
id = "pdf"           # optional; defaults to the folder name
path = "/abs/path/to/pdf"

[[skill_dir]]        # a folder scanned at list time: every <sub>/SKILL.md is a skill (SkillOrigin::Dir)
path = "/abs/path/to/my-skills"

[[skill_source]]     # a remote place to search and install from (see below)
id = "anthropics"
label = "Anthropic skills"
kind = "git"         # "git" | "index"
url = "https://github.com/anthropics/skills"
rev = "main"         # optional
subpath = ""         # optional: only scan under this path
```

and the copies under `skills/<id>/` (`SkillOrigin::Installed`). On an id clash inside one layer:
installed `skills/<id>` > `[[skill]]` > `[[skill_dir]]` (first folder wins); it is not an error.
A dangling `[[skill]]` or `[[skill_dir]]` (folder gone) is skipped. Dot-prefixed folders under
`skills/` are install scratch space and never listed.

## Managing the catalog: `CatalogStore`

`Registry` is read-only. `CatalogStore: Registry` (`registry/manage.rs`) is the write side:
`link_skill`, `add_skill_dir` / `remove_skill_dir` / `skill_dirs`, `install_skill`, `remove_skill`,
`put_mcp`, `remove_mcp`, `skill_sources` / `set_skill_sources`. `FsRegistry` implements it and is the
override point: Ubiq uses one `FsRegistry` per layer, an embedder may bring its own store.

- Ids are `[A-Za-z0-9._-]+` (`registry::valid_id`); anything else is rejected.
- `catalog.toml` is rewritten through a `toml::Table` (only the edited array changes) and a temp
  file + rename. Comments in the file are not preserved. The root is created on the first write.
- `install_skill` copies into `skills/.<id>.tmp`, then swaps it in; a failed install leaves the old
  copy. `.git` is never copied. When a `RemoteOrigin` is given it is written beside `SKILL.md` as
  `.am-origin.toml` (`url`, `rev`, `path`, `commit`); `Source::materialize` skips that file, so a
  run never sees it.
- `remove_skill`: installed copy -> delete the folder; linked -> drop the `[[skill]]` entry; found
  in a scanned folder -> error ("remove the folder instead").
- `put_mcp` writes `mcp/<id>.json`; an id declared inline in `catalog.toml` is rewritten there.
- `skill_sources()` returns the declared `[[skill_source]]` entries, or `remote::default_skill_sources()`
  (anthropics/skills and huggingface/skills, both git) when none are declared. Setting an empty list
  goes back to the defaults.

## Remote skills (`registry/remote.rs`)

A `SkillSource` is `Git { url, rev, subpath }` or `Index { url }`. Git sources use the `git` CLI
(no extra dependency): `refresh_source` does a shallow clone (or `fetch --depth 1` + `reset --hard`)
into `<cache>/<source id>` with `GIT_TERMINAL_PROMPT=0`. `list_source` refreshes only when the cache
is missing or older than an hour (a failed refresh of an existing cache falls back to it), then walks
it (depth 4 under `subpath`) for `SKILL.md` and reads the frontmatter; a skill inside another skill's
folder belongs to that skill. `search(sources, cache, query)` is a case-insensitive substring match
on id, name and description, and a failing source becomes a problem line, not an error.
`install_remote` copies the folder through `CatalogStore::install_skill` and records the origin.

An `Index` source is an HTTP JSON listing, `[{ "name", "description", "url" (git url), "path", "rev"? }]`
or `{ "skills": [...] }`; it needs the `remote` feature (without it the source yields a problem line).
**No `Index` is among the defaults**: agentskills.io could not be checked for a machine-readable
listing from the build sandbox (its host was blocked), so the parser follows the shape above until
a real listing is known; adding it is one entry in `default_skill_sources`.

## Pasted MCP configs (`registry/mcp_parse.rs`)

`parse_mcp_config(text) -> Vec<McpServer>` reads any harness's dialect, ids from the map keys:
`{"mcpServers"}` (Claude, Cursor, Copilot), `{"servers"}` (VS Code), `{"mcp"}` (opencode:
`type: local|remote`, `command: [cmd, args…]`, `environment`), `{"context_servers"}` (Zed, `command`
plain or `{path, args, env}`), a bare `{id: server}` map, a single server object (id `""`), and
Codex TOML `[mcp_servers.<id>]` (`http_headers` is `headers`). `type`: `stdio`/`local` -> stdio,
`http`/`streamable-http`/`streamableHttp`/`remote` -> http, `sse` -> sse; missing -> stdio with a
`command`, else http. JSON may carry `//` and `/* */` comments and trailing commas.

## The MCP registry (`registry/mcp_registry.rs`, feature `remote`)

`McpRegistryClient` (default base `https://registry.modelcontextprotocol.io`) calls
`GET /v0.1/servers?search=&limit=&version=latest[&cursor=]` (honouring the proxy environment) and
`parse_registry_page` maps each server to `RegistryServer { name, title, description, version,
repository, options: Vec<McpDraft> }`. A `McpDraft { label, server, params }` is one package or
remote: npm -> `npx -y id@ver`, pypi -> `uvx id==ver`, oci -> `docker run -i --rm … id:ver`,
nuget -> `dnx id@ver --yes` (`runtimeHint` replaces the command; runtime and package arguments are
added), `remotes[]` and http/sse package transports -> http/sse servers. Environment variables and
headers become entries with their default or an empty value, each with a `ParamHint { name, kind:
Env|Header|Arg, description, required, secret, default }` so a UI can ask for them.

Both sources merge into one namespace; an id collision between a single-file MCP
and an inline one is a load-time error.

## `am catalog import` — the adoption on-ramp

Importing ingests **well-known agent config directories** into the catalog so
existing skills/MCPs become injectable by id without hand-copying:

```bash
am catalog import                       # scan the default well-known roots
am catalog import --from ~/.claude      # a specific root
am catalog import --dry-run             # show what would be added, write nothing
```

Well-known roots scanned by default (read-only):

| Source                         | Skills read from            | MCP read from                              |
|--------------------------------|-----------------------------|--------------------------------------------|
| `~/.claude/`                   | `skills/<name>/SKILL.md`    | `~/.claude.json` → `mcpServers`            |
| `~/.agent/` (generic)          | `skills/<name>/`            | `mcp/*.json`                               |
| project `./.claude/`, `.mcp.json` | `.claude/skills/…`       | `.mcp.json` → `mcpServers`                 |
| `~/.codex/`, `~/.config/opencode/` | (harness-specific)      | harness config file                        |

Import **copies definitions into the catalog**; it never modifies the source
dirs. On id collision it prompts / requires `--force`, and `--dry-run` prints
the plan without writing.

## Project vs global catalogs — global + project overlay (decided)

There are **two catalog layers**:

- **Global** — the root from `--catalog` / `AM_CATALOG` / the `catalog =` key /
  the default location. Always present.
- **Project** — an optional `<project>/.agent-manager/catalog` discovered by
  walking up from the CWD (same walk as the settings file).

The project catalog **layers on top of** the global one. Its entries can either
**add** new ids or **replace** a global entry of the same id — id collision =
project wins (override), no collision = project entry is simply added. This lets
a repo ship a curated MCP/skill set (e.g. a pinned `postgres` definition) that
overrides the developer's global one for that project, while still inheriting
everything else global.

Mechanically this is two `Registry` instances composed by an
`OverlayRegistry(global, project)` that resolves an id against the project layer
first, then falls back to the global layer, and unions the two for listing.

## Relationship to MCP-as-skill

A catalog MCP entry can be marked to be exposed **as a skill** rather than a raw
tool set, to keep the agent's context lean. That mechanism has its own doc:
[`mcp-as-skill.md`](./mcp-as-skill.md).

## See also: accounts catalog

The **accounts catalog** mirrors the MCP/skill registry shape and lives under
`~/.config/agent-manager/accounts/` (env override: `AM_ACCOUNTS`). Accounts are stored
as TOML files (`accounts.toml` inline `[[account]]` definitions plus per-file
`<id>.toml` entries) and hold credential **references**, never secret material: env-var
names, a base URL, a helper command, and/or a private home directory. When injected
via `--account <id>`, the account's references are resolved into the harness's native
auth slots. See the account section of the `cli.md` for the full CLI (`am account ls|use|import`).
