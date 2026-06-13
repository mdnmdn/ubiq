# Harness documentation structure

This file defines the canonical structure for every `_docs/harness/<id>.md`
file in this directory. Each harness document **must** follow the section
ordering and the rules below. The `agent-manager` sync engine and any
contributor editing these docs can rely on a consistent shape.

## File naming

- Filename: `<id>.md`, where `<id>` matches the stable id used in code
  (see `src/harness.rs::Harness::id`).
- Display name and vendor live in the first three lines of the file
  (the *Header* block below), not in YAML frontmatter — these are humans
  docs, not machine config.

## Section ordering (mandatory)

Every harness doc **must** use exactly these H2 sections, in this order.
A section may be omitted only with an explicit "Not supported" note
explaining why (e.g. Gemini CLI has no per-agent memory above `AGENTS.md`).

1. **Header** — a three-line block: `# <Display name>`, blank, then
   `Stable id: \`<id>\``, `Display name: <display name>`, `Vendor: <vendor>`.
2. **Quick reference** — a one-row table of the same four fields as the
   Header, plus the global + project config roots. Lives directly under
   the Header so it survives table-of-contents collapse.
3. **On-disk layout** — an ASCII tree of the harness's directories and
   files, separated into "Global" and "Project" groups. One fenced code
   block per group.
4. **Discovery precedence** — an ordered list of the layers the harness
   reads, highest precedence first. Include the precedence rules
   (override vs merge, arrays concatenate, scalars replace, etc.).
5. **Feature matrix** — a four-row table:

   | Feature   | Support | Where it lands |
   |-----------|---------|----------------|
   | Rules     | full / partial / n/a | `<global path>` and/or `<project path>` |
   | Skills    | full / partial / n/a | `<global path>` and/or `<project path>` |
   | MCP       | full / partial / n/a | `<config root> -> <key path>` |
   | Agents    | full / partial / n/a | `<global path>` and/or `<project path>` |
   | Slash commands | full / partial / n/a | `<global path>` and/or `<project path>` |
   | Auth          | full / partial / n/a | `<global path>` and/or `<project path>` |
   | Permissions | full / partial / n/a | `<config root> -> <key path>` |
   | Policies / Rules | full / partial / n/a | `<memory file>` and/or `<config root> -> <key path>` |

   "Support" is the unified `agent-manager` view of how completely the
   harness's native feature can be expressed via the sync engine; not
   a statement about the harness's own capability.
6. **Skills** — locations (global + project), file/dir shape, format
   (e.g. YAML frontmatter keys), a minimal example, and any
   discovery / invocation notes.
7. **Sub-agents** — locations, format, frontmatter / schema, minimal
   example. If the harness has no sub-agent concept, write a one-paragraph
   "Not supported" note with a link to where the concept is replaced
   (e.g. opencode plugins, Codex skills).
8. **MCP servers** — locations, transport variants (stdio / sse / http),
   minimal example for each, the exact JSON / TOML key the server is
   nested under (`mcpServers`, `mcp_servers.*`, `mcp`, etc.), and any
   per-server fields the harness adds (env, headers, oauth).
9. **Slash commands** — built-in catalogue (one-line each is enough for
   the most common ones) and the custom-command locations / format.
   If the harness treats slash commands as aliases of skills (e.g.
   Claude Code, opencode), say so explicitly and point at the Skills
   section.
10. **Authentication** — every supported auth method (API key,
    OAuth, cloud-provider delegation such as Bedrock / Vertex /
    Foundry, custom proxy / gateway, BYOK), where the credential
    lives, the env vars and `settings.json` keys that control it,
    the auth precedence order, how to switch between multiple
    accounts, and headless / CI patterns. **This is a mandatory
    section** — every supported harness has at least one auth
    method, and almost every rendering decision depends on which
    one is active. Group by auth method (each its own H3) when
    there are more than one; put the precedence order and
    troubleshooting at the end of the section.
11. **Permissions** — locations, rule syntax, the actions or decisions
    the harness supports (`allow` / `deny` / `ask`, or whatever the
    harness calls them), evaluation order, and any sandbox or approval
    mode that is conceptually part of "what may the agent do?".
12. **Policies / Rules / Memory** — the harness's always-on instruction
    system: file(s) that are prepended to the system prompt on every
    turn. Cover global, project, and any subdirectory walk rules.
13. **Format quirks / gotchas** — bullet list of non-obvious behaviours
    an external sync tool must respect. Every entry should be
    actionable: "do X, not Y" or "X is true, not Y".
14. **Renderer notes (planned)** — what `agent-manager`'s sync engine
    for **this** harness needs to do. Numbered list, one operation per
    item. May include a list of files the renderer does **not** own
    (must be left untouched) and a list of files the renderer **does**
    own (may be overwritten wholesale).
15. **Sources** — a bulleted list of the official documentation URLs
    used to write this doc, each with a one-line annotation of what
    was found there. Group by section of the doc if it helps.

## Voice and style

- **Imperative where possible** ("`agent-manager` writes…", "load order is…").
- **One concept per bullet** in the gotchas section. Don't combine two
  quirks in one bullet.
- **All paths are POSIX-style**, even on Windows harnesses, unless the
  harness's own docs use Windows paths. Quote the path exactly as the
  vendor docs do when referencing a primary source.
- **All examples must be runnable** (i.e. they parse, even if the
  surrounding system would reject the literal values). If a value is
  only an example (`your-api-key`), call it out.
- **Frontmatter keys are quoted in backticks** (`name`, `description`)
  and listed in a table when there are more than two.
- **Vendor-specific terminology is kept verbatim** (`mcpServers` in
  Claude Code vs `mcp_servers.*` in Codex vs `mcp` in opencode).
  Don't silently rename a key to match another harness.

## Cross-cutting concepts to keep consistent

These concepts are supported by most harnesses; refer to them in the
same way in each doc:

- **Skills** — A reusable, on-demand package of instructions and (sometimes)
  supporting files. The Agent Skills open standard
  (<https://agentskills.io>) is used by Claude Code, Codex, Gemini CLI,
  opencode, and Copilot. Default shape: a directory with a `SKILL.md`
  whose YAML frontmatter carries at least `name` and `description`.
- **Sub-agents** — A specialised persona with its own system prompt,
  optional tool allowlist, and (often) its own model. Invoked either
  by the main agent (`Agent` / `task` / `@name`) or by the user.
- **MCP servers** — Model Context Protocol tool providers, configured
  per-server with stdio (subprocess) or HTTP (streamable) transports.
  Vendor keys differ (`mcpServers` / `mcp_servers.*` / `mcp`) but the
  underlying transport shape is the same.
- **Slash commands** — `/name` invocations, either built-in or
  custom. Some harnesses (Claude Code, opencode) fold custom slash
  commands into the Skills concept.
- **Permissions** — Per-tool or per-command allow/deny/ask rules.
  Often paired with a sandbox mode.
- **Policies / Rules / Memory** — Always-on instruction content that
  the harness prepends to the system prompt on every turn. The
  concrete filename varies (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`,
  `copilot-instructions.md`).
- **Authentication** — how the harness proves identity to the model
  provider. Methods: API key in env or settings file, OAuth flow
  (browser-based), cloud-provider delegation (Bedrock, Vertex,
  Foundry, Azure OpenAI), custom proxy / gateway (LiteLLM, OpenRouter,
  self-hosted), BYOK (VS Code Copilot's "use your own API key"). The
  precedence order and storage location differ per harness but the
  conceptual surface is the same: pick a method, supply credentials,
  switch between accounts/profiles.

## Updating a harness doc

When the vendor changes their on-disk format, edit the doc in this order:

1. Update **Sources** with the new URL you read.
2. Update **On-disk layout** first (the most concrete artefact).
3. Update **Discovery precedence** and **Feature matrix** to match.
4. Update the per-feature section (**Skills**, **MCP servers**, etc.).
5. Update **Format quirks / gotchas** — a new feature often implies a
   new gotcha.
6. Update **Renderer notes (planned)** with what the sync engine now
   needs to do (and any tasks it must stop doing).

Do **not** invent features the vendor has not documented. If the
vendor's docs are silent on something, mark it as "Not documented
as of `<date>`" with the URL you checked.
