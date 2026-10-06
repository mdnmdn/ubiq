<!-- Adapted from Archify 3.0.1 schemas, examples and references/ (MIT, (c) tt-a1i). -->
# sequence (schema_version 1)

API call chains, request lifecycles, async traces, back-and-forth between people. Required:
`participants`, `messages`.

- `participants[]`: `id`, `type`, `label` required; optional `sublabel`, `icon`, `sources`. Order them
  by conversation role (who talks first, left to right).
- `messages[]`: `from`, `to`, `y` (number, >= 160), `label` required; optional `id`, `variant`
  (`default | emphasis | security | dashed | return`), `note`. Time flows down: `y` strictly
  increases through the story, about 25-30 apart. The canvas grows with the largest `y`.
- `segments[]` `{from,to,label}` (y range) are light phase guides.
- `activations[]` `{participant, from, to, type}` mark when a participant is busy.
- `meta.column_fit: "spread"` when the viewBox is wide or labels need width. No automatic port spread.
- `return` = quiet response; `dashed` = async; `security` = auth/policy; `emphasis` = main call.
- Use variants for meaning, not decoration. Keep message labels short (verb or `METHOD /path`).

Minimal example:

```json
{
  "schema_version": 1,
  "diagram_type": "sequence",
  "meta": { "title": "Cache miss", "output": "cache-miss.html", "quality_profile": "showcase" },
  "participants": [
    { "id": "web", "type": "frontend", "label": "Web" },
    { "id": "api", "type": "backend", "label": "API" },
    { "id": "cache", "type": "database", "label": "Redis" },
    { "id": "db", "type": "database", "label": "Postgres" }
  ],
  "messages": [
    { "id": "m1", "from": "web", "to": "api", "y": 180, "label": "GET /report", "variant": "emphasis" },
    { "id": "m2", "from": "api", "to": "cache", "y": 210, "label": "get" },
    { "id": "m3", "from": "cache", "to": "api", "y": 240, "label": "miss", "variant": "return" },
    { "id": "m4", "from": "api", "to": "db", "y": 270, "label": "SELECT" },
    { "id": "m5", "from": "api", "to": "web", "y": 300, "label": "200 JSON", "variant": "return" }
  ]
}
```
