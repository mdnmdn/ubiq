use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

const ASSETS: &[&str] = &[
    "web-export/template.html",
    "web-export/style.css",
    "web-export/script.js",
    "web/bridge.js",
    "web/demo/index.html",
    "web/demo/app.js",
    "web/excalidraw/app.js",
];

/// Assets that are substitution sources rather than bytes on the wire, so gzipping them would buy
/// nothing: the Excalidraw chrome carries a `__UBIQ_NONCE__` placeholder the route fills per
/// response, exactly as `web-export/template.html` does.
const TEMPLATES: &[&str] = &["web/excalidraw/index.html"];

fn main() {
    for path in ASSETS.iter().chain(TEMPLATES) {
        println!("cargo:rerun-if-changed=assets/{path}");
    }

    if env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    for path in ASSETS {
        let src = fs::read(format!("assets/{path}")).expect("web asset exists");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&src).expect("gzip encode web asset");
        let compressed = encoder.finish().expect("finish gzip stream");
        let out_name = format!("{}.gz", path.replace('/', "-"));
        fs::write(Path::new(&out_dir).join(out_name), compressed)
            .expect("write compressed web asset");
    }
}
