//! A drag with an annotation tool picked must draw, not pan.
//!
//! The reported bug: in a capture tab, every tool behaves like Select-on-empty-space — the
//! viewport pans and nothing is drawn. Three links can break that: the tool never lands on the
//! tab (`image_file_mut` finds nothing), the panel is never measured (`to_scene` declines every
//! gesture), or the viewport's own hit layer wins the press before the tool overlay sees it.
//! Each is its own assertion below, so a failure names the link.

use std::ops::Deref as _;

use chrono::Utc;
use gpui::{
    AppContext as _, ClipboardItem, Entity, Image, ImageFormat, Modifiers, MouseButton, Point,
    TestAppContext, VisualTestContext, px,
};
use ubiq::app::{AppState, BusHub, PasteClipboardImage};
use ubiq::state::FileBody;
use ubiq::state::WindowRegistry;
use ubiq::state::image_edit::ImageTool;
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// A flat PNG of one colour, the capture stand-in.
fn png(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    let picture = image::RgbaImage::from_pixel(width, height, image::Rgba(pixel));
    let mut out = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("an in-memory encode succeeds");
    out
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// The number of elements in the tab's scene, and the tool it holds.
fn scene_state(state: &Entity<AppState>, key: &str, cx: &TestAppContext) -> (usize, ImageTool) {
    state.read_with(cx, |state, cx| {
        match state.file(key, cx).map(|file| &file.body) {
            Some(FileBody::ImageEdit(edit)) => (edit.scene.elements.len(), edit.tool),
            _ => panic!("the tab is not an image edit"),
        }
    })
}

/// A window on a project with one untitled capture open, drawn once.
fn capture_window(cx: &mut TestAppContext) -> (Entity<AppState>, gpui::AnyWindowHandle) {
    let snapshot = a_project();
    let project = snapshot.record.id;
    let (hub, _host) = bus::hub();

    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        ubiq::app::install_key_bindings(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    // The capture: the clipboard's paste arm, which is the untitled-image entry point.
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
            ImageFormat::Png,
            png(400, 300, [255, 255, 255, 255]),
        )));
    });
    handle
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.paste_clipboard_image(&PasteClipboardImage, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    (state, *handle.deref())
}

/// The tab key an untitled capture takes.
const KEY: &str = "capture-1.png";

#[gpui::test]
fn a_tool_drag_draws_instead_of_panning(cx: &mut TestAppContext) {
    let (state, window) = capture_window(cx);
    let handle = window;
    let key = KEY;

    // ---- link 1: the tab is a scene, and it opened in Edit -------------------------------
    state.read_with(cx, |state, cx| {
        let file = state.file(key, cx).expect("the capture tab is open");
        assert!(
            matches!(file.body, FileBody::ImageEdit(_)),
            "the capture tab's body is a scene (FileBody::ImageEdit)"
        );
        assert!(
            file.image_editing,
            "a capture opens in Edit, so the tool layer is drawn"
        );
    });

    // ---- link 2: the toolbar's write lands on the tab ------------------------------------
    state.update(cx, |state, cx| {
        state.set_image_tool(key, ImageTool::Rectangle, cx);
    });
    cx.run_until_parked();
    let (before, tool) = scene_state(&state, key, cx);
    assert_eq!(
        tool,
        ImageTool::Rectangle,
        "set_image_tool reached the tab (suspect b: image_file_mut found it)"
    );

    // ---- link 3: the panel was measured, so to_scene will not decline --------------------
    let mut vcx = VisualTestContext::from_window(handle, cx);
    vcx.run_until_parked();
    let viewport = state.read_with(&vcx.cx, |state, _| state.viewport(key));
    assert!(
        viewport.panel_w >= 8.0 && viewport.panel_h >= 8.0,
        "the viewport panel was measured for key {key:?} (suspect a) — got {}x{} at ({}, {})",
        viewport.panel_w,
        viewport.panel_h,
        viewport.origin_x,
        viewport.origin_y
    );

    // ---- the gesture ---------------------------------------------------------------------
    let centre = Point {
        x: px(viewport.origin_x + viewport.panel_w / 2.0),
        y: px(viewport.origin_y + viewport.panel_h / 2.0),
    };
    let start = Point {
        x: centre.x - px(40.),
        y: centre.y - px(30.),
    };
    let end = Point {
        x: centre.x + px(40.),
        y: centre.y + px(30.),
    };

    vcx.simulate_mouse_move(start, None, Modifiers::none());
    vcx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();

    let (after, _) = scene_state(&state, key, &vcx.cx);
    let moved = state.read_with(&vcx.cx, |state, _| state.viewport(key));

    assert_eq!(
        after,
        before + 1,
        "the drag committed a rectangle (before {before}, after {after})"
    );
    assert_eq!(
        (moved.zoom, moved.pan_x, moved.pan_y),
        (viewport.zoom, viewport.pan_x, viewport.pan_y),
        "the viewport did not pan — the tool overlay took the press, not the hit layer \
         (suspect c)"
    );

    // ---- the control: Select on empty space must still pan -------------------------------
    //
    // Without this, a harness that delivers no events at all would pass every assertion above
    // by doing nothing. The pan is the proof the hit layer underneath is reachable.
    state.update(&mut vcx.cx, |state, cx| {
        state.set_image_tool(key, ImageTool::Select, cx);
    });
    vcx.run_until_parked();
    let corner = Point {
        x: px(viewport.origin_x + 4.0),
        y: px(viewport.origin_y + 4.0),
    };
    let dragged = Point {
        x: corner.x + px(25.),
        y: corner.y + px(25.),
    };
    vcx.simulate_mouse_move(corner, None, Modifiers::none());
    vcx.simulate_mouse_down(corner, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(dragged, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(dragged, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();

    let panned = state.read_with(&vcx.cx, |state, _| state.viewport(key));
    assert_ne!(
        (panned.pan_x, panned.pan_y),
        (moved.pan_x, moved.pan_y),
        "Select on empty space reaches the hit layer and pans — the control that proves the \
         harness delivers real events"
    );
}

/// The link the drag test bypasses: the toolbar button itself.
///
/// "Pressing a tool button appears to do nothing" is the reported symptom, and with the tool
/// left on Select every drag on empty space is a pan — which is the rest of the report. So the
/// button's click is asserted through the rendered strip rather than by calling the handler.
///
/// The strip's geometry is fixed: 32 pixels tall at the top of the panel, `px_2` of padding,
/// then eight 30-pixel buttons with a 4-pixel gap. Rectangle is the third.
#[gpui::test]
fn pressing_the_rectangle_tool_button_picks_it_up(cx: &mut TestAppContext) {
    let (state, window) = capture_window(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();

    let viewport = state.read_with(&vcx.cx, |state, _| state.viewport(KEY));
    assert!(
        viewport.panel_w >= 8.0,
        "the panel is on screen, so the strip above it is too"
    );

    // The strip sits directly above the picture surface.
    let at = Point {
        x: px(viewport.origin_x + 8.0 + 2.0 * 34.0 + 15.0),
        y: px(viewport.origin_y - 16.0),
    };
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();

    let (_, tool) = scene_state(&state, KEY, &vcx.cx);
    assert_eq!(
        tool,
        ImageTool::Rectangle,
        "the Rectangle button's click reached set_image_tool"
    );
}
