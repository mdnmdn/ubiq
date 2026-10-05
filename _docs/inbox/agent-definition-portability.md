---
id: inbox-agent-portability
title: Proposal — one agent identity, every harness, Ubiq-spawned or harness-spawned
kind: proposal
status: proposal
summary: Make an agent definition behave the same whether Ubiq spawns it as its own process or a harness spawns it as an in-process subagent, on any harness. The definition splits into an identity (description, instructions, skills, grants, delegates) and a placement (harness, account, model, mode). The identity travels as a rendered brief in the opening prompt, the one carrier both spawn paths share. The agent's link to Ubiq — the binding — is a `ubiq` CLI plus a skill, not per-harness MCP configuration, because a child process inherits it on every harness. Every CLI call is authorised on the host by a per-agent token minted at spawn and injected in the environment, checked against the caller's process tree, and resolved to the agent's grant — per function, `allow`, `ask` or `deny`. An agent-spawned agent gets the intersection of its definition's grant and its parent's. In-harness subagents share their parent's process and therefore its grant; narrowing it is advisory, and a hard limit means spawning a separate agent. Each fork is a numbered decision with a recommendation.
read_when: you are deciding what an agent definition carries, how an identity reaches a harness or a harness subagent, how an agent talks to Ubiq, or how Ubiq limits which tools and MCP functions a spawned agent may call
updated: 2026-10-05
depends_on: [inbox-agents-definitions, wip-agent-setup, tech-agent-manager, inbox-agent-graph-final, inbox-mission, inbox-hitl-dialogs]
---

# Proposal — one agent identity, every harness, Ubiq-spawned or harness-spawned

Today an agent definition (`AgentDefinition`, `crates/ubiq-proto/src/messages.rs:3819`, backed by the
library's `Profile` in `crates/agent-manager/src/profile.rs`) is pinned to one harness, its
instructions are a file path only a hand edit of `profile.toml` sets (`G89`), and no other agent can
use it. Its MCP servers are written into each harness's own configuration — six writers with six sets
of quirks (`G377`–`G379`, `G383`). Every harness meanwhile has its own agent-file format
(`.claude/agents`, `.codex/agents/*.toml`, `.gemini/agents`, `.opencode/agents`, `.github/agents`),
close to each other and compatible with none; no cross-harness standard for them exists.

**The goal: the same behaviour whether an identity is spawned by Ubiq** — a hosted agent, its own
harness process, user- or agent-requested — **or by a harness** as one of its own subagents. And it
must hold on every harness. This proposal gets there by refusing per-harness translation entirely:
one carrier for the identity, one channel to Ubiq, one place where authorisation happens.

The three original topics map as:

1. **Share the initial knowledge** → the identity brief (§2).
2. **Share with other agents** → `ubiq agent brief` / `ubiq agent spawn` through the skill (§4).
3. **Different MCP functions from the parent** → grants, enforced on the host by the env token (§5).

## 1. Identity and placement

| Part | Fields | Portable? |
|---|---|---|
| **Identity** — who the agent is | `name`, `description`, `instructions`, `skills`, `grants`, `delegates`, an optional `model_tier` hint | yes |
| **Placement** — where it runs | `harness`, `account`, `model` (per-harness map), `mode`, `thinking`, `isolate`, hooks | no |

The identity lives in **agent.md** in the definition's directory (`agent-definitions/<id>/` under the
config root, or the project equivalent): YAML frontmatter for the identity fields, the body for the
instructions — the shape four of five harnesses already use, so a hand-copied Claude or opencode
agent file mostly just works. `profile.toml` keeps the placement.

Placement is a default a spawn may override (`ubiq agent spawn reviewer --harness codex`). A
definition with no harness pin is identity-only and takes its placement from the spawn or the
project default (`G31`); `Agents::infos` (`crates/ubiq-host/src/agent.rs:682`) stops skipping it.
`model` is the one straddling field: the placement keeps a per-harness map, the identity at most a
`model_tier = "fast" | "deep"` hint the placement resolves.

**Binding** in this document means the agent's channel to Ubiq (§3) — not the placement.

## 2. The identity brief — carried in the opening prompt

`brief(X)` renders an identity into one text block: the instructions, the skill names, what the
agent may delegate to, how to reach Ubiq, and one closing line — *if you lose track of your role,
run `ubiq agent brief X`*.

It is carried in the **opening prompt**, ahead of the goal. That is the only carrier both spawn paths
have: a harness's subagent tool takes a prompt and nothing else; a system prompt or an agent file is
something only Ubiq can set, and only per harness.

| Spawned by | How the brief arrives |
|---|---|
| Ubiq (user or `ubiq agent spawn`) | the host sends `brief(X) + goal` as the first turn, through the existing opening-prompt path |
| A harness, as its subagent | the parent runs `ubiq agent brief X --goal "…"` and passes the output as the subagent's prompt |

Same text, same behaviour. Three layers, each with one owner:

- **Project knowledge** — the repository's `AGENTS.md`, read natively by the harness; Ubiq never
  writes it. For a harness that does not read it (Claude Code, unverified), `brief` adds
  *read and follow ./AGENTS.md*.
- **Identity** — the brief.
- **Goal** — the spawn's text, today's `prompt` field.

Costs, accepted: a brief weighs less than a system prompt, and a long session's compaction can wear
it away — the closing line lets the agent re-fetch it. Automatic delegation by description, which
native agent files give a harness, is replaced by the skill telling the agent to `ubiq agent list`
(D5). Shared paragraphs across identities are skills, not a new include mechanism.

## 3. The binding — a CLI and a skill, not MCP

The agent reaches Ubiq through a `ubiq` CLI client, taught by one skill, `ubiq-agents`:

- **Inherited everywhere.** `PATH` and the environment reach every child process on every harness;
  an MCP connection reaches a harness subagent only if that harness passes it on.
- **One writer, not six.** No per-harness MCP config, no `--strict-mcp-config` interaction, no
  shared-home collision: the launch adds one environment variable and the skill.
- **Cheaper context.** A skill loads on demand; MCP tool schemas sit in context all session.
- **Credentials stay in the host.** The database connection lives in `ubiq-db`; the CLI only relays.

The CLI is a thin client over a local socket to the running host, speaking the existing message set:

```
ubiq agent list | brief <id> | spawn <id> --goal … [--harness …]
ubiq tool list
ubiq tool <server>.<function> [args]
```

`ubiq tool` relays to the MCP servers Ubiq already hosts (`use-task`, `use-mission`, `ubiq-kb`, the
DB servers); they stay as they are. MCP injection keeps working during the transition and is retired
per server once its functions are reachable through the CLI (D8).

Nothing like this exists today: the `ubiq` binary is the application, plus `--serve` for a remote
host (`crates/ubiq-app/src/lib.rs`). The client is the main new piece (D7).

## 4. Sharing with other agents

`delegates: [id, …]` (or `"*"`) is the allow-list of identities an agent may hand work to. Two ways,
the two process-ownership cells of `inbox-agents-definitions` §1:

| | **In-harness subagent** | **Hosted agent** |
|---|---|---|
| How | `ubiq agent brief X --goal …` → the harness's own subagent tool | `ubiq agent spawn X --goal …` |
| Process | the parent's | its own harness and pane, launched **by the host** |
| Harness | the parent's — only the identity travels | X's placement, or an override: **the cross-harness path** |
| Grant | the parent's (§5.5) | its own (§5.1) |
| Ubiq sees it | as a logged `brief` call — enough to label it (`G122`) | as a full agent: killable, resumable, on the Teams graph |

The skill states when to use which: in-harness for cheap, same-model work; hosted for another
harness, another model, a narrower grant, or anything long-running. `ubiq agent spawn` is the same
contract as `agents.spawn` in `inbox-agent-graph-final` §8, over the CLI; `may_spawn` and
`max_children` (`inbox-agents-definitions` §1) still gate it.

The registry's import (`crates/agent-manager/src/registry/import.rs`, today skills and MCP servers
from `~/.claude` and `./.claude`) also reads native agent files from the five locations into
**agent.md** identities — one-shot, no sync.

## 5. Authorisation — the env token

### 5.1 At spawn

Whoever asks — the UI or an agent through `ubiq agent spawn` — the host:

1. creates the agent and resolves its **grant** (§5.3);
2. mints a fresh random token and records `token → agent id, grant`;
3. launches the harness itself, with `UBIQ_AGENT_TOKEN=<token>` in its environment, and records the
   root PID and its start time once it is up.

The host launches it, not the requesting agent, so a spawned agent is never a child of its spawner
and never inherits the spawner's environment or token.

### 5.2 On every call

The CLI decides nothing; it forwards the call and the token from its environment. The host:

1. resolves the token to an agent — unknown or missing token, lowest grant;
2. checks the socket's peer PID (`LOCAL_PEERPID` on macOS, `SO_PEERCRED` on Linux,
   `GetNamedPipeClientProcessId` on Windows) descends from that agent's recorded root — a token
   copied into a file and used from another agent is refused;
3. checks the function against the grant: `allow` runs it, `ask` raises a host confirm
   (`inbox-hitl-dialogs`) and runs or refuses on the answer, `deny` refuses with a *not granted*
   error;
4. logs the call against the verified agent.

`ubiq tool list` is filtered by the same grant: an agent without `sql.execute` never sees it, as
today an instance without an MCP server does not have it.

### 5.3 The grant

Per function, not per server, in the identity's frontmatter:

```yaml
grants:
  sql.query: allow
  sql.execute: ask
  kb.*: allow
  "*": deny
```

Most specific pattern wins; the default for anything unlisted is `deny`. A dangerous function —
`sql.execute` — is worth `ask` even for an identity allowed to use it. The host may change a live
agent's grant; it applies from the next call, with no relaunch.

### 5.4 Agent-spawned agents cannot escalate

```
grant(child) = grant(definition) ∩ grant(parent)
```

A `reviewer` holding `sql.query` that spawns a `db-writer` gets a `db-writer` with `sql.query` and no
`sql.execute`. Granting more than the parent holds takes the user's confirmation at spawn. A spawn
from the UI has no parent: the child gets exactly its definition's grant.

### 5.5 What the token cannot do

- **An in-harness subagent shares its parent's process, environment and token.** Its grant is its
  parent's; `ubiq agent brief` may name a narrower one and the CLI may honour it as an advisory
  scope, but nothing at the OS level tells the two apart. **The enforced limit is per process; a
  subagent that must not hold a function is spawned as a hosted agent.** Today's per-instance MCP
  injection has the same property — a Claude subagent inherits its parent's MCP tools.
- **It does not guard what Ubiq does not hold.** An agent that can read database credentials from a
  file can run `psql` itself. Keeping credentials only in the host is what makes the grant mean
  something.
- **Remote and sandboxed placements.** The peer check must run where the harness runs: on a remote
  host or a drone, the local relay does it and forwards the verified agent id. A sandbox that hides
  the process tree falls back to the token alone.

## 6. Decisions

| # | Fork | Recommendation | Cost |
|---|---|---|---|
| D1 | Where the identity lives | **agent.md** beside `profile.toml`, which keeps the placement | A loader change; `defaults.instructions` becomes a fallback |
| D2 | Unpinned definitions | Yes — placement from the spawn or the project default | `Agents::infos` stops skipping them; a "choose at spawn" harness value |
| D3 | Identity carrier | The brief in the opening prompt, everywhere; no system-prompt reinforcement until a case proves it is needed | One renderer |
| D4 | Native agent files | Not written. Import only (§4). Generation stays a later optimisation if description-driven delegation proves to matter | None now |
| D5 | Discovery | The `ubiq-agents` skill points at `ubiq agent list`; a per-run generated skill description naming the delegates is the fallback if agents miss them | One static skill |
| D6 | Authorisation | Env token minted at spawn, verified against the peer PID's process tree, grant per function with `allow` / `ask` / `deny`, intersection on agent spawn | Token table, peer-PID lookup per platform, grant matcher |
| D7 | The CLI client | A subcommand of the `ubiq` binary over a local socket, speaking the existing message set; a separate small binary only if the GPUI dependency makes startup too slow | The main new piece |
| D8 | MCP injection | Kept during the transition, retired per server once reachable through `ubiq tool` | Two paths for a while |

## 7. Build order

1. **P1** — **agent.md**, `instructions` on the wire, the editor field (closes `G89`'s instructions
   half); `brief(X)` sent as the opening prompt of a Ubiq-spawned agent.
2. **P2** — the CLI client with `agent list | brief`, the `ubiq-agents` skill, the env token minted
   at launch. In-harness subagents with Ubiq identities work from here, on every harness.
3. **P3** — `ubiq tool` relaying to the hosted MCP servers, grants, the peer-PID check, `ask` through
   the host confirm. The dangerous functions move first.
4. **P4** — `ubiq agent spawn` with grant intersection, unpinned definitions, the import.
5. **P5** — retire per-harness MCP injection server by server.

All harness knowledge that remains — launching, the opening prompt, adding one env var and one skill
— stays in `crates/agent-manager`.

## 8. Open questions

- Claude Code's current `AGENTS.md` support — decides whether `brief` adds the read line for it.
- Whether the peer-PID walk survives every placement's isolation (`isolate`, the confine shim, the
  drone), or which of them need the token-only fallback.
- Whether `ubiq agent brief` from inside a hosted agent should record the in-harness subagent it
  produces as a reported child on the Teams graph, or only in the log.
- How a long-running grant change reaches the agent's own picture of itself — the brief it holds
  still lists the old grant until it re-fetches.

## Related docs

- [`agents-definitions.md`](./agents-definitions.md) — the seven dimensions; this proposal fills the
  definition dimension, and its two sharing paths are the hosted and internal cells
- [`../wip/agent-setup.md`](../wip/agent-setup.md) — the landed profile and what the library cannot
  deliver yet
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — harness launch, skills and MCP injection
- [`backlog/agent-graph-final.md`](./backlog/agent-graph-final.md) — §8, the `agents.spawn` contract
  `ubiq agent spawn` carries
- [`hitl-registered-dialogs.md`](./hitl-registered-dialogs.md) — the host confirm `ask` raises
- [`../backlog.md`](../backlog.md) — `G31`, `G89`, `G122`, `G377`, `G378`, `G379`, `G383`
