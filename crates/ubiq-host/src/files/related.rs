//! What else belongs to a path — a general related-files resolver, so a rename, move or delete
//! raised on one explorer row can offer to carry along whatever else names it.
//!
//! **One rule today, and the shape is what matters, not the count.** T-119's annotation sidecar
//! (`D161`) is the first [`RelatedRule`]; a `foo.ts`/`foo.test.ts` pair or a component and its
//! stylesheet are the same shape and would be a second rule beside it, never a special case bolted
//! onto [`super::edit`]. Nothing above this module — the wire, the coordinator, the explorer —
//! knows the word "annotation"; it only knows [`ubiq_proto::files::RelatedFile`].
//!
//! Only a *file's* related files are ever resolved. A folder's own rename or delete already takes
//! everything under it — `fs::rename` and `fs::remove_dir_all` move or remove a sidecar sitting
//! inside the folder the same as any other entry — so asking here for a folder would be answering
//! a question nothing needs asked.

use std::path::Path;

use ubiq_proto::files::RelatedFile;

/// One rule: what a path's related file is, if it is actually there, and where that related file
/// goes when the path itself is renamed or moved.
trait RelatedRule {
    /// The related file beside `rel_path`, if the rule's own naming convention finds one on disk.
    /// `root` is the project's root, `rel_path` project-relative.
    fn find(&self, root: &Path, rel_path: &str) -> Option<RelatedFile>;

    /// Where the related file goes when the path it belongs to moves from `old_rel_path` to
    /// `new_rel_path`. Not consulted for [`ubiq_proto::files::PathOp::Trash`] or
    /// [`ubiq_proto::files::PathOp::Delete`], which apply the same op to [`Self::find`]'s answer
    /// unchanged.
    fn rebase(&self, old_rel_path: &str, new_rel_path: &str) -> String;
}

/// T-119's sidecar: `<file>.md.annotation.json`, beside the markdown file it annotates
/// (`crate::store::plan::sidecar_beside`, in the project-relative form that path takes here).
struct AnnotationSidecar;

impl RelatedRule for AnnotationSidecar {
    fn find(&self, root: &Path, rel_path: &str) -> Option<RelatedFile> {
        if !rel_path.to_ascii_lowercase().ends_with(".md") {
            return None;
        }
        let sidecar_rel = sidecar_rel_path(rel_path);
        if root.join(&sidecar_rel).is_file() {
            Some(RelatedFile {
                rel_path: sidecar_rel,
                label: "annotations".to_string(),
            })
        } else {
            None
        }
    }

    fn rebase(&self, _old_rel_path: &str, new_rel_path: &str) -> String {
        sidecar_rel_path(new_rel_path)
    }
}

/// The project-relative sidecar path for a markdown file — the whole file name with
/// `.annotation.json` appended, matching `crate::store::plan::sidecar_beside`'s rule for the
/// absolute path it works from. Duplicated rather than shared: that function takes a `Path` it
/// then joins onto a root, and a `rel_path` here is a wire string with no root to join.
fn sidecar_rel_path(rel_path: &str) -> String {
    format!("{rel_path}.annotation.json")
}

fn rules() -> &'static [&'static dyn RelatedRule] {
    &[&AnnotationSidecar]
}

/// Everything related to `rel_path`, across every rule — what
/// [`ubiq_proto::messages::Message::ProjectFileRelated`] answers, and what the explorer's rename
/// and delete questions describe their checkbox with.
pub fn related(root: &Path, rel_path: &str) -> Vec<RelatedFile> {
    rules()
        .iter()
        .filter_map(|rule| rule.find(root, rel_path))
        .collect()
}

/// Every related file `rel_path` actually has, paired with where it goes if `new_rel_path` is
/// given — `None` for [`ubiq_proto::files::PathOp::Trash`] and
/// [`ubiq_proto::files::PathOp::Delete`], which carry a related file by applying the same op to it
/// rather than moving it anywhere.
///
/// Resolved fresh here rather than trusting whatever [`related`] answered earlier: the answer the
/// interface was shown may be stale by the time the edit lands, and re-resolving is cheap — one
/// `stat` per rule.
pub fn carry_pairs(
    root: &Path,
    rel_path: &str,
    new_rel_path: Option<&str>,
) -> Vec<(String, Option<String>)> {
    rules()
        .iter()
        .filter_map(|rule| {
            let found = rule.find(root, rel_path)?;
            let to = new_rel_path.map(|new_rel_path| rule.rebase(rel_path, new_rel_path));
            Some((found.rel_path, to))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_markdown_file_with_no_sidecar_has_nothing_related() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.md"), "# Spec").unwrap();
        assert!(related(dir.path(), "spec.md").is_empty());
    }

    #[test]
    fn a_markdown_file_with_a_sidecar_carries_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.md"), "# Spec").unwrap();
        std::fs::write(dir.path().join("spec.md.annotation.json"), "{}").unwrap();

        let found = related(dir.path(), "spec.md");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rel_path, "spec.md.annotation.json");
        assert_eq!(found[0].label, "annotations");
    }

    #[test]
    fn a_non_markdown_file_is_never_matched() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "hi").unwrap();
        std::fs::write(dir.path().join("notes.txt.annotation.json"), "{}").unwrap();
        assert!(related(dir.path(), "notes.txt").is_empty());
    }

    #[test]
    fn carry_pairs_rebases_the_sidecar_to_the_new_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.md"), "# Spec").unwrap();
        std::fs::write(dir.path().join("spec.md.annotation.json"), "{}").unwrap();

        let pairs = carry_pairs(dir.path(), "spec.md", Some("notes.md"));
        assert_eq!(
            pairs,
            vec![(
                "spec.md.annotation.json".to_string(),
                Some("notes.md.annotation.json".to_string())
            )]
        );
    }

    #[test]
    fn carry_pairs_names_no_destination_for_a_removal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("spec.md"), "# Spec").unwrap();
        std::fs::write(dir.path().join("spec.md.annotation.json"), "{}").unwrap();

        let pairs = carry_pairs(dir.path(), "spec.md", None);
        assert_eq!(pairs, vec![("spec.md.annotation.json".to_string(), None)]);
    }
}
