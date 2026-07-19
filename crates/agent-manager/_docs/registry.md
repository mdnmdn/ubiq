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

pub struct SkillEntry { pub id: String, pub path: PathBuf, pub meta: SkillMeta }
pub struct McpEntry   { pub id: String, pub def: McpServer /* transport, cmd, env, … */ }
```

`resolve` turns `--skills`/`--mcps` ids into `SkillRef`/`McpRef` by querying the
registry; a missing id is a hard error listing the near matches.

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

Skills are **always** folders (a `SKILL.md` + supporting files), because that is
the portable on-disk shape every harness already understands. The registry
resolves a skill id to its folder and the provisioner copies/links it into the
ephemeral config dir.

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
