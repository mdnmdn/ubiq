//! How a picture sits in its panel: a zoom and a pan the user can change.
//!
//! A Mermaid diagram and an Excalidraw scene both draw into a rectangle they do not own. The
//! camera that maps their own coordinates onto that rectangle is the same for both, which is why
//! it lives here rather than in either viewer: the viewers paint, this module answers where.
//!
//! **Fitted is the default.** A picture the user has not touched is scaled uniformly to the panel
//! with a margin, centred, never stretched. The first wheel or drag pins that fit as an explicit
//! zoom and pan so the picture does not jump, and from then on both are numbers the user owns.
//! Double-clicking restores the fit.

/// How far in or out a picture may be taken. The floor is high enough that a large scene is still
/// a picture rather than a spec; the ceiling is high enough to read a label.
pub const ZOOM_MIN: f32 = 0.05;
pub const ZOOM_MAX: f32 = 16.0;

/// The gap between a fitted picture and the edge of its panel.
pub const MARGIN: f32 = 24.0;

/// The picture's own rectangle, in its own coordinates.
///
/// For a Mermaid diagram that is `(0, 0, width, height)` from the SVG's viewBox. For a scene it is
/// the unpadded union of the live elements, which is what [`super::scene::Scene::bounds`] already
/// is — the camera adds the margin, so the parser must not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Content {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

impl Content {
    pub fn from_size(width: f32, height: f32) -> Self {
        Self {
            min_x: 0.0,
            min_y: 0.0,
            width,
            height,
        }
    }
}

/// One mapping from content coordinates onto a panel.
///
/// `scale` is content-units-per-pixel inverted: a content point `(x, y)` lands at
/// `origin + (offset_x + x * scale, offset_y + y * scale)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

/// The user's camera on one picture, plus the last panel it was drawn in.
///
/// The panel size is recorded while the frame is built so a wheel or a drag — which arrives with a
/// window coordinate and nothing else — can keep the point under the pointer still. It is not
/// state the user set; it is what the last layout happened to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// `None` is fitted to the panel. `Some` is a scale the user chose, or the fit pinned by the
    /// first interaction.
    pub zoom: Option<f32>,
    pub pan_x: f32,
    pub pan_y: f32,
    pub panel_w: f32,
    pub panel_h: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub content: Content,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: None,
            pan_x: 0.0,
            pan_y: 0.0,
            panel_w: 0.0,
            panel_h: 0.0,
            origin_x: 0.0,
            origin_y: 0.0,
            content: Content::from_size(1.0, 1.0),
        }
    }
}

impl Viewport {
    /// The uniform scale that puts `content` inside the panel with [`MARGIN`] around it.
    pub fn fit_scale(content: Content, panel_w: f32, panel_h: f32) -> f32 {
        let room_w = (panel_w - MARGIN * 2.0).max(1.0);
        let room_h = (panel_h - MARGIN * 2.0).max(1.0);
        let cw = content.width.max(1.0);
        let ch = content.height.max(1.0);
        (room_w / cw).min(room_h / ch).clamp(ZOOM_MIN, ZOOM_MAX)
    }

    /// The offset that centres a picture of this scale in the panel.
    pub fn fitted_offset(
        content: Content,
        scale: f32,
        panel_w: f32,
        panel_h: f32,
    ) -> (f32, f32) {
        (
            (panel_w - content.width * scale) / 2.0 - content.min_x * scale,
            (panel_h - content.height * scale) / 2.0 - content.min_y * scale,
        )
    }

    /// Whether the panel has been measured. A viewer that draws before the first layout uses a
    /// 1:1 camera rather than dividing by an empty rectangle.
    pub fn measured(&self) -> bool {
        self.panel_w >= 8.0 && self.panel_h >= 8.0
    }

    /// The camera that draws `content` into a panel of this size, honouring the user's zoom and
    /// pan when they have one.
    pub fn camera(&self, content: Content, panel_w: f32, panel_h: f32) -> Camera {
        if panel_w < 8.0 || panel_h < 8.0 {
            return Camera {
                scale: 1.0,
                offset_x: -content.min_x,
                offset_y: -content.min_y,
            };
        }
        let fit = Self::fit_scale(content, panel_w, panel_h);
        let scale = self.zoom.unwrap_or(fit);
        let (offset_x, offset_y) = if self.zoom.is_none() {
            Self::fitted_offset(content, scale, panel_w, panel_h)
        } else {
            (self.pan_x, self.pan_y)
        };
        Camera {
            scale,
            offset_x,
            offset_y,
        }
    }

    /// Remember the panel this picture was just laid out in. Returns whether it went from
    /// unmeasured to measured, which is the one change that owes the window another frame.
    pub fn set_panel(&mut self, width: f32, height: f32, origin_x: f32, origin_y: f32) -> bool {
        let first = !self.measured() && width >= 8.0 && height >= 8.0;
        self.panel_w = width;
        self.panel_h = height;
        self.origin_x = origin_x;
        self.origin_y = origin_y;
        first
    }

    /// Remember the picture's own rectangle, so a wheel or a drag can pin the fit without the
    /// handler having to carry it.
    pub fn set_content(&mut self, content: Content) {
        self.content = content;
    }

    /// Pin the current fit as an explicit zoom and pan, so a following change does not jump.
    fn pin_fit(&mut self) {
        if self.zoom.is_some() {
            return;
        }
        let scale = Self::fit_scale(self.content, self.panel_w, self.panel_h);
        let (x, y) = Self::fitted_offset(self.content, scale, self.panel_w, self.panel_h);
        self.zoom = Some(scale);
        self.pan_x = x;
        self.pan_y = y;
    }

    /// Zoom by `factor` about a window-space point, keeping that point on the same content.
    pub fn zoom_at(&mut self, factor: f32, cursor_x: f32, cursor_y: f32) {
        self.pin_fit();
        let old = self.zoom.unwrap_or(1.0);
        let new = (old * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new - old).abs() < f32::EPSILON {
            return;
        }
        let local_x = cursor_x - self.origin_x;
        let local_y = cursor_y - self.origin_y;
        self.pan_x = local_x - (local_x - self.pan_x) * (new / old);
        self.pan_y = local_y - (local_y - self.pan_y) * (new / old);
        self.zoom = Some(new);
    }

    /// Shift the picture by a window-space delta.
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        self.pin_fit();
        self.pan_x += dx;
        self.pan_y += dy;
    }

    /// Back to the fitted camera.
    pub fn reset(&mut self) {
        self.zoom = None;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
    }
}
