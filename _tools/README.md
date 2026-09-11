# `_tools/`

Dev-only scripts. Nothing here is imported by the crates, and nothing here imports them.

Each script is self-contained: it declares its own dependencies inline, so `uv run` needs no
environment set up. Every one is fronted by a `just` recipe — nobody should have to remember a
path or a flag.

| Script | Recipes | Does |
|---|---|---|
| `docs.py` | `just docs-lint`, `docs-index`, `docs-check`, `docs-drift`, `docs-touched`, `docs-graph` | Maintains `_docs/`: the mechanical checks, the generated catalogue and code map, the drift queue, and the map from a diff to the documents it obliges you to update |
| `excalidraw.py` | `just diagram` | Converts, validates and renders the compact diagram format the wireframes are authored in |
| `icons.py` | `just icons-check`, `icons-sheet`, `icons-variants`, `icons-adopt`, `icons-audit`, `icons-dupes` | Keeps the icon set in `assets/icons/` coherent: the registry against the files, the SVG spec, and the review sheets an agent looks at while drawing one — written to the ignored `assets/icons/preview/` |
| `webassets.py` | `just web-assets`, `web-assets-verify` | Snapshots the Excalidraw offline mirror from jsDelivr — Excalidraw's own `dist/prod` plus the transitive `+esm` closure — and writes the expected SHA-256 of every file, with the import map that collapses the stray React copies, as generated Rust the host verifies downloads against |
| `icns.py` | `just icns`, `just bundle` | Builds the macOS application icon from the logo in `assets/` — the ten representations an `.iconset` needs — through `iconutil` |

What the check ids mean and what to do about each is in `_docs/_meta/librarian.md`. The diagram
format is specified in `_docs/tech/diagram-format.md`. The icon rules and the drawing loop are in
the `ubiq-icons` skill.

## Adding a script

Give it inline `uv` script metadata (PEP 723), add a `just` recipe with a one-line comment above it — that
comment is what `just --list` prints — and add its row to the table above.
