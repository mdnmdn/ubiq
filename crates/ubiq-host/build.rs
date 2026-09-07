//! One link argument, for one feature, on one platform.
//!
//! The `assist-apple` backend compiles a Swift bridge, and the Swift concurrency runtime is
//! referenced as `@rpath/libswift_Concurrency.dylib`. A Rust link emits no `LC_RPATH` entries, so
//! `@rpath` expands to nothing and anything linked against the backend dies at load with
//! "Library not loaded" — this crate's own test binaries included, which is what makes it this
//! crate's problem and not only the binary's.
//!
//! `cargo:rustc-link-arg` does not propagate to dependents, so `crates/ubiq-app` emits its own
//! copy for the executable. This one covers the targets built here.
//!
//! macOS keeps the library in the dyld shared cache rather than on disk, so `/usr/lib/swift` looks
//! empty to `ls` while still being the correct search path.
fn main() {
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if macos && std::env::var_os("CARGO_FEATURE_ASSIST_APPLE").is_some() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
