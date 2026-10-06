<!-- Adapted from Archify 3.0.1 schemas, examples and references/ (MIT, (c) tt-a1i). -->
# lifecycle (schema_version 2 for new documents; 1 is legacy)

State and status transitions, retries, waiting and terminal states, where an order or application
stands. Required: `lanes`, `states`, `transitions`.

- `lanes[]`: 1..4 of `{id,label}`; `main` is required, `terminal` is reserved for exits. v2: each
  populated lane is one row, `main` first, `terminal` last, others in `lanes[]` order.
- `states[]`: `id`, `type`, `label`, `lane`, `col` (0..4, one x grid shared by every row) required.
  `type`: `start active waiting decision success failure neutral external`. Optional `sublabel`,
  `tag`, `step`, `icon`, `sources`. Place an interruption in the column of the state it leaves so
  its transition is a straight vertical.
- `transitions[]`: `from`, `to` required; optional `label` (keep it short: a gap with several
  parallel lines has little room), `note`, `route` (incl. `right-channel`, `left-channel`),
  `cornerRadius`. Every transition is authored, including the main path: there is no implied rail.
- A recoverable failure needs a real transition back to an active state; a card saying "retry" is
  not topology. Semantics: `success` completion, `failure` terminal, `waiting` pause, `decision` gate.
- v2: omit `meta.viewBox`; the canvas is sized from the layout.

Minimal example:

```json
{
  "schema_version": 2,
  "diagram_type": "lifecycle",
  "meta": { "title": "Order status", "output": "order-status.html", "quality_profile": "showcase" },
  "lanes": [ { "id": "main", "label": "Fulfilment" }, { "id": "terminal", "label": "Exits" } ],
  "states": [
    { "id": "placed", "type": "start", "label": "Placed", "lane": "main", "col": 0 },
    { "id": "paid", "type": "active", "label": "Paid", "lane": "main", "col": 1 },
    { "id": "shipped", "type": "waiting", "label": "Shipped", "lane": "main", "col": 2 },
    { "id": "delivered", "type": "success", "label": "Delivered", "lane": "terminal", "col": 2 },
    { "id": "cancelled", "type": "failure", "label": "Cancelled", "lane": "terminal", "col": 0 }
  ],
  "transitions": [
    { "from": "placed", "to": "paid", "label": "payment ok" },
    { "from": "paid", "to": "shipped", "label": "dispatch" },
    { "from": "shipped", "to": "delivered", "label": "signed" },
    { "from": "placed", "to": "cancelled", "label": "timeout" }
  ]
}
```
