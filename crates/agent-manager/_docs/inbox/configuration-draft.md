# Configuration source of truth — WIP

> **Status: working document.** This is a reasoning scratchpad, not a spec.
> It exists to lay out the design space for "where does the canonical config
> live?" and "how do we propagate it to the harnesses?" before we commit
> to one. Comments, counter-proposals and experience reports welcome.

---

## 1. The decision

`agent-manager` needs to decide, per project, **what the source of truth is**
for rules / skills / MCP / agents. Once that is decided, it also needs to
decide **how the data physically lands** in each harness's config dir
(copy, symlink, render-in-place, ...).

Both choices are independent. Mixing the wrong pair (e.g. using Claude Code
as the SoT but symlinking MCP into Codex) is the kind of thing that looks
fine on day 1 and is a nightmare to debug at month 6.

---

## 2. Source-of-truth options

### Option A — `agent-manager`'s TOML is the SoT (status quo)

```
my-project/
├── .agent-manager.toml        # <-- single source of truth
├── rules/no-secrets.md
├── skills/agent-browser/SKILL.md
└── ...
```

Every other harness's files are *derived* from this file by `agent-manager
sync`. The user edits one file; the tool writes N.

- **Pros**
  - One place to edit. Diffable. Reviewable in PRs.
  - Tool-agnostic: no Claude / Codex / Copilot leakage in the input.
  - Easy to express things that no single harness supports (e.g. an MCP
    server with a hint that "only enable for harnesses X, Y").
- **Cons**
  - Existing users with a hand-curated `~/.claude/CLAUDE.md` or `.codex/AGENTS.md`
    have to migrate. Friction on adoption.
  - We are inventing yet another config format.
  - Power users may want features that are easier to express in the
    native format of their primary harness.

### Option B — Claude Code's config is the SoT

```
my-project/
├── .claude/CLAUDE.md
├── .claude/settings.json
├── .claude/commands/<id>.md
└── .claude/agents/<id>.md
```

We read Claude Code's directory structure verbatim and *project* it onto
the other harnesses. Claude Code users can keep editing `.claude/` as
they always have; `agent-manager sync` picks up the changes.

- **Pros**
  - Zero migration for the largest current user base (Claude Code).
  - Rich feature surface: Claude Code already supports frontmatter, slash
    commands, sub-agents — we get all of that for free.
- **Cons**
  - Couples the model to one vendor's quirks (frontmatter shape, slash
    command naming, the `mcpServers` JSON shape).
  - Users on other harnesses pay the migration cost Claude users avoided.
  - Symmetric principle violated: if a Codex user wants to lead, they
    can't — we'd have to pick a "primary" harness and live with the
    politics.

### Option C — A generic `.agents/` directory is the SoT

```
my-project/
├── .agents/
│   ├── rules/<id>.md
│   ├── skills/<id>/SKILL.md
│   ├── mcp.toml
│   └── agents/<id>.md
```

A vendor-neutral folder that follows a simple, harness-independent
convention. `agent-manager` reads from `.agents/` and renders into every
harness's native layout.

- **Pros**
  - Truly portable. The `.agents/` folder is the same on every project
    regardless of which harness the developer happens to be using today.
  - Human-browsable: you can read `.agents/rules/no-secrets.md` in your
    editor without knowing anything about any specific tool.
  - Aligns with the emerging industry direction of "common agent config
    locations" (e.g. proposals around `.agents/`, `AGENTS.md`, etc.).
- **Cons**
  - Yet another convention. Risk of being a useless middleman if
    `agent-manager.toml` already does the job and the user can just keep
    skills in `skills/` next to their code.
  - File-based with no schema validation unless we layer one on top.
  - Doesn't naturally express cross-cutting concerns
    (e.g. "this MCP server is only enabled for harnesses X, Y").

### Option D — Hybrid / multi-rooted with declared precedence

```
my-project/
├── .agent-manager.toml        # optional override
├── .agents/                   # optional fallback
├── .claude/                   # read if present
├── .codex/                    # read if present
└── AGENTS.md                  # read if present
```

`agent-manager` reads from every root it can find, in a declared
precedence order, and merges the result.

- **Pros**
  - Maximum flexibility. New users opt in gradually.
  - Zero migration for existing users of any harness.
- **Cons**
  - The merge rules are subtle. "If a rule with the same `id` exists in
    both `.claude/CLAUDE.md` and `.agent-manager.toml`, who wins?"
  - Bug magnet. The kind of feature that gets demoed and then
    re-implemented three times.
  - The user can't easily reason about the final state.

### Option E — User picks one, explicitly, per project

The bootstrap is a one-time choice: at the top of `.agent-manager.toml`
(or a separate `.agent-manager.source`), declare which root is canonical.

```toml
# .agent-manager.source
source = "managed"   # options: "managed" | "claude-code" | "codex" | "opencode" | "agents-dir"
```

- **Pros**
  - Explicit is better than implicit. No surprises.
  - The choice is project-scoped, so different repos can pick different
    sources.
- **Cons**
  - One more knob. New users don't know what to pick.
  - Migration is a one-shot per project; not zero-cost.

### Option F — `AGENTS.md` itself is the SoT (see §5.6)

The source of truth for memory is `AGENTS.md` at the project root, with
`@`-references to modular content files. `.agent-manager.toml` is a
sibling for non-textual config. Discussed in detail in §5.6; included
in the comparison table below.

---

## 3. Comparison

| Axis                          | A: managed | B: claude | C: `.agents/` | D: hybrid | E: pick-one | F: `AGENTS.md` |
|-------------------------------|------------|-----------|---------------|-----------|-------------|---------------|
| Zero migration for Claude users | no        | yes       | no            | yes       | yes (if you pick B) | no (need a wrapper) |
| Zero migration for Codex users  | no        | no        | no            | yes       | yes (if you pick C-like) | partial (need a TOML for MCP) |
| Tool-agnostic input            | yes       | no        | yes           | partly    | depends on pick | yes (markdown) |
| Reviewable in PRs              | yes       | yes       | yes           | messy     | depends on pick | yes |
| Expresses cross-cutting concerns (per-harness overrides, etc.) | yes | no | no | yes (kinda) | yes | partial (TOML sibling) |
| Implementation complexity      | low       | medium    | low           | high      | low          | medium         |
| Readable in any editor without our tool | no  | yes       | yes           | yes       | yes          | yes            |

---

## 4. Sync mechanism options

Once we know the SoT, we still have to decide how the data physically
arrives in each harness's config dir.

### Copy (render + write)

We read the source, render the harness-native format, and write a
brand-new file at the target path. The file is owned by `agent-manager`.

- **Pros**
  - Simple. Each harness sees a real file in its real format.
  - No FS surprises: no symlink loops, no cross-device failures.
  - Drift detection: store a checksum or `<!-- agent-manager -->` marker.
- **Cons**
  - Edits the user makes in the harness dir are overwritten on next sync.
    (Mitigated by the marker + a `writable` flag, or by promoting the
    change back into the SoT.)
  - Two copies of the data exist on disk.

### Symlink

The target path is a symlink pointing at a file in the source dir.

```bash
~/.claude/commands/agent-browser.md -> ../../.agents/skills/agent-browser/SKILL.md
```

- **Pros**
  - One copy of the data on disk.
  - Edits to the source are seen by the harness immediately, no `sync`
    required.
- **Cons**
  - Some harnesses resolve symlinks badly or refuse to follow them in
    sandboxed contexts.
  - Confusing in `ls -la`, in git status (need `.gitignore` for the
    symlink targets), in editor project trees.
  - Symlink targets must use relative paths, which break if the user
    moves the project.

### Hard link

`~/.claude/commands/agent-browser.md` and `.agents/...` share an inode.

- **Pros**
  - One copy of the data, no path-resolution issues.
- **Cons**
  - Edits propagate in both directions silently. Surprise.
  - Cross-device moves break.
  - Backup tools that follow or break hardlinks behave inconsistently.
  - Not portable to Windows / non-Unix filesystems.

### Hybrid: render configs, symlink content

Config-format files (`settings.json`, `opencode.json`, `mcp.json`,
`config.toml`) are *rendered* — they need to be in the harness's native
format anyway. Content files (skill bodies, rule bodies, agent bodies)
are *symlinked* — the content is identical across harnesses.

- **Pros**
  - The two operations have different semantics, and the hybrid matches
    that.
  - Edits to a skill body in one harness's folder update everywhere.
  - Config files are still owned by `agent-manager`, so we can reason
    about drift.
- **Cons**
  - Two mechanisms in one tool. Users have to learn the rule
    ("configs are rendered, content is linked").
  - Symlink caveats still apply for content.

---

## 5. The memory model

So far we have treated "the source of truth" as one big blob. In practice,
agent memory is its own sub-problem with its own trade-offs.

### 5.1 What "memory" means here

For our purposes, **memory** is the set of always-on textual instructions
the agent loads at the start of a session — the human-readable "this is
how we do things on this project" document. It is distinct from:

- **Skills** — large, on-demand bodies of expertise the agent loads only
  when relevant.
- **MCP servers** — runtime tools, not text.
- **Sub-agents** — alternate personas, each with their own (smaller)
  memory.

Memory is what every harness loads *first*, every session, so it has to
be small enough to fit in the context window and high-signal enough to
be worth the cost.

### 5.2 The `@`-reference pattern

Claude Code introduced (and other tools have adopted) the convention of
embedding file references in a memory file with an `@` prefix:

```markdown
# AGENTS.md
@rules/no-secrets.md
@rules/no-pii.md
@skills/agent-browser/SKILL.md
```

At session start, the harness reads the memory file, sees each `@`-line,
inlines the referenced file's content, and proceeds. The memory file
itself is a *table of contents*; the actual content lives in modular
files on disk.

Why this matters for `agent-manager`:

- The same `@`-reference works in any harness that implements it. If we
  emit `AGENTS.md` as a list of `@`-lines, the result is small, diffable
  and the human can read it as a manifest.
- It decouples *what* a rule is from *how* it is phrased in a given
  harness's prose. The body of `rules/no-secrets.md` is identical
  everywhere; only the file that references it changes.
- It composes: a rule can itself contain `@`-references to even smaller
  files. Modular all the way down.

**Open question (§6):** which harnesses actually implement `@` semantics
today, and which silently treat `@<path>` as literal text?

### 5.3 `AGENTS.md` vs `CLAUDE.md` vs friends

Different harnesses call their project memory by different names:

| Harness      | Project memory file                     | User memory file           |
|--------------|------------------------------------------|----------------------------|
| Claude Code  | `CLAUDE.md`                              | `~/.claude/CLAUDE.md`      |
| Codex        | `AGENTS.md`                              | `~/.codex/AGENTS.md`       |
| opencode     | `AGENTS.md`                              | `~/.config/opencode/AGENTS.md` |
| GitHub Copilot | `.github/copilot-instructions.md`      | *(none — project only)*    |

Three of four converge on `AGENTS.md` for project memory. Claude Code
uses its own name but the *content shape* is identical. Copilot is the
odd one out: project-only, no user-level file, and the file name encodes
the tool.

Practical implications:

- A single rendered `AGENTS.md` covers Codex + opencode at once.
- Claude Code needs the same content under a different filename.
- Copilot needs the same content under a third filename, in a slightly
  different shape (no `@`-references supported as of this writing).

### 5.4 Granularity: one big file or many small ones?

Two extreme shapes for the rules in the SoT:

**Shape 1: monolith.** All rules inlined into one big markdown file.

```markdown
# AGENTS.md

## no-secrets
Never log secrets. ...

## no-pii
Do not commit PII. ...

## review-style
All PRs require ...
```

**Shape 2: one-file-per-rule.** A directory of small files, referenced
from a thin manifest.

```
rules/
├── no-secrets.md
├── no-pii.md
└── review-style.md
# AGENTS.md
@rules/no-secrets.md
@rules/no-pii.md
@rules/review-style.md
```

| Axis                      | Monolith                 | One-file-per-rule         |
|---------------------------|--------------------------|---------------------------|
| Diff in PRs               | noisy (every rule changed) | surgical                 |
| Reuse across projects     | hard                     | trivial                   |
| Harness compatibility     | works everywhere         | requires `@` support      |
| Read in any editor        | one file                 | need to follow references |
| Discoverability           | see all rules in one go  | need to know to look      |

### 5.5 Inclusion strategy: inline vs reference at sync time

Even if the SoT is one-file-per-rule, at sync time we have to choose how
to materialise it in each harness's memory file:

- **Inline:** the sync engine reads each rule's body, concatenates them
  under section headers, and writes one big `CLAUDE.md` /
  `AGENTS.md` / `copilot-instructions.md`. Self-contained; works in
  every harness.
- **Reference:** the sync engine writes a thin `AGENTS.md` that contains
  only `@<path>` lines. Tiny; requires harness support.
- **Both:** write a thin reference manifest for harnesses that support
  `@`, and an inline-rendered version for harnesses that don't.

For Copilot (which doesn't support `@`), **inline** is the only safe
choice. For Codex / opencode (which support `@`), **reference** is
strictly better but adds a requirement. **Both** is the maximum-compat
option and is the most defensible v1 behaviour.

### 5.6 A new SoT option F: the SoT *is* `AGENTS.md`

The `@`-pattern is so close to what the harnesses already want that
it's worth asking: **what if the SoT for memory is just `AGENTS.md` at
the project root, with `@`-references to content?**

- The file a developer is most likely to open when they want to know
  "how does this project expect me to behave?" is *already* the SoT.
- A user opening the project in Codex or opencode sees the right
  memory file even if `agent-manager` has never been run.
- `agent-manager`'s job shrinks to: "keep `AGENTS.md` in sync with the
  cross-cutting metadata (MCP, agent allowlists) and project it onto
  the other harnesses' flavours of memory file."

This is basically Option C (`.agents/`) with the twist that the entry
point is a file the user can read directly, not a folder they have to
explore.

- **Pros**
  - Zero ceremony. The most natural file in the project *is* the SoT.
  - Works in harnesses without any tool support at all.
  - Reads as a manifest, but is also a real document.
- **Cons**
  - Loses the ability to express things markdown cannot (typed MCP
    config, agent tool allowlists). For those, we still need a sibling
    metadata file.
  - Two file formats to keep aligned: `AGENTS.md` (memory) and
    `.agent-manager.toml` (metadata). Risk of drift between them.
  - The user can edit `AGENTS.md` freely, which means the sync engine
    has to choose between "this file is owned by us, overwrite on sync"
    and "this file is the user's, only update managed sections".

### 5.7 User memory vs project memory

The config-format story is not just about projects. Most harnesses also
load a *user-level* memory file (`~/.claude/CLAUDE.md`,
`~/.codex/AGENTS.md`, ...). Two questions:

1. Does `agent-manager` manage those? My current lean is **no** for v1.
   The tool's job is to keep a project's per-harness state in sync with
   its SoT; user-level preferences are out of scope.
2. Can a rule in the SoT be marked as "user-level, not project-level"?
   Pluggable later if needed.

### 5.8 Edit-in-place vs render-on-sync

If the user opens `~/.claude/CLAUDE.md` and tweaks a section by hand,
what happens on next `agent-manager sync`?

- **Overwrite (render wins):** the SoT is authoritative. Any hand edit
  in the harness file is lost. Predictable but unforgiving.
- **Preserve with markers (sections win):** the sync engine only
  touches content inside `<!-- agent-manager:begin --> ... :end -->`
  markers. Anything outside is the user's. This is what I would
  default to — see Codex's own renderer notes in
  `_docs/harness/codex.md`.
- **Write-back (harness wins):** the tool notices the divergence and
  promotes the user's edit back into the SoT. Magic, dangerous, hard
  to review.

For v1, the right answer is **preserve with markers**. Write-back is a
v3 feature at the earliest, and it should require an explicit flag.

---

## 6. Open questions

1. **Is `.agents/` going to be a *thing* in the industry, or just ours?**
   Worth spending 30 minutes to confirm before we bet on Option C.
2. **Do we ever want to *write back* into the SoT?** E.g. the user
   tweaks a rule in Claude Code's settings; should `agent-manager` ever
   propagate that back to the SoT, or is the SoT strictly read-only
   input from the tool's perspective?
3. **User-level vs project-level.** All of the above is project-level.
   Do we also need a `~/.config/agent-manager/config.toml` for global
   rules / MCP that apply across every project? (Probably yes, but
   separate doc.)
4. **Conflict resolution under hybrid sync.** If a skill's body is
   symlinked but Claude Code has hand-edited the frontmatter, what wins
   on next sync?
5. **Import path.** Whatever the SoT is, do we need a one-shot
   `agent-manager import claude-code` that ingests an existing
   `~/.claude/` tree into the SoT? (Strong yes from the adoption angle.)

---

## 7. Current lean (subject to change)

Updated to reflect the memory-model discussion in §5.

- **Memory SoT = `AGENTS.md` at the project root (Option F).** The file
  humans reach for first *is* the source. `.agent-manager.toml` is a
  sibling metadata file for things markdown can't express (MCP, agent
  tool allowlists, per-harness overrides).
- **Content lives in `rules/<id>.md`, `skills/<id>/SKILL.md`, etc.** —
  one file per concept. The memory file is a manifest of `@`-references
  to those.
- **`agent-manager.toml` is for non-textual config only.** MCP servers,
  agent definitions, harness enable/disable. Anything that is just
  prose lives in `AGENTS.md` / `rules/`.
- **Every harness gets a marker-bounded block.** The sync engine only
  writes between `<!-- agent-manager:begin -->` and
  `<!-- agent-manager:end -->` in the harness's memory file. The user's
  hand-written sections are never touched.
- **For harnesses without `@` support (Copilot), inline the bodies.**
  For harnesses with `@` support, prefer the reference form. Emit both
  shapes from the same source so we don't have to special-case the
  content; only the wrapper does.
- **User-level memory is out of scope for v1.** Only project-level
  `AGENTS.md` and its per-harness siblings are managed.
- **Propagation = copy + render for v1.** Symlink-for-content is a v2
  candidate, gated on user feedback.

Rationale, restated: the SoT should be the most natural file in the
project (`AGENTS.md`), the format most harnesses already want, with
`@`-references keeping the content modular. The metadata envelope
(`.agent-manager.toml`) is for what markdown cannot express. Drift is
controlled with marker-bounded blocks. Anything smarter than that is
a v2 conversation.

**This lean is now subsumed by §10 — see the "Updated lean" at the
end of §10.8.** Everything in the metadata envelope collapses to one
abstraction (`profile`), and §8's three feature axes become facets of
a profile.

---

## 8. Beyond the basic sync — three feature axes

The sync model so far is one project, one set of resources, every
harness gets the same content. Real use cases are messier. The three
features below are the most-requested next steps and each one
multiplies the design space.

### 8.1 Specialised agents inside one project

Today the unified config exposes agents as a flat list:

```toml
[[agents]]
id = "reviewer"
path = "agents/reviewer.md"
```

That treats every agent as a peer of the main agent with the same
toolset. In practice users want **roles** with different tools,
different skills, and different MCPs:

- a *reviewer* that only has read-only tools and a `github` MCP
- a *data-scientist* that has a `python-notebook` skill and a
  `bigquery` MCP but no `github` MCP
- a *docs-writer* that has a `markdown-style` skill but no MCPs at all

The question is: what does *different* mean — does the role
**replace** the project's default set, or does it **narrow** it?

#### Option A — additive subset (recommended starting point)

A role declares the *extra* resources it wants, on top of the
project's defaults:

```toml
[[agents]]
id = "reviewer"
path       = "agents/reviewer.md"
skills     = ["lint-checker", "security-auditor"]   # extras
mcp        = ["github"]                             # extras
inherits   = true                                    # default: yes
```

Effective MCP set for the reviewer agent =
`(project mcp) ∪ (agent mcp)`.

#### Option B — explicit allow-list (replaces project set)

```toml
[[agents]]
id = "data-scientist"
path     = "agents/data-scientist.md"
skills   = ["python-notebook"]
mcp      = ["bigquery"]
inherits = false   # do NOT inherit project MCPs
```

Effective MCP set =
`(agent mcp)` only.

This is more powerful but more dangerous — one forgotten entry
silently drops a resource the user expected.

#### Option C — three-list model (allow / extra / deny)

```toml
[[agents]]
id = "ops-debugger"
path      = "agents/ops-debugger.md"
allow_mcp = ["kubernetes", "prometheus"]   # base set
extra_mcp = []                              # additions
deny_mcp  = ["github"]                      # remove from project default
```

The most expressive. Also the most to type. Best suited for a
v3 once we know the common patterns from A and B.

#### Composition questions

- **Can an agent reference another agent?** A `lead-reviewer` that
  is "the reviewer agent plus the security-auditor skill". Probably
  yes in v2, no in v1 (just duplicate the resources).
- **Can a rule say "only applies to agent X"?** Pluggable later; not
  in v1.
- **What is the unit of inheritance — file, line, or concept?** If
  the project has rule `no-secrets` and the agent lists `skills:
  ["lint-checker"]`, does the agent get the `no-secrets` rule? Lean:
  yes, rules are always inherited, only skills/MCPs are
  opt-in/opt-out.

#### Harness support

| Harness      | Sub-agent concept       | Native scoping                |
|--------------|--------------------------|-------------------------------|
| Claude Code  | yes (`agents/<id>.md`)   | skills + tools allowlist     |
| Codex        | yes (`agents/<id>.md`)   | tools allowlist              |
| opencode     | yes (`agent/<id>.md`)    | skills + tools + mcp allowlist |
| GitHub Copilot | "chat modes" (`.agent.md`) | tools + description          |

The capability to scope MCP at the sub-agent level exists in
opencode and (partly) Claude Code; the others would render a warning
("sub-agent MCP scoping requested, but `codex` doesn't support it")
and apply the resources project-wide.

### 8.2 Auth profiles per harness and per agent

Different harnesses want credentials in different shapes:

- **Claude Code / Codex / opencode** — `ANTHROPIC_API_KEY`,
  `OPENAI_API_KEY`, etc. as environment variables, plus optional
  per-user OAuth flows.
- **Gemini CLI** — Google OAuth (browser flow) or a service-account
  JSON.
- **GitHub Copilot** — GitHub OAuth, often via the `gh` CLI token.

A user with two Google accounts, two GitHub accounts (work + personal),
or two OpenAI orgs, can't easily switch between them inside one
project. The unified config should let them.

#### Auth profile model

```toml
# .agent-manager.toml

[[profiles]]
id      = "work-gemini"
harness = "gemini"
auth    = { type = "oauth", account = "[email protected]" }

[[profiles]]
id      = "personal-gemini"
harness = "gemini"
auth    = { type = "api_key", env = "GEMINI_API_KEY_PERSONAL" }

[[profiles]]
id      = "work-claude"
harness = "claude-code"
auth    = { type = "api_key", env = "ANTHROPIC_API_KEY_WORK" }

[[agents]]
id      = "work-reviewer"
profile = "work-gemini"     # uses Gemini with the work account
path    = "agents/reviewer.md"
```

#### What `agent-manager` actually does with profiles

`agent-manager` is a **config manager**, not a secrets manager. It
should:

1. Declare which profile belongs to which harness / agent.
2. Render the right env-var name, OAuth config block, or `keyring`
   reference into each harness's native auth slot.
3. **Not** store the secret itself, by default. The secret comes from
   the user's shell, their `1Password CLI`, their OS keyring, or a
   well-known path the user controls.

The integration options:

| Source            | How `agent-manager` reads it                           | Risk profile        |
|-------------------|--------------------------------------------------------|---------------------|
| Env var           | passthrough, never written to disk                     | safe                |
| `~/.config/agent-manager/auth.toml` | read at sync time, file mode 0600 | convenient         |
| OS keyring        | `keyring` crate, prompt on first use                   | best practice       |
| `1Password CLI` (`op`) | shell out to `op read op://...`                   | good for teams      |
| `direnv` / `.envrc` | shell out, never persist                             | familiar to devs   |

Lean: support env-var and `~/.config/agent-manager/auth.toml` in
v1; keyring and 1Password in v2.

#### OAuth flows

OAuth is a runtime concern that goes *beyond* `agent-manager`'s
scope. Specifically:

- `agent-manager` should store the *result* of an OAuth flow
  (the refresh token) once the user has completed it.
- The actual browser dance is the harness's job (or the user's job,
  e.g. "go to this URL, paste the code back").
- We may provide `agent-manager auth login <profile>` as a thin
  wrapper that delegates to the harness, but we should not be the
  source of truth for OAuth state.

#### Open sub-questions

- What happens when two agents want two different profiles for the
  same harness *in the same session*? Most harnesses have one global
  "current profile" — the answer is "you can't, the user has to
  pick". We should make that limitation explicit.
- Should `profile` be a property of the agent, the harness, or the
  invocation (CLI flag)? Lean: agent or harness, with a CLI flag
  for one-off overrides.

### 8.3 Per-target enable / disable of skills and MCP

The third axis: the same skill (or MCP) should be available to
*some* harnesses / *some* agents, but not all. Reasons include:

- The MCP's transport isn't supported by every harness
  (e.g. `sse` MCPs that only work in Claude Code).
- A skill is relevant to a sub-agent role but not the main agent.
- A MCP is only safe to expose to read-only roles.
- A skill is licensed for a single team member and shouldn't appear
  in the project default for everyone.

#### Where the filter can attach

There are four attachment points for a filter, in increasing
specificity:

1. **Harness-level** — "this MCP is only enabled for Claude Code".
2. **Agent-level** — "this MCP is only enabled for the reviewer agent".
3. **Harness × agent** — "this MCP is enabled for the reviewer agent
   *running on* Codex".
4. **External context** — "this MCP is only enabled when the
   `FLOOR` env var is `prod`". (Probably out of scope for v1.)

The first two cover ~95% of real use cases.

#### Shape options

**Inline in the resource declaration** (simplest):

```toml
[[mcp]]
id       = "browser"
command  = "npx"
args     = ["-y", "@agent-browser/mcp"]
harnesses = ["claude-code", "opencode"]   # empty/missing = all

[[skills]]
id   = "agent-browser"
path = "skills/agent-browser"
agents = ["data-scientist", "reviewer"]   # empty/missing = all
```

Pros: short, the rule is right next to the resource. Cons: only
supports one filter dimension (harness OR agent), not both.

**Matrix declaration** (richer):

```toml
[[mcp]]
id      = "browser"
command = "npx"
args    = ["-y", "@agent-browser/mcp"]
for     = { harnesses = ["claude-code"], agents = ["data-scientist"] }
```

Pros: composable across both dimensions. Cons: more verbose, and
"harnesses AND agents" vs "harnesses OR agents" semantics need to
be nailed down.

**Separate enable declarations** (most flexible):

```toml
# Declare the resource once
[[mcp]]
id      = "browser"
command = "npx"
args    = ["-y", "@agent-browser/mcp"]

# Declare where it's enabled
[[mcp.enable]]
id        = "browser"
harnesses = ["claude-code"]

[[mcp.enable]]
id        = "browser"
agents    = ["data-scientist"]
```

Pros: can express "enabled for Claude Code" *and* "enabled for
data-scientist regardless of harness" as two independent rules.
Cons: the resource and its enable-list are decoupled — easy to
forget to update one when changing the other.

#### Default behaviour

The conservative default is: **if a filter is present, only the
listed targets get the resource; everything else gets nothing**.
This is the principle of least surprise — the user must opt in to
distribution, not opt out.

A future flag could invert this (`mode = "denylist"`) for the case
where most targets should get a resource and a few should be
excluded.

#### Where the filter lives in the rendered output

The sync engine computes the effective set per `(harness, agent)`
and only writes entries that pass the filter. Nothing about the
filter leaks into the rendered harness config — the user doesn't
see `<!-- agent-manager:filter -->` comments in their
`settings.json`.

### 8.4 How the three features interact

A single concrete example that exercises all three:

> A team has a "data-scientist" sub-agent that runs on Gemini
> with the team's work Google account, uses the `bigquery` MCP
> (only on Gemini because the SSE transport isn't supported on
> Codex yet), and inherits the project's rules but not the
> project's MCPs.

In `.agent-manager.toml` that could be:

```toml
# 8.2: auth profile
[[profiles]]
id      = "work-gemini"
harness = "gemini"
auth    = { type = "oauth", account = "[email protected]" }

# 8.3: per-target enable
[[mcp]]
id      = "bigquery"
command = "mcp-bigquery"
harnesses = ["gemini"]                       # 8.3

# 8.1: specialised agent
[[agents]]
id       = "data-scientist"
profile  = "work-gemini"                    # 8.2
path     = "agents/data-scientist.md"
mcp      = ["bigquery"]                      # 8.1 (additive; project mcp = none)
inherits = true                              # 8.1 (still inherits project rules/skills)
```

The features are orthogonal in the config (each lives in its own
table) but the sync engine has to compose them. That composition
*is* the hard part, and it's where most of the bugs will live.

---

## 9. Updated open questions

In addition to the open questions already listed in §6:

1. **Sub-agent inheritance semantics.** Do rules and skills inherit
   by default? Only some? User-configurable? (Lean: rules + skills
   inherit; MCPs are explicit.)
2. **Profile scope.** Can a profile be a property of the harness
   (default profile per harness) *and* overridden per agent, or is
   it always per-agent?
3. **OAuth delegation.** Do we ship a `agent-manager auth login`
   that wraps the harness's flow, or do we just point the user at
   the harness's docs?
4. **Filter composition.** When a resource has both a `harnesses`
   filter and an `agents` filter, is the effective set the
   intersection (must match both) or the union (matches either)?
   (Lean: intersection, with a `mode = "union"` opt-out.)
5. **Profile secrets in `.agent-manager.toml`.** Are we OK with
   secret *references* (env-var names, keyring paths) living in the
   SoT, given that the file is normally committed to git? Do we
   need a separate `.agent-manager.local.toml` that's gitignored?
6. **Multi-profile sessions.** When the user runs two agents with
   two different profiles in the same session, does `agent-manager`
   refuse, or does the harness pick? (Lean: refuse with a clear
   error; the harness isn't built for it.)

---

## 10. Unifying concept: *profiles*

The three features in §8 (specialised agents, auth profiles, per-target
enable/disable) start to overlap as soon as you draw them on the same
page. The natural unification is to make **`profile`** the central
abstraction, and treat the three features as *different facets* of a
profile rather than as three separate concepts.

### 10.1 The idea

A **profile** is a named, reusable bundle of:

- a **harness binding** (which harness it runs on; absent = any)
- an **auth binding** (how it authenticates; absent = harness default)
- a **persona** (a path to a markdown file with the system prompt;
  absent = not a sub-agent, just a config bundle)
- a **skill set** (additive on top of what the project provides)
- an **MCP set** (additive on top of what the project provides)
- a **rule set** (additive on top of what the project provides)
- a **list of profiles it `extends`** (composition)

With this, every "thing" we have talked about is just a profile with
some fields set and others empty:

| Concept from §8            | Just a profile where…                              |
|----------------------------|-----------------------------------------------------|
| Specialised agent          | `path` is set, optional `extends` for composition  |
| Auth-only profile          | `harness` + `auth` set, nothing else               |
| Harness default            | `id = "<harness>-default"`, `harness` set, no `path` |
| Skill / MCP bundle         | `skills` / `mcp` set, no `harness`, no `path`      |
| Per-target enable (8.3)    | The `harness` field — the profile *is* the filter  |
| The "main" project agent   | The implicit `id = "default"` profile              |

### 10.2 A single, profile-only config sketch

```toml
# ─── project-level (the implicit "default" profile) ─────────────
# Anything declared at the top of .agent-manager.toml is treated
# as belonging to a profile with id = "default".

# Project rules
[[rules]]                       # see config-format.md for shape
id    = "no-secrets"
title = "Never log secrets"
body  = "rules/no-secrets.md"

# Project skills
[[skills]]
id   = "agent-browser"
path = "skills/agent-browser"

# Project MCPs
[[mcp]]
id      = "github"
command = "mcp-github"

# ─── named profiles ─────────────────────────────────────────────

# Auth-only profile, reusable
[[profiles]]
id      = "work-gemini"
harness = "gemini"
[profiles.auth]
type    = "oauth"
account = "[email protected]"

# Auth-only profile, reusable
[[profiles]]
id      = "personal-gemini"
harness = "gemini"
[profiles.auth]
type = "api_key"
env  = "GEMINI_API_KEY_PERSONAL"

# A sub-agent persona
[[profiles]]
id   = "reviewer"
path = "agents/reviewer.md"
skills = ["lint-checker", "security-auditor"]
mcp    = ["github"]
# inherits project rules + project skills by default

# A specialised sub-agent that *replaces* the project MCP set
[[profiles]]
id        = "data-scientist"
path      = "agents/data-scientist.md"
extends   = []                # do not extend "default" implicitly
skills    = ["python-notebook"]
mcp       = ["bigquery"]
[profiles.auth]               # the agent can also pin its own auth
type = "api_key"
env  = "BIGQUERY_KEY"

# Composed profile: persona + harness + auth
[[profiles]]
id      = "work-gemini-reviewer"
extends = ["work-gemini", "reviewer"]
# no path, no skills, no mcp — it's a thin composition

# Harness default that just bundles a different MCP set
[[profiles]]
id      = "opencode-with-bigquery"
harness = "opencode"
mcp     = ["bigquery"]   # additive on top of project default
```

### 10.3 The composition algebra

When a profile `extends` one or more other profiles, the effective
configuration is computed by a small, explicit algebra:

```
effective(profile) =
    union of { rule:  extended.rules  + profile.rules,  last-extended wins on id collision }
    union of { skill: extended.skills + profile.skills, last-extended wins on id collision }
    union of { mcp:   extended.mcp    + profile.mcp,    last-extended wins on id collision }
    auth    = profile.auth  ?? last-extended.auth  ?? none
    path    = profile.path  ?? none                  # personas don't compose
    harness = profile.harness ?? last-extended.harness ?? "any"
```

Three properties worth being explicit about:

- **Last-extended wins on collision.** `extends = ["A", "B"]` means
  B's resources override A's on id collision. The order matters; it
  is the user's responsibility to order from "least specific" to
  "most specific".
- **Personas don't compose.** A profile can have at most one `path`.
  If two extended profiles both set a `path`, that is a config
  error.
- **`extends = []` opts out of inheriting the project default.**
  This is the v1 mechanism for "give me a clean slate, then add
  *only* this list of resources".

### 10.4 What this changes vs §8

| §8 feature             | Before (§8)                                     | After (profile)                                  |
|------------------------|-------------------------------------------------|--------------------------------------------------|
| 8.1 specialised agent  | `[[agents]]` table, separate                    | `[[profiles]]` with `path`                       |
| 8.1 inheritance        | `inherits = true/false` flag                    | `extends = ["…"]` list (or `[]` for clean slate) |
| 8.2 auth profile       | `[[profiles]]` + `profile = "…"` on agent       | Same `[[profiles]]`, but a profile is reusable in `extends` |
| 8.3 per-target enable  | `harnesses = ["…"]` inline on the resource      | The `harness` field on the profile IS the filter |
| Project defaults       | `[[rules]]`, `[[skills]]`, `[[mcp]]` top-level  | Same shape, but conceptually bundled under the implicit `default` profile |

The net effect:

- **One table to learn** (`[[profiles]]`) instead of three
  (`[[agents]]`, `[[profiles]]`, plus per-resource filters).
- **Composition is a first-class concept** (`extends`) instead of an
  ad-hoc `inherits` flag with two values.
- **Auth, persona, resources, and harness binding all live on the
  same row** — easier to read, easier to validate, easier to render.

### 10.5 What this *doesn't* unify

A few things are still outside the profile model and should stay that
way:

- **Free-floating resources.** A `[[mcp]]` declared at the top level
  (i.e. belonging to the implicit `default` profile) is *also*
  implicitly a "library entry" that other profiles can opt into via
  id reference. We do not need a separate `[[mcp.library]]` table.
- **The `AGENTS.md` / `@`-reference memory file.** That is markdown,
  not config. The profile model only covers the metadata envelope
  (`.agent-manager.toml`). The two stay separate by design — see
  the `AGENTS.md` lean in §7.
- **Harness enable/disable.** Whether a harness is *targeted* by
  `agent-manager sync` at all is a project-level toggle
  (`harnesses = […]`), not a per-profile property. A profile can
  *bind* to a harness but cannot *enable* a disabled one.

### 10.6 The naming question

Calling everything a *profile* is intuitive ("switch profile") but
risks collision with the OAuth sense of "profile" (a Google account
profile, a Chrome profile). Two alternative names:

- **role** — clear, but "sub-agent" is the existing vocabulary in
  the harnesses.
- **variant** — neutral, no collision, but bland.
- **persona** — evocative, but only fits profiles with a `path`.

Lean: **profile** is the right name despite the OAuth collision,
because the OAuth sense is *contained inside* the auth field of a
profile — it is the auth profile of the profile, not a profile of
its own. The TUI can label it "Auth" in the auth field's editor
view to disambiguate.

### 10.7 What becomes of `config-format.md`

`_docs/config-format.md` currently documents the four-table shape
(`[[rules]]`, `[[skills]]`, `[[mcp]]`, `[[agents]]`). If we adopt
the profile model, that doc needs a follow-up revision. The likely
shape:

- Keep `[[rules]]`, `[[skills]]`, `[[mcp]]` at the top of the file
  (these belong to the implicit `default` profile).
- Add `[[profiles]]` as a new top-level table.
- **Do not** introduce a separate `[[agents]]` table — a profile
  with a `path` *is* an agent.

Open question: do we keep the `[[agents]]` table as a deprecated
alias for `[[profiles]]` with a `path`, to ease migration of early
configs? Lean: yes, one-version deprecation cycle, then remove.

### 10.8 Updated lean

Replace the "memory SoT" lean in §7 with the broader picture:

- **Memory SoT is `AGENTS.md`** (unchanged from §7).
- **Everything else is profiles.** A profile is a named bundle of
  `harness`, `auth`, `path` (optional), `skills`, `mcp`, `rules`,
  and `extends`. The project's own rules/skills/MCPs belong to the
  implicit `default` profile.
- **No separate `[[agents]]` table** — a profile with `path` *is*
  a sub-agent. (One-version deprecation of the legacy table.)
- **No separate `[[auth]]` table** — `auth` is a field on a profile,
  and auth-only profiles are just profiles with everything else
  empty.
- **Per-target enable/disable falls out for free** — the `harness`
  field on a profile *is* the filter; if a profile binds to harness
  X, it is enabled for X and not for others.
- **Composition is `extends`** — the only way to inherit, override,
  and compose profiles. Order is "least specific first".

This single decision collapses three open feature axes into one
configurable surface. Worth the schema churn.
