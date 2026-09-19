# Teamsim scenario format — `ubiq.teamsim/1`

One JSON file describes a whole graph: sessions, the tasks inside them, the agents on those tasks,
the delegates (subagents) inside an agent's fence, and extra links between agents. Both halves of
the spike read it — the Python iteration tool in `_tools/teamsim/` and the `teamsim` section of the
kitchen sink in `crates/ubiq/src/ui/sink/teamsim.rs`, which carries the same files as presets.

```json
{
  "format": "ubiq.teamsim/1",
  "name": "wide-coordination",
  "note": "one session, a lead over eight workers, three of them with delegates",
  "viewport": { "w": 1600, "h": 1000 },
  "sessions": [
    { "id": "s1", "name": "transport split", "branch": "feat/bus", "worktree": true }
  ],
  "tasks": [
    { "id": "t1", "session": "s1", "title": "Split the bus", "shape": "coordinated" }
  ],
  "agents": [
    {
      "id": "a1", "session": "s1", "task": "t1", "parent": null,
      "name": "lead", "role": "coordinator", "harness": "claude-code",
      "activity": "tools",
      "subagents": [ { "id": "d1", "name": "scan" }, { "id": "d2", "name": "patch" } ]
    }
  ],
  "links": [ { "from": "a1", "to": "a2", "kind": "handoff" } ]
}
```

## Fields

| Field | Meaning |
|---|---|
| `format` | Always `ubiq.teamsim/1`. A reader that does not know the string refuses the file. |
| `name` | Short slug. The preset's label and the rendered file's stem. |
| `note` | One line saying what the scenario stresses. Drawn as the caption. |
| `viewport` | The rectangle readability is judged against. Optional; defaults to 1600×1000. |
| `sessions[]` | `id`, `name`, `branch`, `worktree`. A session is the outermost fence. |
| `tasks[]` | `id`, `session`, `title`, `shape` (`direct` \| `chain` \| `coordinated`). A task is the inner fence. |
| `agents[]` | `id`, `session`, `task` (nullable — an agent nobody gave work to sits above the containers), `parent` (nullable agent id), `name`, `role`, `harness`, `activity`, `subagents[]`. |
| `agents[].subagents[]` | `id`, `name`. A delegate is not an agent record: it is a small card inside its parent's fence, and its count is what the arrangement leaves room for. |
| `links[]` | `from`, `to`, `kind` (`spawn` \| `handoff` \| `watch`). |

## What the layout reads

`agents[].parent` is the only edge the arrangement obeys — it is what gives a card its depth inside
its container, and what `tree` hangs containers off. `links` are drawn and counted (a crossing is a
readability cost) but never move a card; `spawn` links that duplicate a `parent` are ignored.

`subagents[]` contributes a count per card: how much room the packers leave under it for the fence.
Which shape those delegates take inside the fence is the algorithm's business, and it is the part
this spike is about.

## Presets

Every file in `_tools/teamsim/scenarios/` is a preset. The Rust sink embeds them with `include_str!`,
so adding a scenario there makes it selectable in the sink with no other change than the row in
`crates/ubiq/src/ui/sink/teamsim.rs`'s preset table.

## Hand-placed positions

A scenario may carry the positions a human dragged things to. They are what makes a saved scenario
reproduce a picture rather than only a graph, and they are the input the `adaptive` arrangement
starts from.

```json
{
  "positions": {
    "tasks":     { "t1": [120, 240] },
    "agents":    { "a1": [0, 0], "a7": [296, 184] },
    "subagents": { "a1/d2": [0, 156] }
  }
}
```

Every entry is optional and every coordinate is a point at 100% zoom, in the same frame the layout
uses: a task's is its container's origin on the canvas, an agent's is its offset inside its task
(or its absolute position when it has no task), and a delegate's is its offset inside the card that
spawned it, keyed `<agent id>/<subagent id>`. Anything the map does not name is placed by the
arrangement. An arrangement that runs from scratch ignores the whole block; `adaptive` reads it and
moves as little as it can.
