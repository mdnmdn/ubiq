<!-- Adapted from Archify 3.0.1 schemas, examples and references/ (MIT, (c) tt-a1i). -->
# architecture (schema_version 1)

Components, services, boundaries, infrastructure. Required: `schema_version`, `diagram_type`,
`meta`, `components`.

- `components[]`: `id`, `type`, `label` required. Optional `sublabel`, `tag`, `icon`, `sources`,
  and placement `row`+`col` (grid) or `pos [x,y]` + `size [w,h]` (free, for a bounded exception).
- `layout`: `{mode:"grid", origin?, cols?(1..12), gapX?, gapY?, cellW?(>=40), cellH?(>=24)}`.
- `boundaries[]`: `kind` (`region` | `security-group`), `label`, `wraps [ids]` (>= 1), `pad?`.
- `connections[]`: `from`, `to` required. Optional `id`, `label`, `variant`, `route`
  (`auto | straight | orthogonal-h | orthogonal-v`), `fromSide/toSide` (`left right top bottom`),
  `via [[x,y]]`, `labelAt/labelDx/labelDy/labelSegment`, `width`.
- `cards[]`: `{dot: cyan|emerald|violet|amber|rose|orange|slate, title, items[]}` for supporting facts.
- Semantics: `emphasis` = main path, `security` = auth/consent/policy/PII, `dashed` = async/batch/event.
- `meta.engineering_profile: "deployment-ownership"` only for an explicit production-deployment or
  ownership review: needs a region and a security-group boundary, an owner `tag` on every
  non-external component, databases inside a security group, and a `label` on every connection
  crossing a boundary.

Minimal example:

```json
{
  "schema_version": 1,
  "diagram_type": "architecture",
  "meta": { "title": "Web request path", "output": "web-request.html", "quality_profile": "showcase" },
  "layout": { "mode": "grid" },
  "components": [
    { "id": "user", "type": "external", "label": "User", "sublabel": "browser", "row": 0, "col": 0 },
    { "id": "api", "type": "backend", "label": "API", "sublabel": "HTTP :8000", "row": 0, "col": 1 },
    { "id": "db", "type": "database", "label": "Postgres", "sublabel": "primary", "row": 0, "col": 2 },
    { "id": "cache", "type": "database", "label": "Redis", "sublabel": "read-through", "row": 1, "col": 1 }
  ],
  "connections": [
    { "id": "user-api", "from": "user", "to": "api", "label": "HTTPS", "variant": "emphasis" },
    { "id": "api-db", "from": "api", "to": "db", "label": "SQL" },
    { "id": "api-cache", "from": "api", "to": "cache", "label": "get/set", "variant": "dashed" }
  ]
}
```

Layout trouble: `archify_guide("architecture-layout-repair")`.
