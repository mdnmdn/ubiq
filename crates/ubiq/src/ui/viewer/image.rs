//! The image itself.
//!
//! The bytes are the file's own and are handed to the platform undecoded, which is the same
//! discipline the diagram viewer follows: what draws a picture is a decoder, and turning the bytes
//! into anything else on the way would be lossy in exactly the way that matters.

use std::sync::Arc;

use gpui::{
    AnyElement, Image, ImageFormat, ImageSource, IntoElement, ParentElement, Styled, div, img, px,
    relative,
};

use crate::theme;

/// The format a path's extension names.
///
/// Only the formats the platform decodes are named, and they are the formats
/// [`crate::state::editor::ViewerKind`] sends here. An extension with nothing behind it is not
/// guessed at.
fn format(path: &str) -> Option<ImageFormat> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    match ext.to_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "gif" => Some(ImageFormat::Gif),
        "webp" => Some(ImageFormat::Webp),
        "svg" => Some(ImageFormat::Svg),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "ico" => Some(ImageFormat::Ico),
        _ => None,
    }
}

/// The picture, centred in the panel and never drawn larger than it.
///
/// An extension with no format behind it says so rather than drawing nothing: an empty panel is
/// indistinguishable from a file that never arrived, and the two want different things done.
pub fn render(bytes: &[u8], path: &str) -> AnyElement {
    let Some(format) = format(path) else {
        return super::note(
            "Not an image this build decodes",
            crate::theme::text_faint(),
        );
    };

    let image = Arc::new(Image::from_bytes(format, bytes.to_vec()));
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .p_3()
        .bg(theme::app_bg())
        .child(
            img(ImageSource::Image(image))
                .max_w(relative(1.))
                .max_h(relative(1.)),
        )
        .into_any_element()
}
