//! The bundle's version identity. `_devops/scripts/bundle-version.sh`, run from the Justfile,
//! sets `UBIQ_VERSION` before `cargo build` — a bundle's version has to stay fixed after it is
//! packaged, so it is baked in at compile time rather than computed when the app runs.

/// The full version string. `"dev"` outside the Justfile's build path (a bare `cargo build`).
pub const FULL: &str = match option_env!("UBIQ_VERSION") {
    Some(v) => v,
    None => "dev",
};

/// How many characters the footer shows before it gives up and elides.
const DISPLAY_LEN: usize = 8;

/// The footer's short form: the full string when it already fits, otherwise the first eight
/// characters and an ellipsis. The full value is always one hover away.
pub fn short() -> String {
    if FULL.chars().count() <= DISPLAY_LEN {
        FULL.to_string()
    } else {
        let head: String = FULL.chars().take(DISPLAY_LEN).collect();
        format!("{head}\u{2026}")
    }
}
