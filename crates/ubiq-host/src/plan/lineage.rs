//! Lineage codes: a short, ordered, readable name for each block of a document, which says where a
//! block came from (`D208`).
//!
//! **In memory only, and never an anchor.** The persisted anchor is the block's ULID
//! ([`ubiq_proto::ids::BlockId`]) and stays so; a code rides beside it on the wire
//! (`PlanBlock::lineage`) and is minted by the host alone. A document's codes are minted flat when
//! it is opened — `AAA`, `AAB`, … in document order — and carried across every save:
//!
//! - a block that keeps its id keeps its code;
//! - a block split into several gives each part a child code under its own — `AAB` →
//!   `AAB.AA`, `AAB.AB` — and splitting a part again goes one level deeper;
//! - a block inserted between two others takes a code ordered between theirs ([`between`]);
//! - a merged or deleted block's code goes with its id.
//!
//! Reopening the document mints flat codes again, which is the compaction.
//!
//! **Order is byte order**: `.` sorts below every letter, so a parent sorts before its children
//! and its children before its next sibling.

use std::collections::{HashMap, HashSet};

use ubiq_proto::ids::BlockId;

/// Width of a flat code: 26³ blocks before a fourth letter is needed.
const FLAT_WIDTH: usize = 3;
/// Width of one child segment.
const CHILD_WIDTH: usize = 2;

/// `n` codes of at least `width` letters, ascending: `AAA`, `AAB`, ….
pub fn mint(n: usize, width: usize) -> Vec<String> {
    let mut width = width.max(1);
    while 26usize.saturating_pow(width as u32) < n {
        width += 1;
    }
    (0..n)
        .map(|mut i| {
            let mut code = vec![b'A'; width];
            for slot in code.iter_mut().rev() {
                *slot = b'A' + (i % 26) as u8;
                i /= 26;
            }
            String::from_utf8(code).expect("ASCII letters")
        })
        .collect()
}

/// The child codes of `parent` split into `n` parts, in order.
pub fn split(parent: &str, n: usize) -> Vec<String> {
    mint(n, CHILD_WIDTH)
        .into_iter()
        .map(|segment| format!("{parent}.{segment}"))
        .collect()
}

/// A code strictly between `lo` and `hi` (`hi` absent: no upper bound), never ending in `.`.
///
/// Fractional indexing over the digits `.` = 0 and `A`…`Z` = 1…26: walk the common prefix, and
/// at the first position with room between the two digits take the middle.
pub fn between(lo: &str, hi: Option<&str>) -> String {
    let digit = |c: u8| if c == b'.' { 0 } else { (c - b'A' + 1) as i32 };
    let lo: Vec<i32> = lo.bytes().map(digit).collect();
    let hi: Option<Vec<i32>> = hi.map(|hi| hi.bytes().map(digit).collect());
    let mut out: Vec<i32> = Vec::new();
    let mut open = hi.is_none();
    for i in 0.. {
        let a = lo.get(i).copied().unwrap_or(0);
        let b = if open {
            27
        } else {
            hi.as_ref().and_then(|hi| hi.get(i).copied()).unwrap_or(0)
        };
        if b - a > 1 {
            out.push((a + b) / 2);
            break;
        }
        out.push(a);
        if a < b {
            open = true;
        }
        // A malformed pair (lo not below hi) cannot loop forever: past both, the upper bound opens.
        if i > lo.len() + hi.as_ref().map_or(0, Vec::len) {
            open = true;
        }
    }
    out.into_iter()
        .map(|d| if d == 0 { '.' } else { (b'A' + (d - 1) as u8) as char })
        .collect()
}

/// Give every block in `order` a code, keeping those `codes` already holds: a document with none
/// is minted flat, and a block missing one takes a code between its neighbours'.
pub fn fill(codes: &mut HashMap<BlockId, String>, order: &[BlockId]) {
    if !order.iter().any(|id| codes.contains_key(id)) {
        for (id, code) in order.iter().zip(mint(order.len(), FLAT_WIDTH)) {
            codes.insert(*id, code);
        }
        return;
    }
    let mut used: HashSet<String> = codes.values().cloned().collect();
    for (at, id) in order.iter().enumerate() {
        if codes.contains_key(id) {
            continue;
        }
        let left = order[..at]
            .iter()
            .rev()
            .find_map(|id| codes.get(id))
            .cloned()
            .unwrap_or_default();
        let right = order[at + 1..].iter().find_map(|id| codes.get(id)).cloned();
        // A neighbour out of order (a moved block) leaves no room between: go above the left.
        let right = right.filter(|right| *right > left);
        let mut code = between(&left, right.as_deref());
        while used.contains(&code) {
            code = between(&code, right.as_deref());
        }
        used.insert(code.clone());
        codes.insert(*id, code);
    }
}

/// What a save did to the codes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Carried {
    pub codes: HashMap<BlockId, String>,
    /// Each previous block that was split, with the blocks it became in document order — what a
    /// thread anchored on it follows.
    pub children: HashMap<BlockId, Vec<BlockId>>,
}

/// One block as the carry reads it: id, and text.
pub type Seen<'a> = (BlockId, &'a str);

/// Carry `codes` (the previous blocks') across a save from `prev` to `next`.
///
/// A split is read from the text: a block new in `next` whose text sits inside a previous block
/// that is gone, or that survives shrunk to a part of itself, is a part of that block — and so is
/// the shrunk survivor. Everything else keeps its code by id or is filled between its neighbours.
pub fn carry(prev: &[Seen<'_>], next: &[Seen<'_>], codes: &HashMap<BlockId, String>) -> Carried {
    let normalise = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let prev_ids: HashSet<BlockId> = prev.iter().map(|(id, _)| *id).collect();
    let next_text: HashMap<BlockId, String> =
        next.iter().map(|(id, text)| (*id, normalise(text))).collect();
    let prev_text: Vec<(BlockId, String)> =
        prev.iter().map(|(id, text)| (*id, normalise(text))).collect();
    // A previous block can have split when it is gone, or survives as a strict part of itself.
    let splittable = |id: &BlockId, old: &str| match next_text.get(id) {
        None => true,
        Some(now) => now != old && !now.is_empty() && old.contains(now.as_str()),
    };

    let mut children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for (id, text) in next {
        if prev_ids.contains(id) {
            continue;
        }
        let text = normalise(text);
        if text.is_empty() {
            continue;
        }
        let parent = prev_text.iter().find(|(parent, old)| {
            old.len() > text.len() && old.contains(&text) && splittable(parent, old)
        });
        if let Some((parent, _)) = parent {
            children.entry(*parent).or_default().push(*id);
        }
    }
    // The shrunk survivor is a part too, in its place in the document.
    for (parent, parts) in children.iter_mut() {
        if next_text.contains_key(parent) {
            parts.push(*parent);
        }
        parts.sort_by_key(|part| next.iter().position(|(id, _)| id == part));
    }

    let mut out: HashMap<BlockId, String> = HashMap::new();
    for (parent, parts) in &children {
        let Some(code) = codes.get(parent) else {
            continue;
        };
        for (part, child) in parts.iter().zip(split(code, parts.len())) {
            out.insert(*part, child);
        }
    }
    let split_parts: HashSet<BlockId> = children.values().flatten().copied().collect();
    for (id, _) in next {
        if !split_parts.contains(id)
            && let Some(code) = codes.get(id)
        {
            out.insert(*id, code.clone());
        }
    }
    // Nothing surviving reads as a fresh document, minted flat.
    let order: Vec<BlockId> = next.iter().map(|(id, _)| *id).collect();
    fill(&mut out, &order);
    Carried {
        codes: out,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_is_flat_ordered_and_widens() {
        assert_eq!(mint(3, 3), vec!["AAA", "AAB", "AAC"]);
        assert_eq!(mint(28, 1)[27], "BB");
        let codes = mint(100, 3);
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn split_children_sort_between_parent_and_next_sibling() {
        assert_eq!(split("AAB", 2), vec!["AAB.AA", "AAB.AB"]);
        assert_eq!(split("AAB.AB", 2)[0], "AAB.AB.AA");
        let mut all = vec!["AAC".to_string(), "AAB.AB".into(), "AAB".into(), "AAB.AA".into()];
        all.sort();
        assert_eq!(all, vec!["AAB", "AAB.AA", "AAB.AB", "AAC"]);
    }

    #[test]
    fn between_is_strictly_between() {
        for (lo, hi) in [
            ("AAB", Some("AAC")),
            ("AAB", Some("AAB.AA")),
            ("AAB.AA", Some("AAB.AB")),
            ("", Some("AAA")),
            ("AAZ", None),
        ] {
            let mid = between(lo, hi);
            assert!(mid.as_str() > lo, "{mid} > {lo}");
            if let Some(hi) = hi {
                assert!(mid.as_str() < hi, "{mid} < {hi}");
            }
            assert!(!mid.ends_with('.'));
        }
    }

    #[test]
    fn fill_mints_flat_then_inserts_between() {
        let ids: Vec<BlockId> = (0..3).map(|_| BlockId::generate()).collect();
        let mut codes = HashMap::new();
        fill(&mut codes, &ids);
        assert_eq!(codes[&ids[1]], "AAB");
        let inserted = BlockId::generate();
        let order = vec![ids[0], inserted, ids[1], ids[2]];
        fill(&mut codes, &order);
        assert!(codes[&inserted] > codes[&ids[0]] && codes[&inserted] < codes[&ids[1]]);
    }

    #[test]
    fn a_split_block_gives_its_parts_child_codes() {
        let (a, b, c, part) = (
            BlockId::generate(),
            BlockId::generate(),
            BlockId::generate(),
            BlockId::generate(),
        );
        let codes: HashMap<BlockId, String> =
            [(a, "AAA"), (b, "AAB"), (c, "AAC")].map(|(i, s)| (i, s.to_string())).into();
        let prev = [(a, "# T"), (b, "One. Two."), (c, "Three.")];
        // `b` survives as its first half; `part` is its second.
        let next = [(a, "# T"), (b, "One."), (part, "Two."), (c, "Three.")];
        let carried = carry(&prev, &next, &codes);
        assert_eq!(carried.codes[&b], "AAB.AA");
        assert_eq!(carried.codes[&part], "AAB.AB");
        assert_eq!(carried.codes[&c], "AAC");
        assert_eq!(carried.children[&b], vec![b, part]);
    }

    #[test]
    fn renumbering_is_a_fresh_flat_mint() {
        let ids: Vec<BlockId> = (0..2).map(|_| BlockId::generate()).collect();
        let mut codes: HashMap<BlockId, String> =
            [(ids[0], "AAB.AA".to_string()), (ids[1], "AAB.AB".to_string())].into();
        codes.clear();
        fill(&mut codes, &ids);
        assert_eq!((codes[&ids[0]].as_str(), codes[&ids[1]].as_str()), ("AAA", "AAB"));
    }
}
