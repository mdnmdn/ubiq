//! The generic, id-keyed, group-ordered contribution list — written once, and never again.
//!
//! A container (settings sections, rail modes, …) is a `Registry<Spec>` plus a `render`/`centre`
//! field on `Spec` itself; this module owns none of that, only the ordering and the four
//! operations every container gets for free.

use std::collections::HashSet;

use super::SlotId;

/// Something a [`Registry`] can hold: it knows its own id.
pub trait Slotted {
    fn id(&self) -> SlotId;
}

struct Entry<T> {
    group: SlotId,
    /// Registration order, which is boot order — the tiebreaker within one group (`D177`).
    seq: usize,
    item: T,
}

/// An id-keyed, group-ordered contribution list.
///
/// Insert, relabel, reorder and remove are the only operations (`D177`); there is no rebind. A
/// second [`insert`](Registry::insert) under an id already present panics — replacing behaviour in
/// an existing item's place means removing it and inserting a new one under a different id, so the
/// registry is always literally true about what a given id does.
///
/// [`resolve`](Registry::resolve) computes the draw order once and caches it: never walk a
/// `Registry` in a draw path. Any mutation invalidates the cache.
pub struct Registry<T: Slotted> {
    entries: Vec<Entry<T>>,
    removed: Vec<SlotId>,
    cache: Option<Vec<usize>>,
}

impl<T: Slotted> Default for Registry<T> {
    fn default() -> Self {
        Registry {
            entries: Vec::new(),
            removed: Vec::new(),
            cache: None,
        }
    }
}

impl<T: Slotted> Registry<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a new contribution into `group`.
    ///
    /// # Panics
    /// If `item`'s id is already registered — a duplicate id is always a bug, base or Studio, and
    /// there is no silent merge.
    pub fn insert(&mut self, group: SlotId, item: T) {
        let id = item.id();
        assert!(
            self.entries.iter().all(|e| e.item.id() != id),
            "Registry: duplicate SlotId `{id}`"
        );
        let seq = self.entries.len();
        self.entries.push(Entry { group, seq, item });
        self.cache = None;
    }

    /// Mutates an existing contribution in place — a relabel, never a rebind (`D177`): the id, its
    /// group and its position are the registry's to keep track of, everything else is the caller's.
    ///
    /// # Panics
    /// If `id` is not registered.
    pub fn relabel(&mut self, id: SlotId, f: impl FnOnce(&mut T)) {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.item.id() == id)
            .unwrap_or_else(|| panic!("Registry: relabel of unknown SlotId `{id}`"));
        f(&mut entry.item);
        self.cache = None;
    }

    /// Moves an existing contribution to a different group.
    ///
    /// # Panics
    /// If `id` is not registered.
    pub fn reorder(&mut self, id: SlotId, group: SlotId) {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.item.id() == id)
            .unwrap_or_else(|| panic!("Registry: reorder of unknown SlotId `{id}`"));
        entry.group = group;
        self.cache = None;
    }

    /// Declares `id` removed.
    ///
    /// Recorded rather than applied immediately: the item a removal names may not be registered
    /// yet at the point the removal is declared (a second edition composes after the base).
    /// Whether it ever resolves is [`resolve`](Registry::resolve)'s loud boot assertion.
    pub fn remove(&mut self, id: SlotId) {
        self.removed.push(id);
        self.cache = None;
    }

    /// Resolves the draw order: `groups`' declared order first, then registration order within a
    /// group. Resolves once and caches; a later call after a mutation resolves again.
    ///
    /// # Panics
    /// - If a declared removal never resolved against a registered id — `D177`'s deliberate trade:
    ///   a rename is a compile error in the base and a boot assertion in a second edition, never a
    ///   silent no-op.
    /// - If a registered item's group is not one of `groups` — the container's own declared list
    ///   is what every item is ordered against, and an item outside it is a misconfiguration, not
    ///   data to degrade gracefully on.
    pub fn resolve(&mut self, groups: &[SlotId]) -> Vec<&T> {
        for removed_id in &self.removed {
            assert!(
                self.entries.iter().any(|e| e.item.id() == *removed_id),
                "Registry: removal of unresolved SlotId `{removed_id}` — nothing registered under \
                 that id; a rename must update the removal too"
            );
        }

        if self.cache.is_none() {
            let removed: HashSet<SlotId> = self.removed.iter().copied().collect();
            // Computed eagerly, one key per item, rather than inside the `sort_by_key` comparator:
            // the standard sort skips calling the key function at all below two elements, which
            // would let an undeclared group on a lone item through unnoticed.
            let mut keyed: Vec<(usize, usize, usize)> = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !removed.contains(&e.item.id()))
                .map(|(i, _)| i)
                .map(|i| {
                    let entry = &self.entries[i];
                    let group_rank = groups
                        .iter()
                        .position(|g| *g == entry.group)
                        .unwrap_or_else(|| {
                            panic!(
                                "Registry: item `{}` is in undeclared group `{}`",
                                entry.item.id(),
                                entry.group
                            )
                        });
                    (group_rank, entry.seq, i)
                })
                .collect();
            keyed.sort_by_key(|&(group_rank, seq, _)| (group_rank, seq));
            self.cache = Some(keyed.into_iter().map(|(_, _, i)| i).collect());
        }

        self.cache
            .as_ref()
            .expect("just computed")
            .iter()
            .map(|&i| &self.entries[i].item)
            .collect()
    }

    /// Whether `id` is currently registered (ignoring pending removals) — the check a boot-time
    /// removal assertion is built from.
    pub fn contains(&self, id: SlotId) -> bool {
        self.entries.iter().any(|e| e.item.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Item {
        id: SlotId,
        label: &'static str,
    }

    impl Slotted for Item {
        fn id(&self) -> SlotId {
            self.id
        }
    }

    const APP: SlotId = SlotId::new("group/app");
    const PROJECT: SlotId = SlotId::new("group/project");
    const GROUPS: &[SlotId] = &[APP, PROJECT];

    fn item(id: &'static str, label: &'static str) -> Item {
        Item {
            id: SlotId::new(id),
            label,
        }
    }

    #[test]
    fn orders_by_declared_group_then_registration() {
        let mut reg = Registry::default();
        reg.insert(PROJECT, item("b", "B"));
        reg.insert(APP, item("a", "A"));
        reg.insert(PROJECT, item("c", "C"));

        let order: Vec<&str> = reg.resolve(GROUPS).into_iter().map(|i| i.id.0).collect();
        assert_eq!(order, ["a", "b", "c"]);
    }

    #[test]
    #[should_panic(expected = "duplicate SlotId")]
    fn refuses_duplicate_id() {
        let mut reg = Registry::default();
        reg.insert(APP, item("a", "A"));
        reg.insert(APP, item("a", "A again"));
    }

    #[test]
    fn relabel_mutates_in_place_without_moving_it() {
        let mut reg = Registry::default();
        reg.insert(APP, item("a", "A"));
        reg.relabel(SlotId::new("a"), |i| i.label = "Renamed");

        let resolved = reg.resolve(GROUPS);
        assert_eq!(resolved[0].label, "Renamed");
    }

    #[test]
    fn reorder_moves_an_item_to_another_group() {
        let mut reg = Registry::default();
        reg.insert(PROJECT, item("a", "A"));
        reg.insert(APP, item("b", "B"));
        // "a" starts in PROJECT, after "b"'s group in the declared order; moving it to APP puts
        // it ahead of "b", which stays put.
        reg.reorder(SlotId::new("a"), APP);

        let order: Vec<&str> = reg.resolve(GROUPS).into_iter().map(|i| i.id.0).collect();
        assert_eq!(order, ["a", "b"]);
    }

    #[test]
    fn remove_drops_a_registered_item() {
        let mut reg = Registry::default();
        reg.insert(APP, item("a", "A"));
        reg.insert(APP, item("b", "B"));
        reg.remove(SlotId::new("a"));

        let order: Vec<&str> = reg.resolve(GROUPS).into_iter().map(|i| i.id.0).collect();
        assert_eq!(order, ["b"]);
    }

    #[test]
    #[should_panic(expected = "removal of unresolved SlotId")]
    fn unresolved_removal_panics_loudly() {
        let mut reg: Registry<Item> = Registry::default();
        reg.remove(SlotId::new("ubiq.never.registered"));
        reg.resolve(GROUPS);
    }

    #[test]
    #[should_panic(expected = "relabel of unknown SlotId")]
    fn relabel_of_unknown_id_panics() {
        let mut reg: Registry<Item> = Registry::default();
        reg.relabel(SlotId::new("missing"), |_| {});
    }

    #[test]
    #[should_panic(expected = "undeclared group")]
    fn item_outside_declared_groups_panics_on_resolve() {
        let mut reg = Registry::default();
        reg.insert(SlotId::new("group/unknown"), item("a", "A"));
        reg.resolve(GROUPS);
    }
}
