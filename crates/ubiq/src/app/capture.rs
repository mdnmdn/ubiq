//! The window-capture control: photograph this window into a dirty untitled tab.
//!
//! The route is the operating system's, through gpui's screen-capture API — there is no
//! render-to-image in the interface crate, so there is no other one. A source is a display,
//! never a window (`zed-scap` filters window targets out), so capturing "the window" is a
//! crop of a display frame to the window's bounds, at the window's scale.
//!
//! The frame behind `ScreenCaptureFrame` is `scap`'s on Windows and Linux and a
//! `CVImageBuffer` on macOS; both are decoded here, so the control is offered on all three.
//! The frame is turned into RGBA on the capturer's own thread, because a `CVImageBuffer` is
//! not `Send` and RGBA bytes are. The first press asks the platform nothing beyond
//! `screen_capture_sources()`: where the platform has a prompt (macOS, Wayland) that call
//! is the prompt, and where it has none (Windows, X11) the button simply works.
//!
//! Nothing crosses the bus. The pixels land in `open_untitled_image`, the tab the clipboard
//! already shares, and are saved by the untitled path that exists.

use std::rc::Rc;

use super::*;

impl AppState {
    /// Whether the capture control is offered in this window: the setting, the platform,
    /// and the runtime backend. Off removes the button, the tooltip and the keystroke
    /// together — the handler below is a no-op where this is false.
    pub fn capture_offered(&self, cx: &App) -> bool {
        // The platforms whose frame this build decodes. Elsewhere — Wayland, the web —
        // offering the button would be offering a capture that cannot land.
        cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        )) && self.workbench.settings.ui.capture_enabled
            && cx.is_screen_capture_supported()
    }

    /// Photograph this window into a dirty untitled tab.
    ///
    /// The capture runs off the frame: one display frame is streamed, cropped to the
    /// window, and opened as `capture-{n}.png`. A window with nothing on its display is
    /// refused rather than saved as whatever the compositor had.
    pub fn capture_window(
        &mut self,
        _: &CaptureWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.capture_offered(cx) || self.project(cx).is_none() {
            return;
        }
        let bounds = window.bounds();
        let scale = window.scale_factor();
        let display = window
            .display(cx)
            .map(|display| (display.id(), display.bounds()));
        let sources = cx.screen_capture_sources();
        let exec = cx.foreground_executor().clone();
        cx.spawn(async move |this, cx| {
            // The receiver's type is the platform's to name, so the awaiting stays here
            // where it is inferred rather than in a helper that would have to spell it.
            let sources = match sources.await {
                Ok(Ok(sources)) => sources,
                Ok(Err(error)) => {
                    tracing::warn!("window capture failed: {error:#}");
                    return;
                }
                Err(_) => return,
            };
            let png = match decode_window_png(&sources, bounds, scale, display, &exec).await {
                Ok(png) => png,
                Err(error) => {
                    tracing::warn!("window capture failed: {error}");
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| this.open_untitled_image(png, cx));
        })
        .detach();
    }
}

/// The platform half: first frame off the window's display, cropped and encoded.
async fn decode_window_png(
    sources: &[Rc<dyn gpui::ScreenCaptureSource>],
    bounds: gpui::Bounds<gpui::Pixels>,
    scale: f32,
    display: Option<(gpui::DisplayId, gpui::Bounds<gpui::Pixels>)>,
    exec: &gpui::ForegroundExecutor,
) -> anyhow::Result<Vec<u8>> {
    let source = pick_display(sources, display.map(|(id, _)| id))
        .ok_or_else(|| anyhow::anyhow!("the platform offered no display to capture"))?;
    let (frame_tx, frame_rx) = flume::bounded(1);
    let stream = match source
        .stream(
            exec,
            Box::new(move |frame| {
                // Converted here, on the capturer's own thread: a macOS frame is a
                // `CVImageBuffer`, which is not `Send`, and RGBA bytes are.
                frame_tx.send(frame_rgba(&frame.0)).ok();
            }),
        )
        .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => return Err(anyhow::anyhow!("capture stream failed: {error:#}")),
        Err(_) => return Err(anyhow::anyhow!("capture stream never started")),
    };
    // The first frame ends the stream: a screenshot is start, take one, stop. Dropping
    // the stream is what stops the capturer thread.
    let frame = frame_rx
        .recv_async()
        .await
        .map_err(|_| anyhow::anyhow!("capture ended before its first frame"))?;
    drop(stream);
    let (rgba, width, height) = frame
        .ok_or_else(|| anyhow::anyhow!("the platform sent a frame this build does not read"))?;
    let (x, y, w, h) = crop_rect(
        width,
        height,
        bounds,
        display.map(|(_, bounds)| bounds),
        scale,
    )
    .ok_or_else(|| anyhow::anyhow!("the window is nowhere on its display"))?;
    crop_png(&rgba, width, height, x, y, w, h)
        .ok_or_else(|| anyhow::anyhow!("the cropped frame did not encode"))
}

/// One platform frame as RGBA, or `None` for a frame this build does not read. The type
/// behind `ScreenCaptureFrame` is gpui-private, so each platform names its own — the same
/// crate versions gpui resolves, or these would be two types that never unify.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn frame_rgba(frame: &zed_scap::frame::Frame) -> Option<(Vec<u8>, u32, u32)> {
    scap_rgba(frame)
}

/// A `CVImageBuffer` as RGBA. ScreenCaptureKit is configured for `BGRA` by gpui, so that
/// is the one format read; a planar or short buffer is refused rather than read wrong.
#[cfg(target_os = "macos")]
fn frame_rgba(frame: &core_video::image_buffer::CVImageBuffer) -> Option<(Vec<u8>, u32, u32)> {
    use core_video::pixel_buffer::{
        CVPixelBuffer, kCVPixelBufferLock_ReadOnly, kCVPixelFormatType_32BGRA,
    };

    let pixels: CVPixelBuffer = frame.downcast()?;
    if pixels.get_pixel_format() != kCVPixelFormatType_32BGRA || pixels.is_planar() {
        return None;
    }
    let (w, h) = (pixels.get_width(), pixels.get_height());
    let stride = pixels.get_bytes_per_row();
    if w == 0 || h == 0 || stride < w * 4 {
        return None;
    }
    if pixels.lock_base_address(kCVPixelBufferLock_ReadOnly) != 0 {
        return None;
    }
    let base = unsafe { pixels.get_base_address() } as *const u8;
    let rgba = (!base.is_null()).then(|| {
        // Held only until the copy below is done, and unlocked immediately after.
        let bytes = unsafe { std::slice::from_raw_parts(base, stride * h) };
        packed_rgba_rows(bytes, stride, w as u32, h as u32, [2, 1, 0], Some(3))
    });
    pixels.unlock_base_address(kCVPixelBufferLock_ReadOnly);
    rgba?.map(|rgba| (rgba, w as u32, h as u32))
}

/// The window's display, by id — or the first source where the window names no display.
/// A source id that matches nothing still captures: the crop clamps to the frame, so a
/// wrong display reads as a cropped picture rather than a failure.
fn pick_display(
    sources: &[Rc<dyn gpui::ScreenCaptureSource>],
    display: Option<gpui::DisplayId>,
) -> Option<Rc<dyn gpui::ScreenCaptureSource>> {
    match display {
        Some(id) => sources
            .iter()
            .find(|source| source.metadata().is_ok_and(|meta| meta.id == u64::from(id)))
            .or_else(|| sources.first())
            .cloned(),
        None => sources.first().cloned(),
    }
}

/// The window's rectangle in frame pixels: global logical bounds, rebased onto the
/// display's origin and scaled to device pixels, clamped to the frame. `None` is a
/// window with nothing on the display — minimised, or dragged wholly off it — which is
/// refused rather than saved as whatever the compositor had.
fn crop_rect(
    frame_w: u32,
    frame_h: u32,
    window: gpui::Bounds<gpui::Pixels>,
    display: Option<gpui::Bounds<gpui::Pixels>>,
    scale: f32,
) -> Option<(u32, u32, u32, u32)> {
    let origin = display
        .map(|bounds| bounds.origin)
        .unwrap_or(gpui::point(gpui::px(0.), gpui::px(0.)));
    let x = ((f32::from(window.origin.x) - f32::from(origin.x)) * scale)
        .round()
        .max(0.) as u32;
    let y = ((f32::from(window.origin.y) - f32::from(origin.y)) * scale)
        .round()
        .max(0.) as u32;
    let w = (f32::from(window.size.width) * scale).round().max(0.) as u32;
    let h = (f32::from(window.size.height) * scale).round().max(0.) as u32;
    let x = x.min(frame_w);
    let y = y.min(frame_h);
    let (w, h) = (w.min(frame_w - x), h.min(frame_h - y));
    if w == 0 || h == 0 {
        None
    } else {
        Some((x, y, w, h))
    }
}

/// A `scap` frame as RGBA. YUV is what the capturer asks for, packed BGR variants what a
/// backend may send anyway; a short buffer in any of them is refused rather than read
/// past its end.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn scap_rgba(frame: &zed_scap::frame::Frame) -> Option<(Vec<u8>, u32, u32)> {
    use zed_scap::frame::Frame;
    match frame {
        Frame::YUVFrame(frame) => {
            let (w, h) = (frame.width.max(0) as u32, frame.height.max(0) as u32);
            if w == 0 || h == 0 {
                return None;
            }
            Some((
                nv12_to_rgba(
                    &frame.luminance_bytes,
                    frame.luminance_stride.max(0) as u32,
                    &frame.chrominance_bytes,
                    frame.chrominance_stride.max(0) as u32,
                    w,
                    h,
                )?,
                w,
                h,
            ))
        }
        Frame::RGB(frame) => packed_rgba(&frame.data, frame.width, frame.height, [0, 1, 2], None),
        Frame::RGBx(frame) => {
            packed_rgba(&frame.data, frame.width, frame.height, [0, 1, 2], Some(3))
        }
        Frame::XBGR(frame) => {
            packed_rgba(&frame.data, frame.width, frame.height, [3, 2, 1], Some(0))
        }
        Frame::BGRx(frame) => {
            packed_rgba(&frame.data, frame.width, frame.height, [2, 1, 0], Some(3))
        }
        Frame::BGR0(frame) => {
            packed_rgba(&frame.data, frame.width, frame.height, [2, 1, 0], Some(3))
        }
        // Already alpha-ordered bytes; the reorder below is the identity for it, kept so
        // every packed variant reads through one code path.
        Frame::BGRA(frame) => {
            packed_rgba(&frame.data, frame.width, frame.height, [2, 1, 0], Some(3))
        }
    }
}

/// One packed pixel in, one RGBA pixel out, over tightly packed rows. `order` names which
/// source byte feeds R, G, B; `alpha` which feeds A, or `None` for opaque.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn packed_rgba(
    data: &[u8],
    width: i32,
    height: i32,
    order: [usize; 3],
    alpha: Option<usize>,
) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h) = (width.max(0) as u32, height.max(0) as u32);
    let px = if alpha.is_some() { 4 } else { 3 };
    let rgba = packed_rgba_rows(data, w as usize * px, w, h, order, alpha)?;
    Some((rgba, w, h))
}

/// The same reorder, honouring a row stride — a padded row reads correctly, and a short
/// buffer is refused rather than read past its end.
fn packed_rgba_rows(
    data: &[u8],
    stride: usize,
    w: u32,
    h: u32,
    order: [usize; 3],
    alpha: Option<usize>,
) -> Option<Vec<u8>> {
    let px = if alpha.is_some() { 4 } else { 3 };
    let (cols, rows) = (w as usize, h as usize);
    if cols == 0 || rows == 0 || stride < cols * px || data.len() < stride * rows {
        return None;
    }
    let mut rgba = Vec::with_capacity(cols * rows * 4);
    for row in 0..rows {
        for col in 0..cols {
            let at = row * stride + col * px;
            let pixel = &data[at..at + px];
            rgba.extend_from_slice(&[
                pixel[order[0]],
                pixel[order[1]],
                pixel[order[2]],
                alpha.map(|a| pixel[a]).unwrap_or(0xFF),
            ]);
        }
    }
    Some(rgba)
}

#[cfg(any(test, target_os = "windows", target_os = "linux"))]
/// NV12 (one luma plane, one interleaved chroma plane at half resolution) to RGBA, by
/// BT.601. Strides are honoured, so padded rows read correctly.
fn nv12_to_rgba(
    y: &[u8],
    y_stride: u32,
    uv: &[u8],
    uv_stride: u32,
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || y_stride < w || uv_stride < w {
        return None;
    }
    let y_stride = y_stride as usize;
    let uv_stride = uv_stride as usize;
    let w = w as usize;
    let h = h as usize;
    if y.len() < y_stride * h || uv.len() < uv_stride * h.div_ceil(2) {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        for col in 0..w {
            let luma = y[row * y_stride + col] as i32;
            let uv_at = (row / 2) * uv_stride + (col / 2) * 2;
            let u = uv[uv_at] as i32 - 128;
            let v = uv[uv_at + 1] as i32 - 128;
            // BT.601, integerised: r = y + 1.402v, g = y − 0.344u − 0.714v,
            // b = y + 1.772u.
            let c = luma - 16;
            let r = (298 * c + 409 * v + 128) >> 8;
            let g = (298 * c - 100 * u - 208 * v + 128) >> 8;
            let b = (298 * c + 516 * u + 128) >> 8;
            rgba.extend_from_slice(&[
                r.clamp(0, 255) as u8,
                g.clamp(0, 255) as u8,
                b.clamp(0, 255) as u8,
                0xFF,
            ]);
        }
    }
    Some(rgba)
}

/// The cropped rectangle, PNG-encoded. `None` is an encode that failed, not pixels
/// worth saving half of.
fn crop_png(
    rgba: &[u8],
    frame_w: u32,
    frame_h: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Option<Vec<u8>> {
    use image::{ImageFormat, RgbaImage};
    let frame = RgbaImage::from_raw(frame_w, frame_h, rgba.to_vec())?;
    if x + w > frame_w || y + h > frame_h || w == 0 || h == 0 {
        return None;
    }
    let cropped = image::imageops::crop_imm(&frame, x, y, w, h).to_image();
    let mut png = Vec::new();
    cropped
        .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
        .ok()?;
    Some(png)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window sits mid-display at 2x: its logical rect doubles into frame pixels.
    #[test]
    fn crop_scales_and_rebases() {
        let rect = crop_rect(
            3000,
            2000,
            gpui::Bounds {
                origin: gpui::point(gpui::px(100.), gpui::px(50.)),
                size: gpui::size(gpui::px(400.), gpui::px(300.)),
            },
            Some(gpui::Bounds {
                origin: gpui::point(gpui::px(0.), gpui::px(0.)),
                size: gpui::size(gpui::px(1500.), gpui::px(1000.)),
            }),
            2.0,
        );
        assert_eq!(rect, Some((200, 100, 800, 600)));
    }

    /// A window hanging off the display's edge is clamped to the frame, not refused.
    #[test]
    fn crop_clamps_to_the_frame() {
        let rect = crop_rect(
            800,
            600,
            gpui::Bounds {
                origin: gpui::point(gpui::px(700.), gpui::px(500.)),
                size: gpui::size(gpui::px(400.), gpui::px(300.)),
            },
            None,
            1.0,
        );
        assert_eq!(rect, Some((700, 500, 100, 100)));
    }

    /// A window wholly off the display has nothing on it: refused, not saved as black.
    #[test]
    fn crop_refuses_a_window_off_its_display() {
        let rect = crop_rect(
            800,
            600,
            gpui::Bounds {
                origin: gpui::point(gpui::px(900.), gpui::px(0.)),
                size: gpui::size(gpui::px(400.), gpui::px(300.)),
            },
            None,
            1.0,
        );
        assert_eq!(rect, None);
    }

    /// Flat grey in, flat grey out: luma 235 with neutral chroma is near-white.
    #[test]
    fn nv12_neutral_chroma_is_grey() {
        let (w, h) = (4, 2);
        let rgba = nv12_to_rgba(&[235; 8], 4, &[128; 8], 4, w, h).unwrap();
        assert_eq!(rgba.len(), 32);
        // 298 * (235 − 16) / 256 ≈ 255.
        assert!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .all(|px| px[0] > 250 && px[3] == 0xFF)
        );
    }

    /// Short planes are refused rather than read past their end.
    #[test]
    fn nv12_refuses_short_planes() {
        assert_eq!(nv12_to_rgba(&[0; 4], 4, &[128; 8], 4, 4, 2), None);
    }

    /// A padded row reads by its stride, not by its width: the pad is never a pixel.
    #[test]
    fn packed_rows_honour_the_stride() {
        // Two 1-pixel BGRA rows in an 8-byte stride: four bytes of pixel, four of pad.
        let data = [10, 20, 30, 40, 0, 0, 0, 0, 50, 60, 70, 80, 0, 0, 0, 0];
        let rgba = packed_rgba_rows(&data, 8, 1, 2, [2, 1, 0], Some(3)).unwrap();
        assert_eq!(rgba, vec![30, 20, 10, 40, 70, 60, 50, 80]);
    }

    /// A buffer shorter than its stride says so rather than reading past its end.
    #[test]
    fn packed_rows_refuse_a_short_buffer() {
        assert_eq!(packed_rgba_rows(&[0; 8], 8, 1, 2, [2, 1, 0], Some(3)), None);
        assert_eq!(packed_rgba_rows(&[0; 8], 2, 1, 2, [2, 1, 0], Some(3)), None);
    }

    /// A 1x1 crop encodes to a PNG that decodes back to the same pixel.
    #[test]
    fn crop_png_round_trips() {
        let png = crop_png(&[10, 20, 30, 255], 1, 1, 0, 0, 1, 1).unwrap();
        let back = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!((back.width(), back.height()), (1, 1));
        assert_eq!(back.into_raw(), vec![10, 20, 30, 255]);
    }
}
