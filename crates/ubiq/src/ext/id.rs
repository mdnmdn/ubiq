//! The one id type every extension container and every item inside one is keyed by.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, OnceLock};

/// A stable identifier for a container or an item registered inside one.
///
/// Every registration is compile-time, so `'static` costs nothing and the id stays `Copy` — which
/// is what lets it replace a `Copy` enum in `AppState`, and sit as a `HashMap` key, without churn.
/// Hierarchical and owner-prefixed by convention: containers are base-owned paths (`settings/app`,
/// `rail`), items carry their owner (`ubiq.settings.editor`, a second edition's own prefix for its
/// own items). Base ids are consts in [`crate::ext::ids`] (`D178`); a second edition's ids are
/// string literals in its own code, so a rename in the base is a compile error there and a boot
/// assertion in it instead (`D177`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub &'static str);

impl SlotId {
    /// Const constructor, so a base id can be declared as a top-level `const`.
    pub const fn new(id: &'static str) -> Self {
        SlotId(id)
    }

    /// An id read back from a file rather than written in code.
    ///
    /// The whole point of a `SlotId` is that it is `Copy` and `'static`, and a saved blob is the
    /// one place an id arrives as an owned string — a rail mode a second edition registered, read
    /// back by a build that does not have that edition's registration (`D184`). Interning rather
    /// than leaking per call: a decode that ran once per project open would otherwise leak a
    /// string every time, and the set of ids a config root has ever written down is small and
    /// fixed.
    pub fn intern(id: &str) -> Self {
        static POOL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
        let pool = POOL.get_or_init(|| Mutex::new(HashSet::new()));
        let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = pool.get(id) {
            return SlotId(found);
        }
        let leaked: &'static str = Box::leak(id.to_owned().into_boxed_str());
        pool.insert(leaked);
        SlotId(leaked)
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
