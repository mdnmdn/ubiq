//! The stores, in memory.
//!
//! Not only for tests: they are what an unwritable config root falls back to. Both carry the two
//! knobs the failure paths need — a load that fails, and writes that fail — so the "corrupt
//! catalogue" and "unwritable store" behaviours are exercised without a disk, the same way
//! `crates/ubiq/tests/windows.rs` exercises the registry without a frame.

use std::collections::BTreeMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectRecord, Scope};

use super::{PreferenceStore, ProjectStore, StoreError};

#[derive(Default)]
pub struct MemoryProjectStore {
    records: RwLock<Vec<ProjectRecord>>,
    fail_writes: AtomicBool,
    fail_load: AtomicBool,
    writes: AtomicUsize,
}

impl MemoryProjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed it, as a store read back from disk would be.
    pub fn with(records: Vec<ProjectRecord>) -> Self {
        Self {
            records: RwLock::new(records),
            ..Self::default()
        }
    }

    /// Make every write fail from now on.
    pub fn fail_writes(&self, failing: bool) {
        self.fail_writes.store(failing, Ordering::Relaxed);
    }

    /// Make the next load report a corrupt file.
    pub fn fail_load(&self, failing: bool) {
        self.fail_load.store(failing, Ordering::Relaxed);
    }

    /// How many writes actually landed. What the debouncer's coalescing is asserted against.
    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }
}

impl ProjectStore for MemoryProjectStore {
    fn load(&self) -> Result<Vec<ProjectRecord>, StoreError> {
        if self.fail_load.load(Ordering::Relaxed) {
            return Err(StoreError::Parse {
                path: "memory".into(),
                preserved_as: Some("memory.corrupt".into()),
                message: "made to fail".to_string(),
            });
        }
        Ok(self
            .records
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone())
    }

    fn upsert(&self, record: &ProjectRecord) -> Result<(), StoreError> {
        // In memory first, then the failure — the same order the file store uses, so a failing
        // store still answers from what the session knows.
        {
            let mut records = self.records.write().unwrap_or_else(|e| e.into_inner());
            match records.iter_mut().find(|r| r.id == record.id) {
                Some(existing) => *existing = record.clone(),
                None => records.push(record.clone()),
            }
            records.sort_by_key(|r| r.id);
        }
        if self.fail_writes.load(Ordering::Relaxed) {
            return Err(StoreError::NotDurable);
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove(&self, id: ProjectId) -> Result<(), StoreError> {
        self.records
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|record| record.id != id);
        if self.fail_writes.load(Ordering::Relaxed) {
            return Err(StoreError::NotDurable);
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryPreferenceStore {
    values: RwLock<BTreeMap<Scope, String>>,
    fail_writes: AtomicBool,
    writes: AtomicUsize,
}

impl MemoryPreferenceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail_writes(&self, failing: bool) {
        self.fail_writes.store(failing, Ordering::Relaxed);
    }

    /// How many writes landed, which is what a debounce is measured by.
    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::Relaxed)
    }
}

impl PreferenceStore for MemoryPreferenceStore {
    fn get(&self, scope: &Scope) -> Result<Option<String>, StoreError> {
        Ok(self
            .values
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(scope)
            .cloned())
    }

    fn set(&self, scope: &Scope, value: &str) -> Result<(), StoreError> {
        if self.fail_writes.load(Ordering::Relaxed) {
            return Err(StoreError::NotDurable);
        }
        self.values
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(scope.clone(), value.to_string());
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn clear(&self, scope: &Scope) -> Result<(), StoreError> {
        self.values
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(scope);
        Ok(())
    }
}
