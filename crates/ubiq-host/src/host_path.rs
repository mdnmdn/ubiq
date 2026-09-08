//! Absolute host paths as they cross the UI/host line.
//!
//! Two rules, both Windows-only. Everywhere else this module is the identity function, so the
//! same call sites read correctly on all platforms.
//!
//! **The wire never carries the verbatim `\\?\` prefix.** `std::fs::canonicalize` returns one on
//! Windows (`\\?\C:\…`, `\\?\UNC\server\share`), which file operations want — it is what keeps
//! long paths working — but nothing else does: it does not display, it does not round-trip
//! through a UI that joins it with `/`, and `cmd.exe` refuses it outright (see `pty::spawn_cwd`,
//! which strips it for the same reason). [`wire_string`] strips it at the single point a
//! canonical path becomes a message string.
//!
//! **A verbatim path carrying `/` separators is rejected with os error 123.** Past the `\\?\`
//! prefix only `\` is legal, so `\\?\C:/works` — exactly what a `/`-joining UI builds from a
//! verbatim parent — fails to canonicalise. [`request_path`] normalises `/` to `\` on Windows
//! before any filesystem call, so an older UI's mixed string still lists.

use std::path::{Path, PathBuf};

/// The filesystem path a wire string names.
///
/// On Windows `/` never names a distinct file from `\`, so normalising is safe for the absolute
/// paths this family carries — and required past a `\\?\` prefix, where `/` is rejected outright.
pub fn request_path(raw: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(raw.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(raw)
    }
}

/// The wire/display string for a canonical absolute path: verbatim prefix stripped on Windows.
pub fn wire_string(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
        text.into_owned()
    }
    #[cfg(not(windows))]
    {
        text.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_separators_become_a_single_usable_path() {
        let built = request_path("C:/works/ubiq");
        assert_eq!(built, PathBuf::from(expected_mixed()));
    }

    #[test]
    fn verbatim_mixed_separators_normalise_to_verbatim_native() {
        let built = request_path(r"\\?\C:/works");
        assert_eq!(built, PathBuf::from(expected_verbatim_mixed()));
    }

    #[test]
    #[cfg(windows)]
    fn verbatim_prefix_is_stripped_for_the_wire() {
        assert_eq!(wire_string(Path::new(r"\\?\C:\works")), r"C:\works");
    }

    #[test]
    #[cfg(windows)]
    fn verbatim_unc_becomes_plain_unc_for_the_wire() {
        assert_eq!(
            wire_string(Path::new(r"\\?\UNC\server\share")),
            r"\\server\share"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn non_windows_paths_pass_through_untouched() {
        assert_eq!(wire_string(Path::new("/home/mdn")), "/home/mdn");
    }

    #[cfg(windows)]
    fn expected_mixed() -> String {
        r"C:\works\ubiq".to_string()
    }

    #[cfg(not(windows))]
    fn expected_mixed() -> String {
        "C:/works/ubiq".to_string()
    }

    #[cfg(windows)]
    fn expected_verbatim_mixed() -> String {
        r"\\?\C:\works".to_string()
    }

    #[cfg(not(windows))]
    fn expected_verbatim_mixed() -> String {
        r"\\?\C:/works".to_string()
    }
}
