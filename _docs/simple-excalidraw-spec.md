# Simple Excalidraw Format — Specification

A compact, YAML-based authoring format for [Excalidraw](https://excalidraw.com) diagrams,
designed to be easy for humans and AI agents to write by hand. The companion CLI
(`_tools/excalidraw.py`) converts it to and from the native `.excalidraw` JSON, validates it,
and renders SVG/PNG previews.

## Why

Native `.excalidraw` files carry ~25 bookkeeping fields per element (`seed`, `versionNonce`,
`version`, `updated`, fractional `index`, radians for `angle`, binding objects, …). That is
noise for an author. The simple format asks only for **type, position, size, text, and style**;
the converter generates everything else exactly the way Excalidraw's importer expects.

## Quick start

```bash
# render the bundled example to PNG
uv run _tools/excalidraw.py to-image -i _docs/examples/sample.excalidraw.yaml -o /tmp/out.png

# author -> native, via a pipe
cat diagram.yaml | uv run _tools/excalidraw.py to-excalidraw > diagram.excalidraw

# native -> simple (lossy), to inspect/edit an existing file
uv run _tools/excalidraw.py from-excalidraw -i diagram.excalidraw -o diagram.yaml

uv run _tools/excalidraw.py validate -i diagram.yaml
uv run _tools/excalidraw.py schema            # machine-readable JSON Schema
```

The tool uses [PEP 723](https://peps.python.org/pep-0723/) inline dependencies, so `uv`
auto-installs PyYAML and (for PNG) cairosvg on first run.

## Document structure

```yaml
version: 1                                   # simple-format version (optional)
canvas:                                      # optional
  background: "#ffffff"                      # -> appState.viewBackgroundColor
  theme: light                               # light | dark
layout:                                      # optional auto-placement (see below)
  mode: row                                  # row | col | grid | none
  gap: 40
  start: [40, 40]
  cols: 3                                    # grid only
defaults:                                    # optional; merged into every element
  stroke: "#1e1e1e"
  strokeWidth: bold
  roughness: 1
  fontSize: 20
elements:                                    # required: the list of elements
  - <element>
```

Because YAML is a superset of JSON, JSON input is accepted by every command too.

## Elements

Each list item is a mapping with **exactly one type key** whose value is the element **id**
(a string) — or an inline props map. The remaining keys are properties.

```yaml
- rect: box1            # type key `rect`, id `box1`
  at: [100, 100]
  size: [220, 90]
  text: Login

- arrow: { from: box1, to: c1 }   # inline props; id auto-generated

- text: "A free-standing label"   # text element: value is the content, not an id
  id: note1                       #   (optional; only needed to reference it)
```

If the id is omitted it is auto-generated (`el-<n>`); give an explicit id whenever another
element needs to reference it (`from`/`to`/`frame`).

### Type keys

| Key | Native type | Notes |
|-----|-------------|-------|
| `rect` / `rectangle` | rectangle | |
| `ellipse` / `circle` | ellipse | |
| `diamond` | diamond | |
| `text` | text | free-standing text; the **type-key value is the content** (`- text: "Hello"`); add `id:` to reference it |
| `arrow` | arrow | connector; defaults to an end arrowhead |
| `line` | line | connector; no arrowhead by default |
| `frame` | frame | a named container region; `name:` sets its label |
| `draw` / `freedraw` | freedraw | requires explicit `points` |

`frame` doubles as a **parent reference**: on a non-frame element, `frame: <id>` places it
inside that frame (`rect: box, frame: screen1`).

### Common properties (all optional)

| Property | Maps to | Accepts / default |
|----------|---------|-------------------|
| `at: [x, y]` | x, y | numbers; or use `x:`/`y:` |
| `size: [w, h]` | width, height | numbers; or `w:`/`h:`/`width:`/`height:`; per-type defaults |
| `text` | bound/standalone text | string (centered inside shapes) |
| `stroke` | strokeColor | `#rrggbb`, `rrggbb`, name, `transparent` (default `#1e1e1e`) |
| `bg` | backgroundColor | as above (default `transparent`) |
| `textColor` | bound-text color | overrides `stroke` for a shape's label |
| `fill` | fillStyle | `solid` (default) · `hachure` · `cross-hatch` |
| `strokeWidth` | strokeWidth | `thin`/`bold`/`extra` → 1/2/4, or a number (default `bold`) |
| `strokeStyle` | strokeStyle | `solid` (default) · `dashed` · `dotted` |
| `roughness` | roughness | `0` (clean) · `1` (default) · `2` (sketchy) |
| `roundness` | roundness | `true` → rounded corners; `false` (default) → sharp |
| `opacity` | opacity | `0`–`100` (default `100`) |
| `angle` | angle | **degrees** (converted to radians) |
| `fontSize` | fontSize | number (default `20`) |
| `fontFamily` | fontFamily | `hand`/`normal`/`code` → 1/2/3 (default `normal`) |
| `align` | textAlign | `left` · `center` · `right` |
| `valign` | verticalAlign | `top` · `middle` · `bottom` |
| `group` | groupIds | a name or list of names; same name → grouped together |
| `frame` | frameId | parent frame id (non-frame elements) |
| `name` | name | frame label |
| `link` | link | URL |

### Connectors (`arrow`, `line`)

Choose one way to define the path:

- **Binding (preferred):** `from: <id>` and `to: <id>`. The converter computes edge-to-edge
  points, sets `startBinding`/`endBinding`, and registers the connector on both shapes'
  `boundElements`, so the arrow follows the shapes when moved in Excalidraw.
- **Explicit endpoints:** `start: [x, y]` and `end: [x, y]`.
- **Polyline:** `points: [[x, y], …]` (absolute coordinates).

Optional: `gap` (binding gap, default 4), `arrowhead`/`endArrowhead`/`startArrowhead`
(`arrow` · `triangle` · `dot` · `bar` · `none`).

## Auto-layout

Elements **without** explicit coordinates are placed automatically using the `layout:` block
(default `mode: row`). Explicit `at:`/`x:`/`y:` always wins; connectors are never auto-placed.

- `row` — left to right, advancing by element width + `gap`.
- `col` — top to bottom, advancing by height + `gap`.
- `grid` — left to right, wrapping after `cols` columns.
- `none` — unplaced elements default to `(0, 0)`.

`start: [x, y]` sets the origin. This lets an agent declare a sequence of boxes without
computing a single coordinate.

## Text containers

When a **shape** (rect/ellipse/diamond) has a `text:` property, the converter emits a
separate native text element bound to it (`containerId` + an entry in the shape's
`boundElements`), centered by default. A top-level `text:` element instead stands on its own.

## Commands

| Command | Purpose |
|---------|---------|
| *(none)* | print help / available commands |
| `schema [--format json\|yaml]` | print the JSON Schema of the simple format |
| `to-excalidraw [--input F] [--output F] [--compressed] [--seed N]` | simple → native JSON (`--compressed` gzips it) |
| `from-excalidraw [--input F] [--output F]` | native → simple YAML (lossy) |
| `to-image [--input F] [--output F] [--format svg\|png] [--scale N] [--padding N]` | render preview |
| `validate [--input F]` | structural + reference validation; non-zero exit on failure |

I/O rules shared by all commands: when `--input` is omitted the tool reads **stdin** if data is
piped; when `--output` is omitted it writes **stdout**. `--seed` makes id/nonce generation
deterministic (useful for tests and stable diffs).

## Rendering notes

`to-image` produces a **clean vector** rendering — it does *not* reproduce Excalidraw's
hand-drawn "rough" style. It is meant for quick previews. SVG output is plain stdlib string
building; PNG is rendered with Pillow (`--scale` controls resolution). Both work offline with
no system libraries. The native fill styles (`hachure`/`cross-hatch`) render as solid fills in
the preview.

## Round-tripping & fidelity

`to-excalidraw` is the authoritative direction. `from-excalidraw` is intentionally **lossy**:
it folds bound text back into shapes, folds arrow bindings into `from`/`to`, drops default
field values, and converts radians back to degrees — producing minimal, re-editable YAML.
Element **count and types are preserved** through a `from → to` round trip, but exact
coordinates of auto-bound arrows may be re-solved by Excalidraw on load.

## Minimal example

```yaml
canvas: { background: "#ffffff" }
layout: { mode: row, gap: 60, start: [60, 80] }
elements:
  - rect: a
    text: Start
    bg: "#a5d8ff"
  - rect: b
    text: Finish
    bg: "#b2f2bb"
  - arrow: { from: a, to: b }
```
