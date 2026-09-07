// The Windows executable carries its icon as a resource. `embed-resource` compiles
// res/ubiq-app.rc with the platform C compiler (windres on the GNU toolchain, rc.exe on
// MSVC) and links the .ico into the binary. The ico lives in ../../assets, next to the
// source artwork; both compilers are handed that directory as an include search path
// relative to the crate root. Under the `cfg(windows)` gate this is a no-op on macOS,
// which keeps the reference build untouched.
fn main() {
    link_swift_runtime();

    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=res/ubiq-app.rc");
        println!("cargo:rerun-if-changed=../../assets/AppIcon.ico");
        embed_resource::compile(
            "res/ubiq-app.rc",
            embed_resource::ParamsIncludeDirs(["../../assets"]),
        )
        .manifest_optional()
        .expect("failed to embed the Windows app icon");
    }
}

/// The Swift runtime is linked in by the host's on-device assist backend, which compiles a Swift
/// bridge. Most of it is referenced by absolute path and resolves on its own, but
/// `libswift_Concurrency.dylib` is back-deployable and is referenced as
/// `@rpath/libswift_Concurrency.dylib`. A Rust link emits no `LC_RPATH` entries, so `@rpath`
/// expands to nothing and the binary aborts at launch with "Library not loaded" — both when run
/// directly and when bundled.
///
/// This is here rather than in `ubiq-host` because `cargo:rustc-link-arg` does not propagate to
/// dependents: only the binary crate's own build script reaches the final link. `ubiq-app` is
/// already the only crate that names both halves.
///
/// macOS keeps the library in the dyld shared cache rather than on disk, so `/usr/lib/swift` looks
/// empty to `ls` while still being the correct search path.
fn link_swift_runtime() {
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if macos && std::env::var_os("CARGO_FEATURE_ASSIST_APPLE").is_some() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
}
