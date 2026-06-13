# Ubiq — Wireframes (wireframe-opus)

Desktop UI wireframes for Ubiq, the harness multiplexer. Authored in the
[simple Excalidraw format](../../simple-excalidraw-spec.md); each screen ships as
`*.excalidraw.yaml` (source), `*.excalidraw` (native, open in Excalidraw), and `*.png` (preview).

A persistent **left sidebar** (dark) is present on every screen: brand, an **Add session**
button, the list of running **sessions**, and a pinned **⚙ Settings** entry at the bottom.

## Screens

| # | File | What it shows |
|---|------|----------------|
| 1 | `01-projects` | Project launcher — grid of existing projects (manage / open) plus a **New Project** form that includes the **agent picker** (claude / opencode / gemini / codex / copilot). |
| 2 | `02-session` | Session view with a **single main agent** filling the work area. The agent pane has the **two-row title**: row 1 = badge + workspace + status, row 2 = folder · model · context. Terminal body + prompt line below. |
| 3 | `03-subagents` | The main agent has spawned subagents, so **side panes** appear (codex/tests, gemini/docs). Every pane keeps the same two-row title. |
| 4 | `04-settings` | Settings page (reached via the sidebar). Sub-nav + **Agents** section listing the agent types from `agents.toml` with enable toggles. |
| — | `all-in-one` | All four screens on **one board** in a 2×2 grid, with flow arrows (create / open → add agent → settings). Generated from the four sources by `_merge.py`. |

Regenerate the combined board after editing any screen:

```bash
uv run --with pyyaml python _merge.py        # writes all-in-one.excalidraw.yaml
uv run ../../../_tools/excalidraw.py to-image -i all-in-one.excalidraw.yaml -o all-in-one.png --scale 2
```

## Rendering

```bash
# from this directory
uv run ../../../_tools/excalidraw.py to-image      -i 02-session.excalidraw.yaml -o 02-session.png --scale 2
uv run ../../../_tools/excalidraw.py to-excalidraw  -i 02-session.excalidraw.yaml -o 02-session.excalidraw
uv run ../../../_tools/excalidraw.py validate        -i 02-session.excalidraw.yaml
```

Note: the PNG preview renderer uses a plain font, so decorative glyphs (`◆ ＋ ⚙ 📁`) show as
boxes in the `.png`. They render correctly when the `.excalidraw` file is opened in Excalidraw.
