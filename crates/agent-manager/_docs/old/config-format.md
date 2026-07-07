# Unified config format

A project opts in by dropping a `.agent-manager.toml` at its root. The file is
read once by `agent-manager` and then projected onto every enabled harness.

## Top-level shape

```toml
[project]
name = "my-app"             # optional
description = "..."         # optional

[[rules]]   ...             # zero or more
[[skills]]  ...             # zero or more
[[mcp]]     ...             # zero or more
[[agents]]  ...             # zero or more

harnesses = ["claude-code", "opencode"]   # optional, default: all known
```

If `harnesses` is omitted, every harness known at compile time is targeted.

## `[[rules]]`

A rule is a short, named chunk of text the agent should always honour. It can
either reference a file or contain the body inline.

```toml
[[rules]]
id    = "no-secrets"      # required, stable
title = "Never log secrets"   # required, human-readable
body  = "rules/no-secrets.md"  # path (relative to the config file) OR inline text
```

## `[[skills]]`

A skill is a folder containing a `SKILL.md` plus any supporting files. The
`path` points at the folder.

```toml
[[skills]]
id   = "agent-browser"
path = "skills/agent-browser"
```

## `[[mcp]]`

An MCP server definition. The `transport` selects the rendering strategy
in the target harness.

```toml
[[mcp]]
id   = "browser"
[mcp.transport]
type = "stdio"             # one of: stdio | sse | http
command = "npx"            # stdio only
args    = ["-y", "@agent-browser/mcp"]
[mcp.env]
LOG_LEVEL = "warn"
```

For `sse` / `http` transports, swap `command` / `args` for the remote URL.

## `[[agents]]`

A sub-agent definition (a role-specific persona, prompt, tool allowlist, etc.).

```toml
[[agents]]
id   = "reviewer"
path = "agents/reviewer.md"
```

## Conventions

- **Paths are project-relative.** Absolute paths are rejected at parse time.
- **Stable `id` values.** Changing a rule's `id` is treated as a delete-then-add.
- **No templating.** Bodies are verbatim. If you need interpolation, do it in
  the rule body itself.
