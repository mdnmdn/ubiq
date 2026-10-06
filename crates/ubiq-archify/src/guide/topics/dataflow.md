<!-- Adapted from Archify 3.0.1 schemas, examples and references/ (MIT, (c) tt-a1i). -->
# dataflow (schema_version 1)

Pipelines, ETL/ELT, lineage, governance, where money or documents go. Required: `stages`, `nodes`,
`flows`.

- `stages[]`: `{label}` in order, typically source, ingest, process, store, consume. A stage is a
  transformation or custody step.
- `nodes[]`: `id`, `type`, `label`, `stage` (index into `stages`), `row` (integer; separates parallel
  streams) required. Optional `sublabel`, `tag`, `icon`, `sources`, `width`, `height`, `yOffset`.
- `flows[]`: `from`, `to`, `label` ALL required. Optional `id`, `classification` (sensitivity, e.g.
  `PII`), `variant`, `route` (`auto | straight | vertical-channel | bottom-channel | top-channel`),
  `fromSide/toSide`, `via`, `labelAt`.
- The flow label names the data asset (`clickstream`, `invoice PDF`), not the transport. Use
  `security` for PII or policy paths and `dashed` for batch or async hops.
- Flows go forward through stages; a flow within a stage needs different rows.

Minimal example:

```json
{
  "schema_version": 1,
  "diagram_type": "dataflow",
  "meta": { "title": "Order analytics", "output": "order-analytics.html", "quality_profile": "showcase" },
  "stages": [ { "label": "Source" }, { "label": "Ingest" }, { "label": "Store" }, { "label": "Consume" } ],
  "nodes": [
    { "id": "shop", "type": "frontend", "label": "Shop", "stage": 0, "row": 0 },
    { "id": "stream", "type": "messagebus", "label": "Event stream", "stage": 1, "row": 0 },
    { "id": "wh", "type": "database", "label": "Warehouse", "stage": 2, "row": 0 },
    { "id": "bi", "type": "backend", "label": "Dashboards", "stage": 3, "row": 0 }
  ],
  "flows": [
    { "id": "f1", "from": "shop", "to": "stream", "label": "order events", "variant": "emphasis" },
    { "id": "f2", "from": "stream", "to": "wh", "label": "curated orders", "classification": "PII masked" },
    { "id": "f3", "from": "wh", "to": "bi", "label": "daily revenue", "variant": "dashed" }
  ]
}
```
