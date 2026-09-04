use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

const ASSETS: &[&str] = &["template.html", "style.css", "script.js"];

fn main() {
    for name in ASSETS {
        println!("cargo:rerun-if-changed=assets/web-export/{name}");
    }

    if env::var("PROFILE").as_deref() != Ok("release") {
        return;
    }

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    for name in ASSETS {
        let src = fs::read(format!("assets/web-export/{name}")).expect("web-export asset exists");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(&src)
            .expect("gzip encode web-export asset");
        let compressed = encoder.finish().expect("finish gzip stream");
        fs::write(Path::new(&out_dir).join(format!("{name}.gz")), compressed)
            .expect("write compressed web-export asset");
    }
}
