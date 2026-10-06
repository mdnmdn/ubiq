//! The recorded deviations (D40, "v1 frame padding"): a workflow v1 document whose geometry the
//! frame padding changes ([`padding_changes`]) is not compared with Archify's output, which it cannot
//! match, but with our own, recorded once in `tests/deviations/<test>.json` (keyed `"<case> [<run>]"`).
//! Every other document stays exact against Archify, and the predicate, not a list, says which is
//! which, so a regression elsewhere can never hide in a record. Archify parity of the v1 algorithm
//! itself is `tests/workflow_v1_golden.rs`, on the unpadded [`LegacyPlan::archify`].
//!
//! Re-record after a deliberate change with `ARCHIFY_RECORD_DEVIATIONS=1 cargo test -p ubiq-archify`,
//! and review the diff of `tests/deviations/`. A record whose case no longer deviates fails as stale.
//!
//! [`LegacyPlan::archify`]: ubiq_archify::layout::workflow::legacy::LegacyPlan::archify

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use ubiq_archify::layout::workflow::legacy::padding_changes;
use ubiq_archify::model::common::SchemaVersion;
use ubiq_archify::model::{self, Doc};
use serde_json::Value;

/// Whether `text` is a workflow v1 document the frame padding changes.
pub fn deviates(doc_type: Option<&str>, text: &str) -> bool {
    if doc_type != Some("workflow") {
        return false;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else { return false };
    match model::parse("workflow", &value) {
        Ok(Doc::Workflow(doc)) => doc.schema_version == SchemaVersion::V1 && padding_changes(&doc),
        _ => false,
    }
}

pub struct Deviations {
    file: PathBuf,
    record: bool,
    recorded: BTreeMap<String, Value>,
    seen: BTreeMap<String, Value>,
}

impl Deviations {
    /// The records of one test (`tests/deviations/<test>.json`).
    pub fn open(test: &str) -> Deviations {
        let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/deviations/{test}.json"));
        let recorded = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let record = std::env::var_os("ARCHIFY_RECORD_DEVIATIONS").is_some();
        Deviations { file, record, recorded, seen: BTreeMap::new() }
    }

    /// Compare a deviating run with its record: `None` when it matches (or is being recorded).
    pub fn check(&mut self, key: &str, ours: Value) -> Option<String> {
        let problem = match self.recorded.get(key) {
            _ if self.record => None,
            None => Some(format!("{key}: deviates from Archify (frame padding) and has no record; run with ARCHIFY_RECORD_DEVIATIONS=1")),
            Some(want) if *want != ours => Some(format!(
                "{key}: differs from its recorded deviation\n    recorded: {}\n    ours:     {}",
                excerpt(want),
                excerpt(&ours)
            )),
            Some(_) => None,
        };
        self.seen.insert(key.to_owned(), ours);
        problem
    }

    /// Write the records when recording; otherwise the stale ones (recorded, no longer deviating).
    pub fn finish(self) -> Vec<String> {
        if self.record {
            fs::create_dir_all(self.file.parent().unwrap()).unwrap();
            fs::write(&self.file, serde_json::to_string_pretty(&self.seen).unwrap() + "\n").unwrap();
            return Vec::new();
        }
        let seen: BTreeSet<&String> = self.seen.keys().collect();
        self.recorded.keys().filter(|k| !seen.contains(k)).map(|k| format!("{k}: stale deviation record")).collect()
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }
}

fn excerpt(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 400 { format!("{}…", text.chars().take(400).collect::<String>()) } else { text }
}
