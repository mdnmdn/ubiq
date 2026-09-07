// The Windows executable carries its icon as a resource. `embed-resource` compiles
// res/ubiq-app.rc with the platform C compiler (windres on the GNU toolchain, rc.exe on
// MSVC) and links the .ico into the binary. The ico lives in ../../assets, next to the
// source artwork; both compilers are handed that directory as an include search path
// relative to the crate root. Under the `cfg(windows)` gate this is a no-op on macOS,
// which keeps the reference build untouched.
fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=res/ubiq-app.rc");
        println!("cargo:rerun-if-changed=../../assets/AppIcon.ico");
        embed_resource::compile("res/ubiq-app.rc", embed_resource::ParamsIncludeDirs(["../../assets"]))
            .manifest_optional()
            .expect("failed to embed the Windows app icon");
    }
}