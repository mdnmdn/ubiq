//! Mermaid, drawn here: a source string in, a picture out, and the disk tier that stops it being
//! drawn twice.
//!
//! **A Mermaid document is just text.** The bus already carries a file's bytes, and drawing is the
//! interface's job — so the renderer lives on this side of the seam, next to the viewer that shows
//! what it made. Nothing in this module crosses the bus and nothing in it knows a pane, a project
//! or a window.
//!
//! Rendering is [`merman`], a pure-Rust Mermaid implementation rather than a browser: it is called
//! synchronously, needs no main thread, no system library and no window, which is the whole reason
//! the interface can hold it at all.
//!
//! **Nothing here may run on the frame thread.** A typical diagram takes a few milliseconds, but
//! layout is superlinear and a large graph has been measured at two seconds — long enough to drop
//! a second of frames and freeze every keystroke behind it. [`AppState`](crate::app::AppState)
//! hands each render to the background executor and takes the answer back in a task; this module
//! is the plain synchronous function that runs there.
//!
//! The picture is SVG and nothing here looks inside it beyond one thing: the root `<svg>`'s
//! `viewBox`, which is the picture's size. merman emits `width="100%"` and no `height`, so the
//! `viewBox` is the only honest answer to how big the diagram is — and the viewer needs that number
//! to draw the picture at its own size rather than stretched to whatever box it landed in.
//!
//! The disk tier under this is content-addressed and **disposable**: losing it costs a re-render
//! and nothing else. It lives in the project's workarea — the directory the host reserves for the
//! interface and never reads inside — so deleting the workarea loses a cache and nothing that was
//! ever the truth.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// The renderer, as the cache key names it.
///
/// **This string and the `=` pin on `merman` in `Cargo.toml` move together.** A renderer whose
/// output changed while the key stayed the same would serve yesterday's picture forever, and the
/// cache holds no other version marker — there is no bundle to hash, because there is no bundle.
const RENDERER: &str = "merman 0.8.0-alpha.5";

/// The shape of what is written down, so a change to it invalidates every entry at once.
const CACHE_FORMAT: &str = "v1";

/// What the workarea's diagram cache is called, under the directory the host handed over.
///
/// One name, chosen here, so the workarea can hold other things later without either of them
/// having to know about the other.
const SUBDIRECTORY: &str = "diagrams";

/// What the root `<svg>`'s background is rewritten to.
///
/// merman hard-codes `background-color: white` in the root style **in both themes**, which is a
/// white card in the middle of a dark window. It is rewritten to `transparent` rather than to a
/// colour, and that is deliberate twice over: the diagram then sits on whatever surface the viewer
/// draws it on, and the cached bytes stay a pure function of the source and the palette rather than
/// of the token the surface happened to have. Only that one declaration is touched; the rest of the
/// style attribute passes through.
const BACKGROUND: &str = "transparent";

/// Which palette a diagram is drawn for.
///
/// It is asked for rather than applied afterwards: merman bakes the colours into the markup, so a
/// theme switch is a different render and a different cache entry, not a different way of drawing
/// the same one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramPalette {
    Light,
    Dark,
}

/// One rendered diagram: the SVG, and the size it wants to be drawn at.
///
/// `width` and `height` are the picture's own, read out of its `viewBox`. Drawing at them is what
/// keeps a diagram sharp instead of stretched to whatever box it landed in.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagramImage {
    pub bytes: Vec<u8>,
    pub width: f32,
    pub height: f32,
}

/// Where one project's pictures are written, under the workarea the host handed over.
///
/// **Never composed from the config root.** The workarea arrives on every project message as an
/// absolute path, and using what was sent is what makes a host on another machine a change of value
/// rather than a change of code.
pub fn cache_dir(workarea: &str) -> PathBuf {
    Path::new(workarea).join(SUBDIRECTORY)
}

// ── the render itself ───────────────────────────────────────────────

/// A document-unique id for one render.
///
/// Mermaid uses the root `id` as the prefix for every marker and gradient it puts in `<defs>`, so
/// two diagrams drawn with the same id and shown in one document would fight over each other's
/// arrowheads. A counter is enough: uniqueness is needed within a document, not across time.
fn next_diagram_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("ubiq-diagram-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The Mermaid site config one palette asks for.
fn site_config(palette: DiagramPalette) -> merman::MermaidConfig {
    let theme = match palette {
        DiagramPalette::Light => "default",
        DiagramPalette::Dark => "dark",
    };
    merman::MermaidConfig::from_value(serde_json::json!({ "theme": theme }))
}

/// Render one source, with no cache in front of it.
///
/// The error is a sentence because that is what the viewer shows above the source, and what a
/// person reading it needs: merman's parse errors already say which line and what it expected.
/// **Nothing here panics on bad input** — a source that is not a diagram at all is an ordinary
/// failure, not a bug.
pub fn render(source: &str, palette: DiagramPalette) -> Result<DiagramImage, String> {
    // `resvg_safe` is the terminal contract: it resolves `<foreignObject>` labels into real `<text>`
    // and then strips them, which is what keeps label text from vanishing in a renderer that has no
    // HTML in it. The two passes in front of it run before that contract is applied.
    let pipeline = merman::svg::SvgPipeline::resvg_safe()
        .with_postprocessor(merman::svg::CssOverridePostprocessor::strip_existing_important())
        .with_postprocessor(merman::svg::RootBackgroundPostprocessor::new(BACKGROUND));

    let rendered = merman::svg::HeadlessRenderer::new()
        .with_site_config(site_config(palette))
        // Advance tables rather than a font engine: no system library is opened, which is what
        // keeps a Linux build a plain `cargo build`. The cost is in `D19`.
        .with_vendored_text_measurer()
        .with_diagram_id(&next_diagram_id())
        .render_svg_with_pipeline_sync(source, &pipeline)
        .map_err(|error| error.to_string())?;

    // A real arm, not an impossibility: prose with no diagram in it renders nothing, and saying so
    // is the answer.
    let svg = rendered.ok_or_else(|| "no Mermaid diagram in this source".to_string())?;

    let (width, height) = view_box(&svg)
        .ok_or_else(|| "the renderer produced a picture with no usable size".to_string())?;

    Ok(DiagramImage {
        bytes: svg.into_bytes(),
        width,
        height,
    })
}

/// The picture's size, taken from the root `<svg>`'s `viewBox`.
///
/// The last two of its four numbers are the width and the height, and they are the size the viewer
/// draws at: merman writes `width="100%"` and no `height`, so a renderer resolving the picture
/// against its viewport gets exactly these numbers.
pub fn view_box(svg: &str) -> Option<(f32, f32)> {
    let start = svg.find("<svg")?;
    let tag = &svg[start..start + svg[start..].find('>')?];

    let after = &tag[tag.find("viewBox")? + "viewBox".len()..];
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &after[1..][..after[1..].find(quote)?];

    let mut numbers = value
        .split([' ', '\t', '\n', '\r', ','])
        .filter(|part| !part.is_empty())
        .skip(2)
        .map(str::parse::<f32>);
    let width = numbers.next()?.ok()?;
    let height = numbers.next()?.ok()?;

    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        .then_some((width, height))
}

// ── what a picture is filed under ───────────────────────────────────

/// What one source, in one palette, is known by — in memory and on disk alike.
///
/// Hex, and a usable file name. **The palette is in the hash** because the renderer bakes its
/// colours into the markup, so the same diagram in the two palettes is two entries rather than one
/// drawn twice. The renderer is in it too, rather than in the path, so that upgrading merman
/// orphans the old entries instead of overwriting them — and orphans in a disposable directory cost
/// only space.
pub fn key(source: &str, palette: DiagramPalette) -> String {
    let mut hash = Sha256::new();
    hash.update(CACHE_FORMAT.as_bytes());
    hash.update([0]);
    hash.update(RENDERER.as_bytes());
    hash.update([0]);
    hash.update(match palette {
        DiagramPalette::Light => b"light".as_slice(),
        DiagramPalette::Dark => b"dark".as_slice(),
    });
    hash.update([0]);
    hash.update(source.as_bytes());

    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ── the disk tier ───────────────────────────────────────────────────

/// Rendered pictures, written down in one project's workarea.
///
/// **A cache that will not read is a re-render, never an error.** Every failure below is swallowed,
/// and the worst one can cost is the work that was going to be done anyway. Only successes are
/// kept: a source that would not parse is re-tried each time it is asked for, because the next ask
/// may follow the edit that fixed it.
pub struct Disk {
    dir: PathBuf,
}

impl Disk {
    /// A cache over one directory. The directory is made when something is first written to it, so
    /// a project whose files hold no diagram leaves nothing behind.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// One entry read back, or nothing.
    ///
    /// The size is re-derived from the bytes rather than written beside them: the picture is its
    /// own metadata, so there is no second file to fall out of step and no format to version. A
    /// file that is not an SVG — truncated, or somebody else's — has no `viewBox` and is therefore
    /// a miss rather than a bad picture.
    pub fn read(&self, key: &str) -> Option<DiagramImage> {
        let svg = fs::read_to_string(self.path(key)).ok()?;
        let (width, height) = view_box(&svg)?;
        Some(DiagramImage {
            bytes: svg.into_bytes(),
            width,
            height,
        })
    }

    /// Write one entry down. A failure is logged and forgotten — the picture is already rendered.
    ///
    /// The write goes to a sibling temp file and is renamed over, so a crash or a second window
    /// writing the same key never leaves half a picture for the next run to read back as a whole
    /// one.
    pub fn write(&self, key: &str, image: &DiagramImage) {
        if let Err(error) = self.write_atomic(key, &image.bytes) {
            tracing::debug!("the diagram cache did not keep {key}: {error}");
        }
    }

    fn write_atomic(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        fs::create_dir_all(&self.dir)?;
        let temp = self.dir.join(format!(
            ".{key}.{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&temp, bytes)?;
        match fs::rename(&temp, self.path(key)) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temp);
                Err(error)
            }
        }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.svg"))
    }
}

/// One diagram, from the disk tier if it is there and from the renderer if it is not.
///
/// **This is the whole of what runs on the background executor**, and it is the one call site for
/// both the panel and a Markdown fence: one renderer, one cache, two places that ask. A project
/// with no workarea yet — the frames before the catalogue has arrived — renders without a disk
/// tier rather than not rendering.
pub fn resolve(source: &str, palette: DiagramPalette, dir: Option<PathBuf>) -> DiagramAnswer {
    let key = key(source, palette);
    let disk = dir.map(Disk::new);

    if let Some(image) = disk.as_ref().and_then(|disk| disk.read(&key)) {
        return DiagramAnswer {
            key,
            result: Ok(image),
        };
    }

    let result = render(source, palette);
    // Only a picture is written down. A failure is re-tried next time it is asked for.
    if let (Some(disk), Ok(image)) = (disk.as_ref(), result.as_ref()) {
        disk.write(&key, image);
    }
    DiagramAnswer { key, result }
}

/// What comes back from one background render: the key it was filed under, and the picture or the
/// sentence that says why there is none.
///
/// The key travels with the answer because the window's memory tier is keyed by it and the frame
/// that asked is long gone by the time this lands.
pub struct DiagramAnswer {
    pub key: String,
    pub result: Result<DiagramImage, String>,
}
