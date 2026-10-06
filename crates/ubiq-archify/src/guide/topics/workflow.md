<!-- Adapted from Archify 3.0.1 schemas, examples and references/ (MIT, (c) tt-a1i). -->
# workflow (schema_version 2 for new documents; 1 only for fixed legacy geometry)

Processes, approval gates, tool calls, runbooks, CI/CD. Required: `lanes`, `nodes`, `edges`.
Do not flip `schema_version` on a document that has absolute coordinates.

- `lanes[]`: `id`, `label`, optional `variant` (`normal | exception`). Lane = responsibility; use
  `exception` for human wait, denial, retry and failure lanes.
- `nodes[]`: `id`, `lane`, `col` (0..5), `type`, `label` required. Optional `sublabel`, `tag`, `icon`,
  `sources`, `width`, `height`, `yOffset` (relative to lane centre, e.g. -90/0/90).
- `edges[]`: `from`, `to` required. Optional `id`, `label`, `variant`, `role`
  (`main | branch | async | return | error`), `route` presets (`auto | straight | drop |
  outside-right | return-left | bottom-channel | up-channel`), `bias` (0..1), `channelX/Y`, label
  controls. Prefer a route preset before raw `via`.
- `phases[]` `{id,label,fromCol,toCol}` and `groups[]` `{id,label,lane,fromCol,toCol}` (cols 0..5).
- `mainPath [ids]` (>= 2): the happy path; consecutive ids must have an edge and move left to right.
- `semanticChecks {allowedRoots, allowedTerminals, requiredEdges, requiredPaths}` only for facts the
  evidence establishes; never weaken them to pass.
- A node label must fit the node width (default ~92 px, roughly 10 characters): shorten it, move
  detail to `sublabel`, or set `width`. Otherwise `layout/constraint` fires.
- Columns express progression; keep the happy path monotonic, retries and exception returns outside
  the main corridor. Stacked sequential stages: one lane + one group, omit `viewBox`.
- Viewport overflow with `workflowLanes`: redistribute steps across columns and lanes, never merge
  lanes or drop pins automatically.

Minimal example:

```json
{
  "schema_version": 2,
  "diagram_type": "workflow",
  "meta": { "title": "Deploy approval", "output": "deploy-approval.html", "quality_profile": "showcase" },
  "lanes": [
    { "id": "dev", "label": "Developer" },
    { "id": "ci", "label": "CI" },
    { "id": "ops", "label": "Release owner", "variant": "exception" }
  ],
  "nodes": [
    { "id": "push", "lane": "dev", "col": 0, "type": "frontend", "label": "Push branch" },
    { "id": "test", "lane": "ci", "col": 1, "type": "backend", "label": "Run tests" },
    { "id": "approve", "lane": "ops", "col": 2, "type": "security", "label": "Approve" },
    { "id": "ship", "lane": "ci", "col": 3, "type": "cloud", "label": "Deploy" }
  ],
  "edges": [
    { "id": "e1", "from": "push", "to": "test", "variant": "emphasis" },
    { "id": "e2", "from": "test", "to": "approve", "label": "green", "variant": "security" },
    { "id": "e3", "from": "approve", "to": "ship", "label": "approved", "variant": "emphasis" }
  ],
  "mainPath": ["push", "test", "approve", "ship"]
}
```
