"""The Rust tests at the bottom of `layout.rs`, ported — the evidence the port is faithful.

Run them with `python -m pytest _tools/teamsim/test_algos.py`, or with `just teamsim --selftest`,
which is the same functions through `run()`.

The last few tests are this tool's own: that the space the packers reserved is the space the picture
draws, and that `adaptive` keeps the contract it claims — nothing moves that the arriving block did
not push.
"""

from __future__ import annotations

import itertools
from pathlib import Path

from algos import (
    ALGOS,
    CARD_GAP_X,
    CARD_GAP_Y,
    CARD_HEIGHT,
    CARD_WIDTH,
    EPS,
    LAYOUT_MARGIN,
    LAYOUT_WIDTH,
    RING_PAD,
    SUB_WIDTH,
    TASK_GAP,
    Agent,
    Scenario,
    Session,
    Task,
    _build,
    _drawn,
    _finish,
    _union,
    content_rect,
    fence,
    layout_auto,
    load,
    pack_best,
    pack_shelf,
    pack_skyline,
    pack_tree,
    ring_drop,
    row_width,
    rings_of,
    sub_slot,
    task_forest,
)

HERE = Path(__file__).parent
_next = itertools.count(1)


def agent(session, task, parent, subs=0) -> Agent:
    ident = f"a{next(_next)}"
    return Agent(
        id=ident,
        session=session,
        task=task,
        parent=parent,
        subagents=[(f"{ident}s{ix}", f"sub{ix}") for ix in range(subs)],
    )


def scenario(agents, tasks) -> Scenario:
    sessions = {a.session for a in agents} | {t.session for t in tasks}
    return Scenario(
        name="test",
        sessions=[Session(id=s) for s in sorted(sessions)],
        tasks=list(tasks),
        agents=list(agents),
    )


def built(agents, tasks, algo):
    out = _build(scenario(agents, tasks), ALGOS[algo])
    _finish(out)
    return out


def at_of(out, ident):
    return next(c.at for c in out.cards if c.agent.id == ident)


def offset_of(out, ident):
    return next(c.offset for c in out.cards if c.agent.id == ident)


def spread():
    """A spread of boxes wide, tall and square, which is what a real session looks like."""
    return [
        (300.0, 200.0),
        (120.0, 480.0),
        (640.0, 160.0),
        (240.0, 240.0),
        (180.0, 90.0),
        (520.0, 320.0),
        (90.0, 700.0),
    ]


def overlaps(a, b, gap):
    apart_x = a[0][0] + a[1][0] + gap <= b[0][0] + EPS or b[0][0] + b[1][0] + gap <= a[0][0] + EPS
    apart_y = a[0][1] + a[1][1] + gap <= b[0][1] + EPS or b[0][1] + b[1][1] + gap <= a[0][1] + EPS
    return not (apart_x or apart_y)


def clear(at, sizes, gap):
    for i in range(len(sizes)):
        for j in range(i + 1, len(sizes)):
            assert not overlaps((at[i], sizes[i]), (at[j], sizes[j]), gap), (
                f"box {i} at {at[i]} and box {j} at {at[j]} are closer than {gap}"
            )


def test_a_shelf_keeps_its_boxes_apart_and_wraps():
    sizes = spread()
    at, extent = pack_shelf(sizes, 900.0, TASK_GAP)
    clear(at, sizes, TASK_GAP)
    assert extent[0] <= 900.0, "nothing spills past the shelf's width"
    assert any(p[1] > 0.0 for p in at), "and a second row was needed"


def test_a_skyline_keeps_its_boxes_apart():
    sizes = spread()
    at, extent = pack_skyline(sizes, 800.0, TASK_GAP)
    clear(at, sizes, TASK_GAP)
    for ix, (w, h) in enumerate(sizes):
        assert at[ix][0] >= -EPS and at[ix][1] >= -EPS
        assert at[ix][0] + w <= extent[0] + EPS and at[ix][1] + h <= extent[1] + EPS


def test_packing_beats_the_shelf_and_stays_squarish():
    sizes = spread()
    packed, tight = pack_best(sizes, TASK_GAP)
    clear(packed, sizes, TASK_GAP)

    _, shelf = pack_shelf(sizes, LAYOUT_WIDTH - LAYOUT_MARGIN, TASK_GAP)
    assert tight[0] * tight[1] <= shelf[0] * shelf[1] + EPS, (
        f"packed {tight} is no larger than the shelf's {shelf}"
    )
    aspect = max(tight[0] / tight[1], tight[1] / tight[0])
    assert aspect <= 2.5, f"and reads as a block, not a ribbon: {aspect}"


def test_a_tree_centres_a_parent_over_its_children():
    sizes = [(200.0, 100.0)] * 4
    parents = [None, 0, 0, 0]
    at, extent = pack_tree(sizes, parents, TASK_GAP)
    clear(at, sizes, TASK_GAP)

    middle = lambda ix: at[ix][0] + sizes[ix][0] / 2.0  # noqa: E731
    span = (middle(1) + middle(3)) / 2.0
    assert abs(middle(0) - span) < EPS, f"the parent's middle {middle(0)} is over {span}"
    assert abs(middle(0) - middle(2)) < EPS, "which is over the middle child"
    assert at[1][1] - at[0][1] >= sizes[0][1], "and a row below it"
    assert extent[1] >= sizes[0][1] + sizes[1][1]


def test_a_tree_survives_a_loop():
    sizes = [(200.0, 100.0)] * 3
    at, _ = pack_tree(sizes, [2, 0, 1], TASK_GAP)
    assert len(at) == 3
    clear(at, sizes, TASK_GAP)


def test_columns_put_every_card_in_one_column():
    lead = agent("s", "job", None)
    members = [lead, agent("s", "job", lead.id), agent("s", "job", lead.id)]
    contents = ALGOS["columns"].inside("job", members, rings_of(members))
    assert len(contents.cards) == 3
    assert all(at[0] == 0.0 for _, at in contents.cards)
    assert contents.width == CARD_WIDTH
    for a, b in zip(contents.cards, contents.cards[1:]):
        assert b[1][1] - a[1][1] == CARD_HEIGHT + CARD_GAP_Y


def test_packed_folds_a_wide_row():
    lead = agent("s", "job", None)
    members = [lead] + [agent("s", "job", lead.id) for _ in range(8)]
    rings = rings_of(members)

    wide = ALGOS["flow"].inside("job", members, rings)
    folded = ALGOS["packed"].inside("job", members, rings)
    assert wide.width == row_width(8)
    assert folded.width == row_width(3), "eight workers break three wide"
    assert folded.height > wide.height, "which costs rows"
    assert len(folded.cards) == 9


def test_flow_is_unchanged():
    """**Flow is the arrangement nothing may quietly change.**"""
    one = agent("s", "first", None)
    two = agent("s", "first", None)
    three = agent("s", "second", one.id)
    tasks = [Task(id="first", session="s"), Task(id="second", session="s")]
    out = built([one, two, three], tasks, "flow")

    assert out.origins["first"] == (46.0, 72.0)
    assert out.origins["second"] == (46.0 + 604.0 + 56.0, 72.0)

    assert offset_of(out, one.id) == (0.0, 0.0)
    assert offset_of(out, two.id) == (CARD_WIDTH + CARD_GAP_X, 0.0)
    assert offset_of(out, three.id) == (0.0, 0.0)
    assert at_of(out, three.id) == (706.0, 72.0)


def test_every_arrangement_clears_the_loose_block():
    boss = agent("s", None, None)
    one = agent("s", "first", boss.id)
    two = agent("s", "second", one.id)
    tasks = [Task(id="first", session="s"), Task(id="second", session="s")]

    for key in ("flow", "packed", "tree", "columns"):
        out = built([boss, one, two], tasks, key)
        at = at_of(out, boss.id)
        assert at == (LAYOUT_MARGIN, LAYOUT_MARGIN), key
        for card in (one, two):
            assert at_of(out, card.id)[1] >= at[1] + CARD_HEIGHT, f"{key} draws under the block"
        assert out.origins["first"] != out.origins["second"], f"{key} keeps its containers apart"


def test_an_empty_container_still_gets_a_frame():
    tasks = [Task(id="job", session="s"), Task(id="nobodys", session="s")]
    for key in ("flow", "packed", "tree", "columns"):
        out = built([agent("s", "job", None)], tasks, key)
        origin = out.origins["nobodys"]
        assert origin[0] >= LAYOUT_MARGIN and origin[1] >= LAYOUT_MARGIN, key


def test_a_tree_hangs_a_container_under_its_spawner():
    lead = agent("s", "up", None)
    worker = agent("s", "down", lead.id)
    tasks = [Task(id="up", session="s"), Task(id="down", session="s")]

    assert task_forest(tasks, [lead, worker]) == [None, 0]

    out = built([lead, worker], tasks, "tree")
    above, below = at_of(out, lead.id), at_of(out, worker.id)
    assert below[1] > above[1], "the child container is a row down"
    assert abs(above[0] - below[0]) < EPS, f"and centred under it: {above} over {below}"


def test_a_fence_grows_round_wherever_a_delegate_was_put():
    snug = fence((0.0, 0.0), [sub_slot(0), sub_slot(1)])
    assert snug is not None
    assert snug[0] == -RING_PAD, "the fence starts a pad left of the card"
    assert snug[2] <= CARD_WIDTH + RING_PAD * 2.0 + EPS, f"and the row is no wider: {snug}"

    moved = fence((0.0, 0.0), [sub_slot(0), (600.0, 40.0)])
    assert moved is not None and moved[2] > snug[2], f"the fence widened: {moved} from {snug}"
    assert moved[0] + moved[2] == 600.0 + SUB_WIDTH + RING_PAD

    assert fence((0.0, 0.0), []) is None, "a card with no delegates wears none"


def test_a_row_of_cards_clears_the_fence_above_it():
    lead = agent("s", "job", None, subs=3)
    worker = agent("s", "job", lead.id)
    members = [lead, worker]

    bare = ALGOS["flow"].inside("job", members, {})
    ringed = ALGOS["flow"].inside("job", members, rings_of(members))

    row_of = lambda c, ident: next(at[1] for card, at in c.cards if card == ident)  # noqa: E731
    assert row_of(bare, lead.id) == row_of(ringed, lead.id)
    assert row_of(ringed, worker.id) - row_of(bare, worker.id) >= ring_drop(3) - EPS, (
        "the row under a card with three delegates is pushed clear of its fence"
    )
    assert ringed.height > bare.height, "and the container grew with it"


# ── this tool's own checks ────────────────────────────────────────────────────


def scenarios():
    return [load(p) for p in sorted((HERE / "scenarios").glob("*.json"))]


def test_the_reserved_space_is_the_space_that_gets_drawn():
    """What the packers left room for is what the picture takes: the fence agrees with `ring_drop`."""
    for scen in scenarios():
        for key in ("flow", "packed", "tree", "columns"):
            out = layout_auto(scen, key)
            for box in out.tasks:
                if not box.rect:
                    continue
                assert abs(box.rect[2] - box.size[0]) < EPS and abs(box.rect[3] - box.size[1]) < EPS, (
                    f"{scen.name}/{key}: {box.task.id} reserved {box.size} and drew "
                    f"{(box.rect[2], box.rect[3])}"
                )


def test_containers_never_overlap():
    for scen in scenarios():
        for key in ALGOS:
            out = layout_auto(scen, key)
            for session in out.band:
                drawn = _drawn(out, session)
                for i in range(len(drawn)):
                    for j in range(i + 1, len(drawn)):
                        a, b = drawn[i].rect, drawn[j].rect
                        assert not overlaps(((a[0], a[1]), (a[2], a[3])), ((b[0], b[1]), (b[2], b[3])), 0.0), (
                            f"{scen.name}/{key}: {drawn[i].task.id} and {drawn[j].task.id} overlap"
                        )


def test_adaptive_only_adjusts():
    """The contract: an arriving block moves what it overlaps, and most of the graph stands still."""
    for scen in scenarios():
        out = layout_auto(scen, "adaptive")
        assert out.growth, scen.name
        assert len(out.cards) == len(scen.agents), scen.name
        assert out.growth["order_kept"], f"{scen.name}: adaptive reshuffled the reading order"
        pairs = out.growth["pairs"]
        if pairs:
            still = out.growth["still"] / pairs
            assert still > 0.8, f"{scen.name}: only {still:.0%} of blocks held their place"


def test_hand_placed_positions_are_honoured():
    for scen in scenarios():
        if not scen.positions.any():
            continue
        out = layout_auto(scen, "adaptive")
        for task, origin in scen.positions.tasks.items():
            assert out.origins[task] == origin, f"{scen.name}: {task} was moved off its position"
        for ident, offset in scen.positions.agents.items():
            card = next((c for c in out.cards if c.agent.id == ident), None)
            assert card and card.offset == offset, f"{scen.name}: {ident} lost its offset"
        assert any(scen.positions.tasks or scen.positions.agents), scen.name


def test_every_scenario_loads_and_renders_a_bbox():
    for scen in scenarios():
        for key in ALGOS:
            out = layout_auto(scen, key)
            assert out.bbox[2] > 0.0 and out.bbox[3] > 0.0, f"{scen.name}/{key} drew nothing"
            assert _union([content_rect(c) for c in out.cards])[2] > 0.0


def run() -> int:
    """Every test, in one pass. Answers the number that failed."""
    failed = 0
    for name, fn in sorted(globals().items()):
        if not name.startswith("test_") or not callable(fn):
            continue
        try:
            fn()
            print(f"  ok   {name[5:].replace('_', ' ')}")
        except AssertionError as bad:
            failed += 1
            print(f"  FAIL {name[5:].replace('_', ' ')}\n       {bad}")
    print(f"\n{failed} failed" if failed else "\nall green")
    return failed


if __name__ == "__main__":
    raise SystemExit(1 if run() else 0)
