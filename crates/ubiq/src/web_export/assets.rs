//! Embedded doc-viewer chrome (see `assets/web-export/`). The template is always plain text —
//! it's a substitution source for each response, never sent to a client verbatim, so gzipping
//! it would only cost a decompression this crate has no runtime dependency to do. CSS and JS
//! are served byte-for-byte at `/_assets/...`, so those two are gzip-precompressed in release
//! builds by `build.rs` and sent with a `Content-Encoding: gzip` header.
//!
//! `assets/web/` holds the web-panel bridge shim and the `demo` tenant chrome, served under
//! `/_web/<app>/<token>/...`. Same gzip-precompression scheme, via the same `build.rs` loop.

pub static TEMPLATE_HTML: &str = include_str!("../../assets/web-export/template.html");

/// The Excalidraw chrome, which is a template for the same reason the doc viewer's is: the route
/// substitutes a per-response CSP nonce into it, so it is never sent verbatim and gzipping it
/// would only cost a decompression. `app.js` beside it is bytes, and is gzipped like the rest.
pub static EXCALIDRAW_INDEX_HTML: &str = include_str!("../../assets/web/excalidraw/index.html");

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
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/web-export-style.css.gz")),
        content_type: "text/css; charset=utf-8",
        gzip: true,
    }
}

#[cfg(not(debug_assertions))]
pub fn script_js() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/web-export-script.js.gz")),
        content_type: "text/javascript; charset=utf-8",
        gzip: true,
    }
}

#[cfg(debug_assertions)]
pub fn bridge_js() -> Asset {
    Asset {
        bytes: include_str!("../../assets/web/bridge.js").as_bytes(),
        content_type: "text/javascript; charset=utf-8",
        gzip: false,
    }
}

#[cfg(not(debug_assertions))]
pub fn bridge_js() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/web-bridge.js.gz")),
        content_type: "text/javascript; charset=utf-8",
        gzip: true,
    }
}

#[cfg(debug_assertions)]
pub fn demo_index_html() -> Asset {
    Asset {
        bytes: include_str!("../../assets/web/demo/index.html").as_bytes(),
        content_type: "text/html; charset=utf-8",
        gzip: false,
    }
}

#[cfg(not(debug_assertions))]
pub fn demo_index_html() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/web-demo-index.html.gz")),
        content_type: "text/html; charset=utf-8",
        gzip: true,
    }
}

#[cfg(debug_assertions)]
pub fn demo_app_js() -> Asset {
    Asset {
        bytes: include_str!("../../assets/web/demo/app.js").as_bytes(),
        content_type: "text/javascript; charset=utf-8",
        gzip: false,
    }
}

#[cfg(not(debug_assertions))]
pub fn demo_app_js() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/web-demo-app.js.gz")),
        content_type: "text/javascript; charset=utf-8",
        gzip: true,
    }
}

#[cfg(debug_assertions)]
pub fn excalidraw_app_js() -> Asset {
    Asset {
        bytes: include_str!("../../assets/web/excalidraw/app.js").as_bytes(),
        content_type: "text/javascript; charset=utf-8",
        gzip: false,
    }
}

#[cfg(not(debug_assertions))]
pub fn excalidraw_app_js() -> Asset {
    Asset {
        bytes: include_bytes!(concat!(env!("OUT_DIR"), "/web-excalidraw-app.js.gz")),
        content_type: "text/javascript; charset=utf-8",
        gzip: true,
    }
}
