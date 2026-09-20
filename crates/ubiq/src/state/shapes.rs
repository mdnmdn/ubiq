//! The arrangements that grow a graph rather than stack it.
//!
//! [`layout.rs`](super::layout) holds the four arrangements that predate the layout spike and the
//! packers they share. These are the five on top of them: five ways to arrange one container's
//! cards, and one way to pack containers that never assumes there is a root.
//!
//! Every one of them grows **downwards**. A connector leaves the bottom of a card and arrives at
//! the top of the next one, so an arrangement that seats a child above its parent draws a line
//! backwards through the canvas whatever else it wins. The radial shapes here are all half shapes:
//! the downward sector, never the full turn.
//!
//! | Arrangement | One container's cards | A card's delegates |
//! |---|---|---|
//! | Organic | children fanned under their parent, bowed and jittered, then pushed apart | a jittered fan |
//! | Multiradial | every parent is its own hub: children on a downward semicircle round it | a fan arc |
//! | Spider | one hub per container, cards on concentric downward arcs, spokes to the parent | a fan arc |
//! | Hex | cards on a honeycomb lattice, a row per hand-off depth, rows half a cell apart | a staggered grid |
//! | Islands | disjoint families packed as separate islands, no root assumed | a grid |
//!
//! Islands is the one that answers "what if there is no single parent node": it groups by what is
//! *connected*, at both levels — the cards inside a container, and the containers inside a session
//! — and packs each group as its own island against the screen's rectangle.

use std::collections::HashMap;

use ubiq_proto::work::{AgentId, WorkAgent};

use super::layout::{
    Boxes, CARD_GAP_X, CARD_GAP_Y, CARD_HEIGHT, CARD_WIDTH, Contents, EPS, Packing, RingFn, Rings,
    SUB_DROP, SUB_GAP, SUB_HEIGHT, SUB_WIDTH, TASK_GAP, apart_enough, break_to, pack_best,
    radial_slot, remap, rows_by_depth, without_empties,
};

// ── the constants these shapes add ────────────────────────────────────────────

/// Half the downward sector a fan, a hub's brood and a spider's arc spread over, in degrees either
/// side of straight down. Past 90 a slot climbs back above the card it hangs off, which is the one
/// thing none of these shapes may do.
pub const FAN_SPREAD: f32 = 74.0;
/// The most delegates one fan arc seats before a second arc opens outside it.
pub const FAN_SEATS: usize = 7;
/// The most arcs a fan ring opens before it gives up and crowds the last one.
pub const FAN_ARCS: usize = 4;

/// How far an organic slot is nudged off its regular seat, as a fraction of the step to the next.
/// Enough to read as grown rather than drawn; not enough to make two of them touch.
pub const ORGANIC_JITTER: f32 = 0.3;
/// How much lower the ends of an organic brood hang than its middle, as a fraction of
/// [`CARD_GAP_Y`]. This is the bow that makes a row of children read as a spray rather than a row.
pub const ORGANIC_BOW: f32 = 1.4;

/// A honeycomb's long row. Rows alternate `HEX_COLS` and `HEX_COLS - 1`, offset half a cell. Two,
/// because a delegate is a full [`SUB_WIDTH`] wide: three of them across is 816pt of ring under a
/// 264pt card, which costs more sideways scrolling than the stagger is worth.
pub const HEX_COLS: usize = 2;

/// Half the sector a spider's web spans, in degrees either side of straight down.
pub const SPIDER_SPREAD: f32 = 80.0;

/// The room between two islands. Twice what sits between two containers, because the gap is the
/// only thing saying they are not one arrangement.
pub const ISLAND_GAP: f32 = TASK_GAP * 2.0;

/// How many passes the overlap relaxation makes before it settles for what it has.
pub const SPREAD_ROUNDS: usize = 80;

// ── the odds and ends every shape here uses ───────────────────────────────────

/// Where each card has been put, in the order the arrangement placed it.
///
/// A list rather than a map, because the order is the arrangement's own — a brood is placed
/// breadth-first from its root — and a hashed order would make the same graph come out differently
/// from one build to the next.
type Spots = Vec<(AgentId, (f32, f32))>;

/// The `brood_of` half of [`grown`]: where one parent puts its children.
type BroodFn = fn((f32, f32), (f32, f32), &[AgentId], &Boxes, &mut Spots, f32);

/// A repeatable number in `[-1, 1)` for a name. The jitter has to survive a re-render.
///
/// FNV-1a rather than a cryptographic digest: nothing here needs the hash to be hard to invert,
/// only to be the same number every time the same key is asked for within a build. It is not a
/// wire format and no stored position depends on it, so the constants may change freely.
pub fn seeded(key: &str, salt: u64) -> f32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key
        .bytes()
        .chain(std::iter::once(b'/'))
        .chain(salt.to_le_bytes())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash >> 32) as f32 / (1u32 << 31) as f32 - 1.0
}

/// Where a card has been put, or the origin for one nothing has placed.
fn spot_of(spots: &Spots, agent: AgentId) -> (f32, f32) {
    spots
        .iter()
        .find(|(id, _)| *id == agent)
        .map(|(_, at)| *at)
        .unwrap_or((0.0, 0.0))
}

/// Place a card, keeping the order it was first placed in.
fn put(spots: &mut Spots, agent: AgentId, at: (f32, f32)) {
    match spots.iter_mut().find(|(id, _)| *id == agent) {
        Some(slot) => slot.1 = at,
        None => spots.push((agent, at)),
    }
}

/// How far two boxes overlap once `gap` is owed between them. Both positive means they clash.
fn clash(
    at: (f32, f32),
    held: (f32, f32),
    other: (f32, f32),
    other_box: (f32, f32),
    gap: f32,
) -> (f32, f32) {
    let dx = (at.0 + held.0 + gap - other.0).min(other.0 + other_box.0 + gap - at.0);
    let dy = (at.1 + held.1 + gap - other.1).min(other.1 + other_box.1 + gap - at.1);
    (dx, dy)
}

/// Push clashing boxes apart sideways until nothing touches, in place.
///
/// Sideways only, and by half the overlap each: a shape here earns its look from where it put a
/// card vertically — under its parent, on its arc, in its row — and buying room by moving a card
/// down the page would spend the one axis the whole arrangement is about.
fn spread(spots: &mut Spots, held: &Boxes, gap: f32) {
    for _ in 0..SPREAD_ROUNDS {
        let mut moved = false;
        for i in 0..spots.len() {
            for j in (i + 1)..spots.len() {
                let (a, at_a) = spots[i];
                let (b, at_b) = spots[j];
                let (dx, dy) = clash(at_a, held.box_of(a), at_b, held.box_of(b), gap);
                if dx <= EPS || dy <= EPS {
                    continue;
                }
                let step = dx / 2.0 + EPS;
                let (left, right) = if at_a.0 <= at_b.0 { (i, j) } else { (j, i) };
                spots[left].1.0 -= step;
                spots[right].1.0 += step;
                moved = true;
            }
        }
        if !moved {
            return;
        }
    }
}

/// Card boxes worked out anywhere at all, normalised into a container's own frame.
fn contents_of(spots: &Spots, held: &Boxes) -> Contents {
    if spots.is_empty() {
        return Contents::empty();
    }
    let x0 = spots
        .iter()
        .map(|(_, at)| at.0)
        .fold(f32::INFINITY, f32::min);
    let y0 = spots
        .iter()
        .map(|(_, at)| at.1)
        .fold(f32::INFINITY, f32::min);
    let x1 = spots
        .iter()
        .map(|(a, at)| at.0 + held.box_of(*a).0)
        .fold(f32::NEG_INFINITY, f32::max);
    let y1 = spots
        .iter()
        .map(|(a, at)| at.1 + held.box_of(*a).1)
        .fold(f32::NEG_INFINITY, f32::max);

    let cards = spots
        .iter()
        .map(|(a, at)| {
            let lead = held.lead_of(*a);
            (*a, (at.0 - x0 + lead.0, at.1 - y0 + lead.1))
        })
        .collect();
    Contents {
        cards,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// The roots of one container's cards and each card's children, cycles broken.
///
/// Only a parent that is *in this container* is an edge — a card whose spawner sits in another
/// container is a root here, which is what makes the islands real rather than one graph.
fn family(members: &[&WorkAgent]) -> (Vec<AgentId>, HashMap<AgentId, Vec<AgentId>>) {
    let ids: Vec<AgentId> = members.iter().map(|m| m.id).collect();
    let mut parent: HashMap<AgentId, AgentId> = HashMap::new();
    for member in members {
        if let Some(up) = member.parent
            && up != member.id
            && ids.contains(&up)
        {
            parent.insert(member.id, up);
        }
    }

    let mut kids: HashMap<AgentId, Vec<AgentId>> = ids.iter().map(|id| (*id, Vec::new())).collect();
    let mut roots: Vec<AgentId> = Vec::new();
    for ident in &ids {
        let mut seen = vec![*ident];
        let mut up = parent.get(ident).copied();
        while let Some(at) = up {
            if seen.contains(&at) {
                break;
            }
            seen.push(at);
            up = parent.get(&at).copied();
        }
        // A parent chain that loops has no root, so the edge that closes it is the one dropped.
        if up.is_some() {
            parent.remove(ident);
        }
        match parent.get(ident).copied() {
            Some(above) => kids.entry(above).or_default().push(*ident),
            None => roots.push(*ident),
        }
    }
    (roots, kids)
}

/// The children of one card, or nothing.
fn kids_of(kids: &HashMap<AgentId, Vec<AgentId>>, node: AgentId) -> &[AgentId] {
    kids.get(&node).map(|b| b.as_slice()).unwrap_or(&[])
}

/// How many leaves hang off a node — the weight a wedge of the sector is shared out by.
fn leaves_of(root: AgentId, kids: &HashMap<AgentId, Vec<AgentId>>) -> usize {
    let brood = kids_of(kids, root);
    if brood.is_empty() {
        1
    } else {
        brood.iter().map(|k| leaves_of(*k, kids)).sum()
    }
}

/// Several independent groups of boxes, packed against the screen and merged into one frame.
fn as_islands(blocks: &[Spots], held: &Boxes, target: f32, gap: f32) -> Spots {
    let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(blocks.len());
    let mut framed: Vec<Spots> = Vec::with_capacity(blocks.len());
    for spots in blocks {
        if spots.is_empty() {
            framed.push(Vec::new());
            sizes.push((0.0, 0.0));
            continue;
        }
        let x0 = spots
            .iter()
            .map(|(_, at)| at.0)
            .fold(f32::INFINITY, f32::min);
        let y0 = spots
            .iter()
            .map(|(_, at)| at.1)
            .fold(f32::INFINITY, f32::min);
        framed.push(
            spots
                .iter()
                .map(|(a, at)| (*a, (at.0 - x0, at.1 - y0)))
                .collect(),
        );
        sizes.push((
            spots
                .iter()
                .map(|(a, at)| at.0 + held.box_of(*a).0)
                .fold(f32::NEG_INFINITY, f32::max)
                - x0,
            spots
                .iter()
                .map(|(a, at)| at.1 + held.box_of(*a).1)
                .fold(f32::NEG_INFINITY, f32::max)
                - y0,
        ));
    }

    let (at, _) = without_empties(&sizes, |kept| pack_best(kept, gap, target, true));
    let mut out: Spots = Vec::new();
    for (offset, spots) in at.iter().zip(&framed) {
        for (ident, spot) in spots {
            out.push((*ident, (offset.0 + spot.0, offset.1 + spot.1)));
        }
    }
    out
}

/// One downward arc of delegates, hugging the card's rectangle the way a radial ring does.
fn fan_arc(seats: usize, depth: usize) -> Vec<(f32, f32)> {
    if seats == 0 {
        return Vec::new();
    }
    if seats == 1 {
        return vec![radial_slot(180.0, depth)];
    }
    let (lo, hi) = (180.0 - FAN_SPREAD, 180.0 + FAN_SPREAD);
    let step = (hi - lo) / (seats - 1) as f32;
    (0..seats)
        .map(|ix| radial_slot(lo + ix as f32 * step, depth))
        .collect()
}

/// How many delegates arc `depth` seats while they stay a readable [`SUB_GAP`] apart.
fn fan_room(depth: usize) -> usize {
    let mut room = 1;
    for seats in 2..=FAN_SEATS {
        if !apart_enough(&fan_arc(seats, depth)) {
            break;
        }
        room = seats;
    }
    room
}

/// How the delegates share out between the arcs: the inner one first, then one outside it.
fn fan_arcs(count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut left = count;
    while left > 0 && out.len() < FAN_ARCS {
        let seats = left.min(fan_room(out.len()));
        out.push(seats);
        left -= seats;
    }
    if left > 0 && !out.is_empty() {
        // Past four arcs it is a crowd whatever we do; do not grow the box for it.
        *out.last_mut().expect("checked non-empty") += left;
    }
    out
}

/// Delegates on a downward arc under the card, the outer arcs outside the inner ones.
///
/// The radial ring puts them on the card's two flanks and leaves the top and the bottom clear.
/// This one does the opposite: it takes the bottom, because under Multiradial and Spider a card's
/// children are already out to the sides and the room below it is the room nobody else wants.
pub fn ring_fan(count: usize) -> Vec<(f32, f32)> {
    fan_arcs(count)
        .into_iter()
        .enumerate()
        .flat_map(|(depth, seats)| fan_arc(seats, depth))
        .collect()
}

/// The fan's own seats, nudged by `scale`.
///
/// At `scale = 0` it *is* [`ring_fan`], which is the whole point: [`fan_room`] proved those seats
/// are far enough apart, so the backing-off in [`ring_organic`] terminates.
fn organic_slots(count: usize, scale: f32) -> Vec<(f32, f32)> {
    let mut slots: Vec<(f32, f32)> = Vec::new();
    for (depth, seats) in fan_arcs(count).into_iter().enumerate() {
        let (lo, hi) = (180.0 - FAN_SPREAD, 180.0 + FAN_SPREAD);
        let step = (hi - lo) / (seats.saturating_sub(1)).max(1) as f32;
        for ix in 0..seats {
            let base = if seats > 1 {
                lo + ix as f32 * step
            } else {
                180.0
            };
            let turn =
                base + seeded(&format!("turn/{depth}"), ix as u64) * step * ORGANIC_JITTER * scale;
            let at = radial_slot(turn, depth);
            slots.push((
                at.0,
                at.1 + seeded(&format!("drop/{depth}"), ix as u64) * SUB_GAP * scale,
            ));
        }
    }
    slots
}

/// The fan, off its regular seats — a spray rather than an arc.
///
/// The nudge is seeded off the slot, so a re-render draws the same picture: an arrangement that
/// moves every time it is looked at cannot be judged against the one before it. And it is backed
/// off until nothing touches, because a crowded arc is what the jitter is *for* — two delegates
/// drawn one over the other is not organic, it is broken, and an eye reads it as one card with a
/// shadow.
pub fn ring_organic(count: usize) -> Vec<(f32, f32)> {
    let mut slots = Vec::new();
    for scale in [1.0f32, 0.7, 0.45, 0.25, 0.0] {
        slots = organic_slots(count, scale);
        if apart_enough(&slots) {
            return slots;
        }
    }
    slots
}

/// A honeycomb's rows: [`HEX_COLS`], then one fewer, then [`HEX_COLS`] again.
fn hex_rows(count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let mut left = count;
    let mut wide = true;
    while left > 0 {
        let seats = left.min(if wide { HEX_COLS } else { HEX_COLS - 1 });
        out.push(seats);
        left -= seats;
        wide = !wide;
    }
    out
}

/// Delegates on a honeycomb under the card: rows of three and two, offset half a cell.
///
/// The cells are laid out on the hexagonal lattice, but a delegate is a rectangle, so consecutive
/// rows are a full box apart rather than interlocking — a honeycomb of rectangles is a staggered
/// grid, and pretending otherwise would draw one delegate over another. What the stagger buys is a
/// ring that reads as a cluster rather than as a table, three wide and short.
pub fn ring_hex(count: usize) -> Vec<(f32, f32)> {
    if count == 0 {
        return Vec::new();
    }
    let cell = SUB_WIDTH + SUB_GAP;
    let span = HEX_COLS as f32 * SUB_WIDTH + (HEX_COLS - 1) as f32 * SUB_GAP;
    let left = (CARD_WIDTH - span) / 2.0;
    let top = CARD_HEIGHT + SUB_DROP;

    let mut out: Vec<(f32, f32)> = Vec::new();
    for (row, seats) in hex_rows(count).into_iter().enumerate() {
        let wide = seats as f32 * SUB_WIDTH + (seats - 1) as f32 * SUB_GAP;
        let x = left + (span - wide) / 2.0;
        for col in 0..seats {
            out.push((
                x + col as f32 * cell,
                top + row as f32 * (SUB_HEIGHT + SUB_GAP),
            ));
        }
    }
    out
}

// ── the insides: what one container's cards do ────────────────────────────────

/// A brood too wide for the screen, folded into several bows instead of one long spray.
///
/// Nine children on one fan is a 3400pt row: the shape is lovely and the canvas scrolls sideways
/// for two screens to show it. [`break_to`] is the same judgement the grid ring makes, at the
/// cards.
fn bows(brood: &[AgentId], held: &Boxes, target: f32) -> Vec<Vec<AgentId>> {
    let widest = brood
        .iter()
        .map(|c| held.box_of(*c).0)
        .fold(0.0f32, f32::max);
    let tallest = brood
        .iter()
        .map(|c| held.box_of(*c).1)
        .fold(0.0f32, f32::max);
    let per = break_to(brood.len(), (widest, tallest), target);
    brood.chunks(per.max(1)).map(|row| row.to_vec()).collect()
}

/// One parent's children, fanned under it, bowed at the ends and nudged off their seats.
fn brood_organic(
    at: (f32, f32),
    held_box: (f32, f32),
    brood: &[AgentId],
    held: &Boxes,
    spots: &mut Spots,
    target: f32,
) {
    if brood.is_empty() {
        return;
    }
    let widest = brood
        .iter()
        .map(|c| held.box_of(*c).0)
        .fold(0.0f32, f32::max);
    let step = widest + CARD_GAP_X;
    let mut y = at.1 + held_box.1 + CARD_GAP_Y;
    for bow_row in bows(brood, held, target) {
        let middle = (bow_row.len() - 1) as f32 / 2.0;
        for (ix, child) in bow_row.iter().enumerate() {
            let off = (ix as f32 - middle) * step;
            let bow = if middle > 0.0 {
                (ix as f32 - middle).abs() / middle
            } else {
                0.0
            } * CARD_GAP_Y
                * ORGANIC_BOW;
            let name = child.to_string();
            put(
                spots,
                *child,
                (
                    at.0 + held_box.0 / 2.0 - held.box_of(*child).0 / 2.0
                        + off
                        + seeded(&name, 1) * step * ORGANIC_JITTER,
                    y + bow + seeded(&name, 2).abs() * CARD_GAP_Y * ORGANIC_JITTER,
                ),
            );
        }
        y += bow_row
            .iter()
            .map(|c| held.box_of(*c).1)
            .fold(0.0f32, f32::max)
            + CARD_GAP_Y
            + CARD_GAP_Y * ORGANIC_BOW;
    }
}

/// One parent's children on downward arcs round it — the hub of a multiradial.
///
/// A hub with more children than one arc seats opens a second arc outside the first rather than
/// pushing the first one out to a radius that seats them all: the radius that seats nine children
/// is most of a screen, and nothing is inside it.
fn brood_radial(
    at: (f32, f32),
    held_box: (f32, f32),
    brood: &[AgentId],
    held: &Boxes,
    spots: &mut Spots,
    _target: f32,
) {
    if brood.is_empty() {
        return;
    }
    let centre = (at.0 + held_box.0 / 2.0, at.1 + held_box.1 / 2.0);
    let sector = (2.0 * FAN_SPREAD).to_radians();
    let lo = 180.0 - FAN_SPREAD;

    let mut reach = held_box.0.hypot(held_box.1) / 2.0
        + CARD_GAP_Y
        + brood
            .iter()
            .map(|c| held.box_of(*c).1)
            .fold(0.0f32, f32::max)
            / 2.0;
    let mut left = brood;
    while !left.is_empty() {
        // An arc is filled by what the children actually measure, not by the widest of them.
        let room = sector * reach;
        let (mut run, mut seats) = (0.0f32, 0usize);
        for child in left {
            run += held.box_of(*child).0 + CARD_GAP_X;
            if seats > 0 && run > room {
                break;
            }
            seats += 1;
        }
        let step = (2.0 * FAN_SPREAD) / seats as f32;
        for (ix, child) in left[..seats].iter().enumerate() {
            let rad = (lo + (ix as f32 + 0.5) * step).to_radians();
            let child_box = held.box_of(*child);
            put(
                spots,
                *child,
                (
                    centre.0 + reach * rad.sin() - child_box.0 / 2.0,
                    centre.1 - reach * rad.cos() - child_box.1 / 2.0,
                ),
            );
        }
        reach += left[..seats]
            .iter()
            .map(|c| held.box_of(*c).1)
            .fold(0.0f32, f32::max)
            + CARD_GAP_Y;
        left = &left[seats..];
    }
}

/// Every root grown into its own frame by `brood_of`, the frames packed as islands.
///
/// The two shapes that grow a tree outward — Organic and Multiradial — differ only in where a
/// parent puts its children, so that is the one thing they hand in.
fn grown(members: &[&WorkAgent], held: &Boxes, brood_of: BroodFn, target: f32) -> Spots {
    let (roots, kids) = family(members);
    let mut blocks: Vec<Spots> = Vec::with_capacity(roots.len());
    for root in roots {
        let mut spots: Spots = vec![(root, (0.0, 0.0))];
        let mut rank = vec![root];
        while !rank.is_empty() {
            let node = rank.remove(0);
            let brood = kids_of(&kids, node).to_vec();
            brood_of(
                spot_of(&spots, node),
                held.box_of(node),
                &brood,
                held,
                &mut spots,
                target,
            );
            rank.extend(brood);
        }
        spread(&mut spots, held, CARD_GAP_X);
        blocks.push(spots);
    }
    as_islands(&blocks, held, target, CARD_GAP_X * 2.0)
}

/// Children sprayed under their parent, bowed at the ends, then pushed apart until they fit.
pub fn inside_organic(
    members: &[&WorkAgent],
    rings: &Rings,
    ring: RingFn,
    target: f32,
) -> Contents {
    if members.is_empty() {
        return Contents::empty();
    }
    let held = Boxes::of(members, rings, ring);
    contents_of(&grown(members, &held, brood_organic, target), &held)
}

/// Every parent its own hub, its children on the downward half of a circle round it.
pub fn inside_multiradial(
    members: &[&WorkAgent],
    rings: &Rings,
    ring: RingFn,
    target: f32,
) -> Contents {
    if members.is_empty() {
        return Contents::empty();
    }
    let held = Boxes::of(members, rings, ring);
    contents_of(&grown(members, &held, brood_radial, target), &held)
}

/// Each card's angle and its depth: a node takes the middle of the wedge its leaves earn it.
///
/// This is what makes a web read: a whole subtree sits in one wedge of the sector, so a spoke
/// never crosses the wedge next door, and a card is always somewhere under its parent.
///
/// A list rather than a map, because the order it comes out in is the depth-first walk and both
/// callers read it as an order.
fn wedges(
    roots: &[AgentId],
    kids: &HashMap<AgentId, Vec<AgentId>>,
    lo: f32,
    hi: f32,
) -> Vec<(AgentId, (f32, usize))> {
    let mut out: Vec<(AgentId, (f32, usize))> = Vec::new();
    let weight: Vec<usize> = roots.iter().map(|root| leaves_of(*root, kids)).collect();
    let total = weight.iter().sum::<usize>().max(1) as f32;

    let mut pending: Vec<(AgentId, f32, f32, usize)> = Vec::new();
    let mut at = lo;
    for (root, hold) in roots.iter().zip(&weight) {
        let span = (hi - lo) * *hold as f32 / total;
        pending.push((*root, at, at + span, 0));
        at += span;
    }
    while let Some((node, left, right, depth)) = pending.pop() {
        out.push((node, ((left + right) / 2.0, depth)));
        let brood = kids_of(kids, node);
        if brood.is_empty() {
            continue;
        }
        let share = brood
            .iter()
            .map(|k| leaves_of(*k, kids))
            .sum::<usize>()
            .max(1) as f32;
        let mut cursor = left;
        for child in brood {
            let span = (right - left) * leaves_of(*child, kids) as f32 / share;
            pending.push((*child, cursor, cursor + span, depth + 1));
            cursor += span;
        }
    }
    out
}

/// One hub per container, its cards on concentric downward arcs, its spokes the parent links.
///
/// A radial tree in a sector rather than a full turn, so every spoke still points down the page.
/// Each arc is pushed out far enough to seat the row on it — a web whose outer rings are as
/// crowded as its inner ones is not a web, it is a pile.
pub fn inside_spider(
    members: &[&WorkAgent],
    rings: &Rings,
    ring: RingFn,
    _target: f32,
) -> Contents {
    if members.is_empty() {
        return Contents::empty();
    }
    let held = Boxes::of(members, rings, ring);
    let (roots, kids) = family(members);
    let seats = wedges(&roots, &kids, 180.0 - SPIDER_SPREAD, 180.0 + SPIDER_SPREAD);

    let deepest = seats.iter().map(|(_, (_, d))| *d).max().unwrap_or(0);
    let mut rows: Vec<Vec<AgentId>> = vec![Vec::new(); deepest + 1];
    for (ident, (_, depth)) in &seats {
        rows[*depth].push(*ident);
    }

    let sector = (2.0 * SPIDER_SPREAD).to_radians();
    let mut reach: Vec<f32> = Vec::with_capacity(rows.len());
    for (depth, row) in rows.iter().enumerate() {
        let tall = row.iter().map(|a| held.box_of(*a).1).fold(0.0f32, f32::max);
        let wide: f32 = row.iter().map(|a| held.box_of(*a).0 + CARD_GAP_X).sum();
        let mut want = (wide / sector).max(tall / 2.0);
        if depth > 0 {
            let prev = rows[depth - 1]
                .iter()
                .map(|a| held.box_of(*a).1)
                .fold(0.0f32, f32::max);
            want = want.max(reach[depth - 1] + prev / 2.0 + CARD_GAP_Y + tall / 2.0);
        }
        reach.push(want);
    }

    let mut spots: Spots = Vec::with_capacity(seats.len());
    for (ident, (turn, depth)) in &seats {
        let rad = turn.to_radians();
        let held_box = held.box_of(*ident);
        spots.push((
            *ident,
            (
                reach[*depth] * rad.sin() - held_box.0 / 2.0,
                -reach[*depth] * rad.cos() - held_box.1 / 2.0,
            ),
        ));
    }
    spread(&mut spots, &held, CARD_GAP_X);
    contents_of(&spots, &held)
}

/// Cards on a honeycomb lattice: a row per hand-off depth, every other row half a cell over.
///
/// The stagger is what a hex grid is worth here — a card sits between the two above it rather than
/// directly under one, so a spoke from either parent reaches it without running past a sibling.
pub fn inside_hex(members: &[&WorkAgent], rings: &Rings, ring: RingFn, target: f32) -> Contents {
    if members.is_empty() {
        return Contents::empty();
    }
    let held = Boxes::of(members, rings, ring);
    let (roots, kids) = family(members);

    // The depth-first order, so a family stays together in its row.
    let mut seats = wedges(&roots, &kids, 0.0, 1.0);
    seats.sort_by(|a, b| a.1.1.cmp(&b.1.1).then(a.1.0.total_cmp(&b.1.0)));

    let mut rows: Vec<Vec<AgentId>> = Vec::new();
    for (ident, (_, depth)) in &seats {
        while rows.len() <= *depth {
            rows.push(Vec::new());
        }
        rows[*depth].push(*ident);
    }

    let cell = members
        .iter()
        .map(|m| held.box_of(m.id).0)
        .fold(0.0f32, f32::max)
        + CARD_GAP_X;
    let tall = members
        .iter()
        .map(|m| held.box_of(m.id).1)
        .fold(0.0f32, f32::max);
    // A depth row wider than the screen folds onto the next lattice rows.
    let mut lattice: Vec<Vec<AgentId>> = Vec::new();
    for row in &rows {
        let per = break_to(row.len(), (cell, tall), target);
        lattice.extend(row.chunks(per.max(1)).map(|chunk| chunk.to_vec()));
    }

    let mut spots: Spots = Vec::new();
    let mut y = 0.0f32;
    for (depth, row) in lattice.iter().enumerate() {
        let wide = row.len() as f32 * cell - CARD_GAP_X;
        let mut x = -wide / 2.0 + if depth % 2 == 1 { cell / 2.0 } else { 0.0 };
        for ident in row {
            spots.push((
                *ident,
                (x + (cell - CARD_GAP_X - held.box_of(*ident).0) / 2.0, y),
            ));
            x += cell;
        }
        y += row.iter().map(|a| held.box_of(*a).1).fold(0.0f32, f32::max) + CARD_GAP_Y;
    }
    contents_of(&spots, &held)
}

/// Each disjoint family of cards arranged on its own, the families packed against the screen.
///
/// No root is assumed and none is privileged: a container holding four unrelated cards is four
/// islands, and a container holding one tree is one. The gap between two islands is what says they
/// are not the same piece of work — which a single column of cards never says at all.
pub fn inside_islands(
    members: &[&WorkAgent],
    rings: &Rings,
    ring: RingFn,
    target: f32,
) -> Contents {
    if members.is_empty() {
        return Contents::empty();
    }
    let held = Boxes::of(members, rings, ring);
    let (roots, kids) = family(members);

    let mut blocks: Vec<Spots> = Vec::with_capacity(roots.len());
    for root in roots {
        let mut crew: Vec<&WorkAgent> = Vec::new();
        let mut rank = vec![root];
        while !rank.is_empty() {
            let node = rank.remove(0);
            if let Some(member) = members.iter().find(|m| m.id == node) {
                crew.push(member);
            }
            rank.extend_from_slice(kids_of(&kids, node));
        }
        // `fold_to` answers where the *card* goes; the island is packed by its box.
        let laid = held.fold_to(&rows_by_depth(&crew), target);
        blocks.push(
            laid.cards
                .iter()
                .map(|(a, at)| {
                    let lead = held.lead_of(*a);
                    (*a, (at.0 - lead.0, at.1 - lead.1))
                })
                .collect(),
        );
    }
    contents_of(&as_islands(&blocks, &held, target, ISLAND_GAP), &held)
}

// ── the packer that never assumes a root ──────────────────────────────────────

/// One island mid-pack: which containers are in it, and where each one sits inside it.
type Island = (Vec<usize>, Vec<(f32, f32)>);

/// The connected groups of containers, over the parent relation read as undirected.
pub fn families_of(count: usize, parents: &[Option<usize>]) -> Vec<Vec<usize>> {
    fn find(up: &mut [usize], mut at: usize) -> usize {
        while up[at] != at {
            let grand = up[up[at]];
            up[at] = grand;
            at = grand;
        }
        at
    }

    let mut up: Vec<usize> = (0..count).collect();
    for (node, above) in parents.iter().enumerate().take(count) {
        let Some(above) = *above else { continue };
        if above >= count {
            continue;
        }
        let (a, b) = (find(&mut up, node), find(&mut up, above));
        if a != b {
            up[a] = b;
        }
    }

    let mut order: Vec<usize> = Vec::new();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for node in 0..count {
        let root = find(&mut up, node);
        let group = groups.entry(root).or_default();
        if group.is_empty() {
            order.push(root);
        }
        group.push(node);
    }
    order
        .into_iter()
        .filter_map(|root| groups.remove(&root))
        .collect()
}

/// Containers grouped by what they are connected to, each group packed as its own island.
///
/// Tree needs a root to hang a forest off and gives every container one row of depth whether the
/// graph has that shape or not. This one reads the same forest as a set of *components*: two
/// sessions' worth of unrelated work comes out as two islands side by side against the screen's
/// rectangle, and a graph with no parent edges at all comes out as one island, packed tight.
pub fn pack_islands(sizes: &[(f32, f32)], parents: &[Option<usize>], target: f32) -> Packing {
    let kept_parents = remap(sizes, parents);

    without_empties(sizes, |kept| {
        let groups = families_of(kept.len(), &kept_parents);
        let mut spots: Vec<(f32, f32)> = vec![(0.0, 0.0); kept.len()];
        let mut blocks: Vec<Island> = Vec::with_capacity(groups.len());
        let mut shapes: Vec<(f32, f32)> = Vec::with_capacity(groups.len());
        for group in groups {
            let inner: Vec<(f32, f32)> = group.iter().map(|&ix| kept[ix]).collect();
            let (at, extent) = pack_best(&inner, TASK_GAP, target, true);
            blocks.push((group, at));
            shapes.push(extent);
        }

        let (outer, _) = pack_best(&shapes, ISLAND_GAP, target, true);
        let mut edge = (0.0f32, 0.0f32);
        for (offset, (group, at)) in outer.iter().zip(&blocks) {
            for (&ix, spot) in group.iter().zip(at) {
                spots[ix] = (offset.0 + spot.0, offset.1 + spot.1);
                edge.0 = edge.0.max(spots[ix].0 + kept[ix].0);
                edge.1 = edge.1.max(spots[ix].1 + kept[ix].1);
            }
        }
        (spots, edge)
    })
}
