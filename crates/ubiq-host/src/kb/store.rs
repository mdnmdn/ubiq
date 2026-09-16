//! A project's knowledge-base source list, as one TOML file.
//!
//! Modelled on [`crate::store::file::FileTaskStore`]: one file per project, under the project's own
//! directory in the config root, `version` at the top for a migration to read. Deliberately
//! stateless where that store is not — nothing here calls in from a settings form except through
//! [`super::Kb`], so there is no in-memory copy to keep consistent, only a file to read and a file
//! to write.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::kb::KbSource;

use crate::atomic::write_atomic;

/// The format this Ubiq writes and understands.
///
/// `2`: the field beneath `version` was renamed from `roots` to `sources` alongside the rest of
/// this family's rename — a `1` file no longer parses, since `#[serde(deny_unknown_fields)]` is
/// not set but the field this reads is now a different name, so [`load`] falls back to an empty
/// list for it, the same as any other unreadable file. That costs a re-typed URL, on this
/// module's own rule, and nothing more.
pub const KB_VERSION: u32 = 2;

/// The whole file, mirroring [`crate::store::file::TasksFile`]: `version` at the top, the rows
/// beneath it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct KbFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sources: Vec<KbSource>,
}

/// `<config root>/projects/<project ulid>/kb.toml` — beside `tasks.toml` and `view.toml`, under
/// the directory the orphan collector already sweeps.
pub fn path(config_root: &Path, project: ProjectId) -> PathBuf {
    config_root
        .join("projects")
        .join(project.to_string())
        .join("kb.toml")
}

/// A project's sources. A missing file is a project whose knowledge base was never configured,
/// which is an empty list rather than an error — [`crate::store::file::FileTaskStore::load`]'s own
/// convention, for the same reason.
pub fn load(path: &Path) -> Vec<KbSource> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };

    match toml::from_str::<KbFile>(&raw) {
        Ok(file) => file.sources,
        Err(error) => {
            // Unlike the catalogue, a source list costs the user a re-typed URL to recreate, not a
            // lost project — so it is discarded rather than preserved, on the view-state family's
            // reasoning, and the settings form simply reopens empty.
            tracing::warn!(
                "discarding an unreadable knowledge-base source list at {}: {error}",
                path.display()
            );
            Vec::new()
        }
    }
}

/// Write a project's whole source list, atomically.
pub fn save(path: &Path, sources: &[KbSource]) -> std::io::Result<()> {
    let file = KbFile {
        version: KB_VERSION,
        sources: sources.to_vec(),
    };
    // Every field here is `KbSource`, which is always representable in TOML — this only fails if
    // that ever stops being true, which a test would catch long before a user's sources did.
    let body = toml::to_string_pretty(&file).expect("a knowledge-base source list serialises");
    write_atomic(path, body.as_bytes())
}
