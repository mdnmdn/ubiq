//! Embedded doc-viewer chrome (see `assets/web-export/`). The template is always plain text —
//! it's a substitution source for each response, never sent to a client verbatim, so gzipping
//! it would only cost a decompression this crate has no runtime dependency to do. CSS and JS
//! are served byte-for-byte at `/_assets/...`, so those two are gzip-precompressed in release
//! builds by `build.rs` and sent with a `Content-Encoding: gzip` header.

pub static TEMPLATE_HTML: &str = include_str!("../../assets/web-export/template.html");

/// A static asset ready to hand to `tiny_http`: its bytes, its MIME type, and whether those
/// bytes are already gzip-compressed (so the caller knows to set `Content-Encoding`).
pub struct Asset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
    pub gzip: bool,
}

#[cfg(debug_assertions)]
pub fn style_css() -> Asset {
    Asset {
        bytes: include_str!("../../assets/web-export/style.css").as_bytes(),
        content_type: "text/css; charset=utf-8",
        gzip: false,
    }
}

#[cfg(debug_assertions)]
pub fn script_js() -> Asset {
    Asset {
        bytes: include_str!("../../assets/web-export/script.js").as_bytes(),
        content_type: "text/javascript; charset=utf-8",
        gzip: false,
    }
}

#[cfg(not(debug_assertions))]
pub fn style_css() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/style.css.gz")),
        content_type: "text/css; charset=utf-8",
        gzip: true,
    }
}

#[cfg(not(debug_assertions))]
pub fn script_js() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/script.js.gz")),
        content_type: "text/javascript; charset=utf-8",
        gzip: true,
    }
}
