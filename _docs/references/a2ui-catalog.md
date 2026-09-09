---
id: ref-a2ui-catalog
title: A2UI catalogs and components
kind: reference
status: current
summary: The vocabulary an A2UI agent draws from — the eighteen components of the basic catalog with their props and enums, the shared envelope properties, the fifteen catalog functions, the layout model and its deliberate refusal of padding and colour, templated lists, and how an application defines a catalog of its own.
read_when: you are sizing the widget set a renderer would have to implement, or choosing which components a Ubiq catalog would expose to an agent
updated: 2026-09-09
verified: 2026-09-09
depends_on: [ref-a2ui-protocol]
---

# A2UI catalogs and components

A2UI ships **no fixed widget set**. It defines a catalog mechanism — a JSON Schema document listing
the components and functions an agent may use — and then publishes one reference catalog, the
**basic catalog**, as the design-system-agnostic baseline. The wire that carries these components is
[the A2UI protocol](./a2ui-protocol.md).

Everything below is read from the v1.0 basic catalog at
[`catalogs/basic/catalog.json`](https://raw.githubusercontent.com/a2ui-project/a2ui/main/specification/v1_0/catalogs/basic/catalog.json),
whose `catalogId` is `https://a2ui.org/specification/v1_0/catalogs/basic/catalog.json` and whose
`protocolVersion` is `1.0`.

## Shared properties

Every component carries `component` — the type discriminator, a `const` in each definition — and the
envelope properties from `ComponentCommon`:

| Property | Meaning |
|---|---|
| `id` | **Required.** The unique identifier other components reference it by |
| `catalogId` | Overrides the surface-level default catalog for this one component |
| `accessibility` | `label`, `description`, `live` (`off` \| `polite` \| `assertive`) and `hidden` |
| `metadata.extensions` | Vendor extension keys; the `a2ui_` prefix is reserved |

Every component in the basic catalog also accepts **`weight`**, a number behaving like CSS
`flex-grow` — and settable *only* on a direct child of a `Row` or `Column`.

The six input components additionally compose **`Checkable`**, which in v1.0 contributes exactly one
property: **`checks`**, an array of `{condition, message}` rules where `condition` is a binding or
function call evaluating to a validation result and `message` is an optional fallback string. The
`disabled` and `message` props that some summaries attribute to Checkable are **not** in the v1.0
schema.

## The eighteen components

### Display

| Component | What it is | Props |
|---|---|---|
| `Text` | A run of text. Required: `text` | `text` (DynamicString), `variant`: `caption` \| `body` (default `body`) |
| `Image` | An image from a URL. Required: `url` | `url`, `description`, `fit`: `contain` \| `cover` \| `fill` \| `none` \| `scaleDown` (default `fill`), `variant`: `icon` \| `avatar` \| `smallFeature` \| `mediumFeature` \| `largeFeature` \| `header` (default `mediumFeature`) |
| `Icon` | A named icon. Required: `name` | `name`: one of 59 built-in names, or `{"svgPath": …}`, or a binding |
| `Video` | Video playback. Required: `url` | `url`, `posterUrl` |
| `AudioPlayer` | Audio playback. Required: `url` | `url`, `description` |

`Icon`'s 59 built-ins are a Material-flavoured set: `accountCircle` `add` `arrowBack` `arrowForward`
`attachFile` `calendarToday` `call` `camera` `check` `close` `delete` `download` `edit` `event`
`error` `fastForward` `favorite` `favoriteOff` `folder` `help` `home` `info` `locationOn` `lock`
`lockOpen` `mail` `menu` `moreVert` `moreHoriz` `notificationsOff` `notifications` `pause` `payment`
`person` `phone` `photo` `play` `print` `refresh` `rewind` `search` `send` `settings` `share`
`shoppingCart` `skipNext` `skipPrevious` `star` `starHalf` `starOff` `stop` `upload` `visibility`
`visibilityOff` `volumeDown` `volumeMute` `volumeOff` `volumeUp` `warning`.

`Text` renders a simple Markdown subset. The catalog's own guidance forbids string concatenation —
there is no `+` operator, so dynamic text goes through the `formatString` function.

### Layout

| Component | What it is | Props |
|---|---|---|
| `Row` | Arranges children horizontally. Required: `children` | `children` (ChildList), `justify` (default `start`), `align` (default `stretch`) |
| `Column` | Arranges children vertically. Required: `children` | `children`, `justify` (default `start`), `align` (default `stretch`) |
| `List` | A repeating run, static or templated. Required: `children` | `children`, `direction`: `vertical` \| `horizontal` (default `vertical`), `align` (default `stretch`) |
| `Card` | An elevated container holding **one** child. Required: `child` | `child` (a single component id) |
| `Tabs` | Switchable panes. Required: `tabs` | `tabs`: array of `{title, child}`, at least one |
| `Modal` | An overlay with a trigger. Required: `trigger`, `content` | `trigger`, `content` (both single ids) |
| `Divider` | A separator | `axis`: `horizontal` \| `vertical` (default `horizontal`) |

`justify` takes `start` `center` `end` `spaceBetween` `spaceAround` `spaceEvenly` `stretch`;
`align` takes `start` `center` `end` `stretch`. A grid is a `Row` of `Column`s, or the reverse —
the catalog says so explicitly.

`Card` holding a single child is the trap worth naming: several children mean wrapping them in a
`Column` first.

### Input

| Component | What it is | Props |
|---|---|---|
| `Button` | A clickable trigger. Required: `child`, `action` | `child` (usually a `Text` id; an `Icon` only for an icon-only button), `variant`: `default` \| `primary` \| `borderless`, `action` |
| `TextField` | Text entry. Required: `label` | `label`, `value`, `placeholder`, `variant`: `shortText` \| `longText` \| `number` \| `obscured` (default `shortText`) |
| `CheckBox` | A boolean toggle. Required: `label`, `value` | `label`, `value` (DynamicBoolean) |
| `ChoicePicker` | Select one or many from options. Required: `component` only | `label`, `options`: `{label, value}[]`, `value` (DynamicStringList), `variant`: `multipleSelection` \| `mutuallyExclusive` (default `mutuallyExclusive`), `displayStyle`: `checkbox` \| `chips` (default `checkbox`), `filterable` (default false) |
| `Slider` | A numeric range. Required: `value`, `max` | `label`, `min` (default 0), `max`, `value` (DynamicNumber), `steps` (integer ≥ 1, snaps to discrete divisions) |
| `DateTimeInput` | A date and/or time picker. Required: `value` | `value` (ISO 8601; initialise to `""`), `enableDate` (default false), `enableTime` (default false), `min`, `max`, `label` |

`DateTimeInput` defaults **both** `enableDate` and `enableTime` to false, so a usable picker must
turn at least one on.

## The layout model

Flexbox-inspired and deliberately thin. A container offers `justify`, `align` and per-child
`weight`, and **nothing else**: the basic catalog has no padding, no margin, no gap, no width or
height, no absolute positioning, and **no colour anywhere**. Visual difference is expressed through
discrete `variant` enums — a `primary` button, a `caption` text — which the renderer maps to its own
design language.

This is a v1.0 design decision, not an oversight: v1.0 removed the rigid theme properties and
hardcoded brand colours that v0.9 carried, to "defer visual styling entirely to the target
framework's native theme". A richer catalog may reintroduce a `style` escape hatch, but the standard
one does not have one.

## Templated lists

A container's `children` is a `ChildList`, which is one of two things: a **static array** of
component ids, or a **template object** naming a `componentId` and a data-model `path`. The renderer
iterates the array at that path and instantiates the template once per item, with the item as the
binding scope — which is what makes relative paths resolve.

```json
{"id": "employee_list", "component": "List",
 "children": {"path": "/employees", "componentId": "employee_card_template"}}
```

Inside the template, `{"path": "name"}` resolves to `/employees/N/name` while `{"path": "/company"}`
still reaches the root. The reserved system function **`@index`** returns the 0-based index of the
current item, with an optional `offset` argument for 1-based numbering; it is available only inside
a template scope.

For a validator to check that a parent's references resolve, a catalog **must** type single-child
props as `ComponentId` and child lists as `ChildList`. A raw `"type": "string"` is read as static
text, and the link goes unchecked.

## The function library

Fifteen functions, each usable anywhere a `Dynamic*` value is accepted, written as
`{"call": "name", "args": {...}}`.

| Group | Functions |
|---|---|
| Validation → validationResult | `required(value)`, `regex(value, pattern)`, `length(value, min, max)`, `numeric(value, min, max)`, `email(value)` |
| Formatting → string | `formatString(value)`, `formatNumber(value, decimals, grouping)`, `formatCurrency(value, currency, decimals, grouping)`, `formatDate(value, format)`, `pluralize(value, zero, one, two, few, many, other)` |
| Logic → boolean | `and(values)`, `or(values)`, `not(value)` |
| Navigation → void | `openUrl(url)` — requires user activation |

`formatString` is the interpolation primitive, taking `${...}` placeholders over paths and nested
calls: `formatString("Hello ${/user/name}")`. `pluralize` selects by CLDR plural category.

Validation runs renderer-side: a failing `check` disables the action rather than firing it, and the
catalog's instructions say to put the error text in the check's `message` instead of building a
separate `Text` component to display it.

## Defining your own catalog

Catalogs are the extension point, and for most applications the expected one — the specification
says most production applications will define a catalog reflecting their own design system. A
catalog is a JSON Schema document with `catalogId`, optional Markdown `instructions`, a `components`
map and a `functions` map, with `anyComponent` and `anyFunction` under `$defs` for the envelope to
reference. Component, function and property names must obey Unicode UAX #31
(`^[\p{XID_Start}_][\p{XID_Continue}]*$`), and `Surface` is reserved.

Composition constraints ride on the component definitions: `allowedParents` and `allowedChildren`,
each an array of type names, each omittable to mean "anything". They are how a catalog says a
`MenuItem` appears only inside a `Menu`, or that a component may only be the surface's root.

**Unknown components have no crisply specified handling.** A renderer validating the payload can
reject it with a `VALIDATION_FAILED` error naming the offending JSON Pointer, and the tree model
requires skipping invalid *references* rather than failing; graceful degradation to a
placeholder is stated as intent rather than as a normative rule.

## Richer catalogs, and what the basic one lacks

The basic catalog has **no table, chart, canvas, iframe, progress indicator or code block**. Its only
rich media are `Video` and `AudioPlayer`. Anything beyond the eighteen is a catalog extension.

Google's Angular-targeted **Material catalog** is the worked example of a bigger one, adding a
`MaterialTable` with `columns` and `rows`, progress bars and spinners, expansion panels, grid lists,
badges, menus, chips, radio buttons, selects, slide toggles and separate date and time pickers,
with a per-component `style` object the basic catalog does not have. It is documented in
[Google Cloud's component gallery reference](https://docs.cloud.google.com/gemini/enterprise/docs/a2ui-agents/a2ui-component-gallery-reference)
rather than in the specification directory; treat its exact property names as **unconfirmed against
source**, unlike the basic catalog above, which is read from the schema. The repository's guide to
[authoring custom components](https://a2ui.org/guides/authoring-components/) works through a
canvas-backed `Chart` component as its example.

## Related docs

- [A2UI protocol — the wire](./a2ui-protocol.md) — envelopes, binding and catalog negotiation.
- [Proposal — A2UI surfaces in Ubiq](../inbox/a2ui-proposal.md) — the cost of drawing these in GPUI.
