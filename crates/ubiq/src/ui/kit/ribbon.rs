//! A diagonal band across one corner of its parent, carrying a word.
//!
//! Drawn as an SVG picture rather than styled markup because GPUI rotates images and not boxes,
//! and a ribbon is a rotation. `Image::from_bytes` identifies a picture by the hash of its bytes,
//! so rebuilding the same markup every frame hits the window's image cache rather than the
//! renderer.
//!
//! The parent needs `.relative()` (or to be the window) so the absolute box lands on the corner
//! it was asked for. A click, when there is one, is taken only on the band — the box around the
//! diagonal takes none.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, Image, ImageFormat, ImageSource, InteractiveElement,
    IntoElement, ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window,
    div, img, px,
};
use std::sync::Arc;

/// Which corner of the parent the band crosses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RibbonCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// How large the corner box is drawn when the caller does not say. The band lies across its
/// diagonal.
pub const RIBBON_SIZE: f32 = 96.0;

/// How much of the corner box the band crosses — the click target, as a fraction of the size.
const BAND: f32 = 0.42;

type Click = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A ribbon: a word, a corner, a band colour and the ink the word is written in.
pub struct Ribbon {
    word: SharedString,
    corner: RibbonCorner,
    band: Rgba,
    ink: Rgba,
    size: f32,
    on_click: Option<(ElementId, Click)>,
}

/// Pin a word across `corner` of the parent, in `band` with `ink` for the letters.
pub fn ribbon(
    word: impl Into<SharedString>,
    corner: RibbonCorner,
    band: Rgba,
    ink: Rgba,
) -> Ribbon {
    Ribbon {
        word: word.into(),
        corner,
        band,
        ink,
        size: RIBBON_SIZE,
        on_click: None,
    }
}

impl Ribbon {
    /// How large the corner box is. The band scales with it.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// The band takes a click. `id` is the hit target's; the box around the diagonal does not
    /// receive it.
    pub fn on_click(
        mut self,
        id: impl Into<ElementId>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some((id.into(), Rc::new(on_click)));
        self
    }
}

impl IntoElement for Ribbon {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let img = img(ImageSource::Image(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            markup(&self.word, self.corner, self.band, self.ink).into_bytes(),
        ))))
        .size(px(self.size));

        let mut root = pin(div().absolute().size(px(self.size)), self.corner).child(img);

        if let Some((id, on_click)) = self.on_click {
            root = root.child(
                pin(
                    div()
                        .id(id)
                        .absolute()
                        .size(px(self.size * BAND))
                        .cursor_pointer(),
                    self.corner,
                )
                .on_click(move |ev, window, cx| on_click(ev, window, cx)),
            );
        }

        root.into_any_element()
    }
}

fn pin<T: Styled>(el: T, corner: RibbonCorner) -> T {
    match corner {
        RibbonCorner::TopLeft => el.top_0().left_0(),
        RibbonCorner::TopRight => el.top_0().right_0(),
        RibbonCorner::BottomLeft => el.bottom_0().left_0(),
        RibbonCorner::BottomRight => el.bottom_0().right_0(),
    }
}

/// The picture, in a 100×100 box whose chosen corner the band crosses.
///
/// The band is a strip along that corner's diagonal; the word sits on the midline and turns
/// with it.
fn markup(word: &str, corner: RibbonCorner, band: Rgba, ink: Rgba) -> String {
    let (points, x, y, rotate) = geometry(corner);
    let font = font_size(word);
    let tracking = if word.len() <= 6 { 1.0 } else { 0.4 };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
<polygon points="{points}" fill="{band}"/>
<text x="{x}" y="{y}" transform="rotate({rotate} {x} {y})" fill="{ink}"
 font-family="sans-serif" font-size="{font}" font-weight="700" letter-spacing="{tracking}"
 text-anchor="middle" dominant-baseline="central">{word}</text>
</svg>"##,
        band = hex(band),
        ink = hex(ink),
    )
}

fn geometry(corner: RibbonCorner) -> (&'static str, f32, f32, f32) {
    match corner {
        RibbonCorner::BottomLeft => ("0,45 55,100 28,100 0,72", 20.75, 79.25, 45.0),
        RibbonCorner::TopLeft => ("0,28 0,55 55,0 28,0", 20.75, 20.75, -45.0),
        RibbonCorner::TopRight => ("45,0 72,0 100,28 100,55", 79.25, 20.75, 45.0),
        RibbonCorner::BottomRight => ("45,100 72,100 100,72 100,45", 79.25, 79.25, -45.0),
    }
}

/// The band is about 55 viewBox units of readable length. Bold sans is ~0.62em per letter.
fn font_size(word: &str) -> f32 {
    (55.0 / (word.len().max(1) as f32 * 0.62)).clamp(6.5, 11.0)
}

/// A token as SVG writes colours. Alpha is dropped: ribbon tokens are opaque.
fn hex(colour: Rgba) -> String {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(colour.r),
        channel(colour.g),
        channel(colour.b)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_writes_six_digits() {
        assert_eq!(
            hex(Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.5,
                a: 1.0
            }),
            "#ff0080"
        );
    }

    #[test]
    fn markup_carries_the_word_the_corner_and_the_colours() {
        let yellow = Rgba {
            r: 1.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        };
        let black = Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let svg = markup("alpha", RibbonCorner::BottomLeft, yellow, black);
        assert!(svg.contains(">alpha</text>"));
        assert!(svg.contains("fill=\"#ffff00\""));
        assert!(svg.contains("fill=\"#000000\""));
        assert!(svg.contains("0,45 55,100 28,100 0,72"));
    }

    #[test]
    fn each_corner_has_its_own_band() {
        let band = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let ink = Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let top_left = markup("experimental", RibbonCorner::TopLeft, band, ink);
        assert!(top_left.contains(">experimental</text>"));
        assert!(top_left.contains("0,28 0,55 55,0 28,0"));
        assert!(top_left.contains("rotate(-45"));

        let top_right = markup("x", RibbonCorner::TopRight, band, ink);
        assert!(top_right.contains("45,0 72,0 100,28 100,55"));
        let bottom_right = markup("x", RibbonCorner::BottomRight, band, ink);
        assert!(bottom_right.contains("45,100 72,100 100,72 100,45"));
    }
}
