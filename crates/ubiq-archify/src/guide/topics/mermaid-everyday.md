<!-- Adapted from Archify 3.0.1 SKILL.md and references/ (MIT, (c) tt-a1i). -->
# Mermaid input and everyday subjects

## Mermaid

Read Mermaid for topology and meaning, then author fresh JSON. Never reproduce Mermaid styling.

- `flowchart` / `graph` -> `workflow`, or `architecture` for a component map.
- `sequenceDiagram` -> `sequence`: participants become participants, arrows become messages with
  increasing `y`.
- `stateDiagram` -> `lifecycle`: states and transitions keep their meaning.

## Everyday subjects

A leave or travel plan, an application or approval process, a back-and-forth such as renting, where
money or documents go, where an order stands: same five types and the same semantic `type` values.

- Choose workflow (plan, process), sequence (back-and-forth), dataflow (where things go), lifecycle
  (where it stands), architecture (what something is made of).
- Use everyday `icon` values (`calendar clock person briefcase flag moon`; `none` hides the corner
  symbol) and `meta.legend.entries.<kind>.label` to rename kinds for the reader, for example
  `backend` shown as "Holiday". Icons are decorative; the node label must still carry the meaning.
- Ask for missing personal facts. Never invent dates, amounts, rules or deployment facts; put
  unknowns beside the claim as explicit unknowns.
