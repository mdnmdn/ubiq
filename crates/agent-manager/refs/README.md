# refs/ — external reference projects

This directory vendors **external projects as git submodules** so they can be
read as primary sources when writing `agent-manager` documentation and design
notes. Nothing here is built, linked, or shipped — it is reference material
only.

## Contents

| Submodule  | Upstream                              | Why it's here                                                                 |
|------------|---------------------------------------|-------------------------------------------------------------------------------|
| `multica/` | `git@github.com:multica-ai/multica.git` | A daemon/backend that orchestrates many agent harnesses (Claude Code, Codex, Copilot, Cursor, opencode, Gemini, …). Read for how a real system launches harnesses, streams their output, injects models/skills/MCP, and delegates tasks. |

## Working with the submodules

```bash
# First checkout / after a fresh clone of the parent repo:
git submodule update --init crates/agent-manager/refs/multica

# Pull the latest upstream commit (when you deliberately want to re-sync):
git -C crates/agent-manager/refs/multica fetch origin
git -C crates/agent-manager/refs/multica checkout <commit>
```

The pinned commit is recorded by the parent repo; bump it intentionally, not
incidentally.

## How findings flow out of here

Insights extracted from a reference project are written into the normal docs:

- Runtime / orchestration techniques (headless invocation, output streaming,
  launch-time model / skill / MCP / hook injection) are folded into the
  per-harness docs under [`_docs/harness/`](../_docs/harness/).
- A standalone architectural write-up of how multica itself is structured lives
  in [`_docs/reference/multica.md`](../_docs/reference/multica.md).

Reference docs cite `path/to/file:symbol` against the submodule tree so a reader
can jump straight to the source.
