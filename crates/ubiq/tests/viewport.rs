//! The camera a picture sits under: fit, zoom about a point, pan, reset.
//!
//! Nothing here draws. The viewers apply [`ubiq::state::viewport::Camera`]; this asserts the
//! numbers they are handed.

use ubiq::state::viewport::{Content, Viewport, ZOOM_MAX, ZOOM_MIN};

fn content(w: f32, h: f32) -> Content {
    Content::from_size(w, h)
}

fn measured(content: Content, panel_w: f32, panel_h: f32) -> Viewport {
    let mut view = Viewport::default();
    view.set_content(content);
    view.set_panel(panel_w, panel_h, 0.0, 0.0);
    view
}

/// A picture smaller than its panel is scaled up to fill it, with the margin around it, and sits
/// in the middle rather than in a corner.
#[test]
fn a_fitted_picture_is_centred_and_uniform() {
    let content = content(100.0, 50.0);
    let view = measured(content, 248.0, 148.0);
    let camera = view.camera(content, 248.0, 148.0);

    // Room is 200×100 after the 24px margin each side; the limiting axis is height: 100/50 = 2.
    assert!((camera.scale - 2.0).abs() < 1e-5, "{camera:?}");
    // Width used is 200, so 24px of margin on the left; content origin is 0.
    assert!((camera.offset_x - 24.0).abs() < 1e-5, "{camera:?}");
    assert!((camera.offset_y - 24.0).abs() < 1e-5, "{camera:?}");
}

/// A picture whose origin is not zero is still centred: the offset cancels `min_x` / `min_y` so
/// the visible box, not the coordinate origin, sits in the middle of the panel.
#[test]
fn a_scene_that_does_not_start_at_the_origin_still_fits() {
    let content = Content {
        min_x: 100.0,
        min_y: -20.0,
        width: 200.0,
        height: 100.0,
    };
    let view = measured(content, 248.0, 148.0);
    let camera = view.camera(content, 248.0, 148.0);

    // Same scale as a 200×100 picture in a 248×148 panel.
    assert!((camera.scale - 1.0).abs() < 1e-5, "{camera:?}");
    let (x, y) = Viewport::fitted_offset(content, camera.scale, 248.0, 148.0);
    assert!((camera.offset_x - x).abs() < 1e-5);
    assert!((camera.offset_y - y).abs() < 1e-5);
    // The content's left edge (x=100) lands at margin.
    let left = camera.offset_x + content.min_x * camera.scale;
    assert!((left - 24.0).abs() < 1e-5, "left edge at {left}");
}

/// An unmeasured panel does not invent a scale: 1:1, origin shifted to the content's own.
#[test]
fn an_unmeasured_panel_draws_one_to_one() {
    let content = Content {
        min_x: 40.0,
        min_y: 10.0,
        width: 80.0,
        height: 20.0,
    };
    let camera = Viewport::default().camera(content, 0.0, 0.0);
    assert_eq!(camera.scale, 1.0);
    assert_eq!(camera.offset_x, -40.0);
    assert_eq!(camera.offset_y, -10.0);
}

/// Zooming about a point keeps that point on the same content: the content coordinate under the
/// cursor before the zoom is the content coordinate under it after.
#[test]
fn a_zoom_keeps_the_point_under_the_cursor() {
    let content = content(200.0, 100.0);
    let mut view = measured(content, 400.0, 300.0);
    let before = view.camera(content, 400.0, 300.0);

    // A window-space point in the middle of the panel.
    let cursor = (200.0, 150.0);
    let content_x = (cursor.0 - before.offset_x) / before.scale;
    let content_y = (cursor.1 - before.offset_y) / before.scale;

    view.zoom_at(2.0, cursor.0, cursor.1);
    let after = view.camera(content, 400.0, 300.0);

    assert!((after.scale - before.scale * 2.0).abs() < 1e-4, "{after:?}");
    let again_x = (cursor.0 - after.offset_x) / after.scale;
    let again_y = (cursor.1 - after.offset_y) / after.scale;
    assert!(
        (again_x - content_x).abs() < 1e-3,
        "{again_x} vs {content_x}"
    );
    assert!(
        (again_y - content_y).abs() < 1e-3,
        "{again_y} vs {content_y}"
    );
}

/// A pan is a window-space shift, and it pins the fit first so the picture does not jump to the
/// origin the moment the pointer moves.
#[test]
fn a_pan_pins_the_fit_then_shifts() {
    let content = content(100.0, 100.0);
    let mut view = measured(content, 248.0, 248.0);
    let fitted = view.camera(content, 248.0, 248.0);

    view.pan_by(10.0, -4.0);
    let after = view.camera(content, 248.0, 248.0);

    assert!((after.scale - fitted.scale).abs() < 1e-5);
    assert!((after.offset_x - fitted.offset_x - 10.0).abs() < 1e-5);
    assert!((after.offset_y - fitted.offset_y + 4.0).abs() < 1e-5);
    assert!(view.zoom.is_some());
}

/// Reset forgets the user's camera and not the panel it was measured against.
#[test]
fn reset_returns_to_the_fit_and_keeps_the_panel() {
    let content = content(100.0, 50.0);
    let mut view = measured(content, 248.0, 148.0);
    view.zoom_at(3.0, 100.0, 80.0);
    view.pan_by(20.0, 20.0);
    view.reset();

    assert!(view.zoom.is_none());
    assert_eq!(view.pan_x, 0.0);
    assert_eq!(view.pan_y, 0.0);
    assert_eq!(view.panel_w, 248.0);
    let camera = view.camera(content, 248.0, 148.0);
    let fresh = measured(content, 248.0, 148.0).camera(content, 248.0, 148.0);
    assert_eq!(camera, fresh);
}

/// The scale never leaves the range, however large the factor.
#[test]
fn zoom_is_clamped() {
    let mut view = measured(content(10.0, 10.0), 200.0, 200.0);
    view.zoom_at(1_000.0, 100.0, 100.0);
    assert!((view.zoom.unwrap() - ZOOM_MAX).abs() < 1e-5);

    view.zoom_at(0.0001, 100.0, 100.0);
    assert!((view.zoom.unwrap() - ZOOM_MIN).abs() < 1e-5);
}

/// Going from an empty panel to a real one is the change that owes another frame; a later resize
/// of a panel that was already measured is not, because the resize itself already asked for one.
#[test]
fn the_first_measurement_is_the_one_that_counts() {
    let mut view = Viewport::default();
    assert!(!view.set_panel(0.0, 0.0, 0.0, 0.0));
    assert!(view.set_panel(400.0, 300.0, 12.0, 48.0));
    assert!(!view.set_panel(410.0, 300.0, 12.0, 48.0));
}
