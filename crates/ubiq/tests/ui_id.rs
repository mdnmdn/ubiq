//! The identity layer behind in-place help: the `UiId` grammar and hierarchy, the catalogue, and
//! the overlay's deepest-hit lookup.
//!
//! The hit-test is deliberately plain arithmetic over `MarkRect`, so everything here runs with no
//! window, no GPUI and no rendering — which is the point of keeping the registry in `state/`.

use ubiq::state::RailMode;
use ubiq::state::ui_id::{
    self, MarkRect, MarkRegistry, UiId, UiIdError, UiMark, deepest_hit, validate,
};

fn mark(path: &'static str, x: f32, y: f32, w: f32, h: f32) -> UiMark {
    UiMark {
        id: UiId::new(path),
        rect: MarkRect::new(x, y, w, h),
    }
}

// --- the grammar ---------------------------------------------------------

#[test]
fn a_well_formed_path_parses() {
    for path in [
        "rail",
        "rail.mode.tasks",
        "titlebar.new-terminal-menu",
        "panel.explorer.tree",
        "a",
        "a1.b2c3",
    ] {
        assert!(validate(path).is_ok(), "{path} should be valid");
        assert_eq!(UiId::parse(path).unwrap().as_str(), path);
    }
}

#[test]
fn a_malformed_path_is_refused_with_a_reason() {
    assert_eq!(validate(""), Err(UiIdError::Empty));
    assert_eq!(validate("rail."), Err(UiIdError::EmptySegment));
    assert_eq!(validate(".rail"), Err(UiIdError::EmptySegment));
    assert_eq!(validate("rail..mode"), Err(UiIdError::EmptySegment));
    assert!(matches!(validate("Rail"), Err(UiIdError::BadSegment(_))));
    assert!(matches!(
        validate("rail_mode"),
        Err(UiIdError::BadSegment(_))
    ));
    assert!(matches!(
        validate("rail.mode-"),
        Err(UiIdError::BadSegment(_))
    ));
    assert!(matches!(
        validate("rail.mo--de"),
        Err(UiIdError::BadSegment(_))
    ));
    assert!(matches!(validate("1rail"), Err(UiIdError::BadSegment(_))));
    assert!(matches!(
        validate("rail mode"),
        Err(UiIdError::BadSegment(_))
    ));
    assert!(matches!(
        validate("a.b.c.d.e.f.g.h.i"),
        Err(UiIdError::TooDeep(9))
    ));
    assert!(matches!(
        validate(&"a".repeat(ui_id::MAX_LEN + 1)),
        Err(UiIdError::TooLong(_))
    ));
}

#[test]
fn a_const_id_and_a_parsed_one_are_the_same_id() {
    const CONSTRUCTED: UiId = UiId::new("rail.mode.tasks");
    assert_eq!(CONSTRUCTED, UiId::parse("rail.mode.tasks").unwrap());
    assert_eq!(CONSTRUCTED.to_string(), "rail.mode.tasks");
    assert_eq!("rail.mode.tasks".parse::<UiId>().unwrap(), CONSTRUCTED);
}

// --- the hierarchy -------------------------------------------------------

#[test]
fn the_dots_are_the_hierarchy() {
    let id = UiId::new("rail.mode.tasks");
    assert_eq!(id.depth(), 3);
    assert_eq!(id.leaf(), "tasks");
    assert_eq!(id.segments().collect::<Vec<_>>(), ["rail", "mode", "tasks"]);
    assert_eq!(id.parent(), Some(UiId::new("rail.mode")));
    assert_eq!(
        id.ancestors().collect::<Vec<_>>(),
        [UiId::new("rail.mode"), UiId::new("rail")]
    );
    assert_eq!(id.parent().unwrap().child("tasks"), id);
}

#[test]
fn a_root_has_no_parent() {
    let root = UiId::new("rail");
    assert_eq!(root.depth(), 1);
    assert_eq!(root.parent(), None);
    assert_eq!(root.ancestors().count(), 0);
    assert_eq!(root.leaf(), "rail");
}

#[test]
fn ancestry_is_compared_by_level_not_by_prefix() {
    let rail = UiId::new("rail");
    assert!(rail.is_ancestor_of(&UiId::new("rail.mode")));
    assert!(rail.is_ancestor_of(&UiId::new("rail.mode.tasks")));
    // A shared text prefix that does not end on a separator is a different branch.
    assert!(!rail.is_ancestor_of(&UiId::new("railing.mode")));
    // Nothing is its own ancestor, and the relation only points downwards.
    assert!(!rail.is_ancestor_of(&rail));
    assert!(!UiId::new("rail.mode").is_ancestor_of(&rail));
}

// --- the written-down form -----------------------------------------------

#[test]
fn a_name_round_trips_through_its_qualified_form() {
    let id = UiId::new("rail.mode.tasks");
    assert_eq!(id.qualified(), "ui.rail.mode.tasks");
    assert_eq!(UiId::from_qualified("ui.rail.mode.tasks"), Some(id));
    // Strings in other namespaces are not ours — help's own context keys among them.
    assert_eq!(UiId::from_qualified("rail.tasks"), None);
    assert_eq!(UiId::from_qualified("panel.ubiq.git-changes"), None);
    // A `ui.` string that is not a legal name resolves to nothing rather than to a bad id.
    assert_eq!(UiId::from_qualified("ui.Rail"), None);
    assert_eq!(UiId::from_qualified("ui."), None);
}

// --- the catalogue -------------------------------------------------------

#[test]
fn the_catalogue_is_well_formed() {
    ui_id::catalogue_is_well_formed().unwrap();
}

#[test]
fn every_rail_mode_has_a_catalogued_name() {
    for mode in [
        RailMode::Control,
        RailMode::Ide,
        RailMode::Git,
        RailMode::Agents,
        RailMode::Teams,
        RailMode::TeamsAll,
        RailMode::TeamsOld,
        RailMode::Kb,
        RailMode::Tasks,
        RailMode::Sink,
    ] {
        let id = ui_id::rail_mode(mode);
        assert!(ui_id::is_known(&id), "{id} is not in the catalogue");
        assert!(
            ui_id::RAIL.is_ancestor_of(&id),
            "{id} should sit under the rail"
        );
    }
}

#[test]
fn the_areas_the_next_phase_marks_are_named() {
    for id in [
        &ui_id::RAIL,
        &ui_id::TITLEBAR,
        &ui_id::TITLEBAR_HELP,
        &ui_id::TITLEBAR_SEARCH,
        &ui_id::TITLEBAR_THEME,
        &ui_id::TITLEBAR_REGION_LEFT,
    ] {
        assert!(ui_id::is_known(id), "{id} is not in the catalogue");
    }
    // A name nobody declared resolves to nothing — that is what the catalogue is for.
    assert!(!ui_id::is_known(&UiId::new("titlebar.helpp")));
}

// --- the hit-test --------------------------------------------------------

#[test]
fn nothing_marked_means_nothing_hit() {
    assert!(deepest_hit(&[], 10., 10.).is_none());
}

#[test]
fn a_cursor_outside_everything_hits_nothing() {
    let marks = [mark("rail", 0., 0., 48., 600.)];
    assert!(deepest_hit(&marks, 100., 100.).is_none());
    assert!(deepest_hit(&marks, -1., 10.).is_none());
}

#[test]
fn the_deepest_name_containing_the_point_wins() {
    // The rail, its mode column, and one button in it — recorded outermost first, as layout
    // would produce them.
    let marks = [
        mark("rail", 0., 0., 48., 600.),
        mark("rail.modes", 0., 40., 48., 400.),
        mark("rail.mode.tasks", 0., 120., 48., 40.),
    ];
    assert_eq!(
        deepest_hit(&marks, 24., 130.).unwrap().id.as_str(),
        "rail.mode.tasks"
    );
    assert_eq!(
        deepest_hit(&marks, 24., 200.).unwrap().id.as_str(),
        "rail.modes"
    );
    assert_eq!(deepest_hit(&marks, 24., 10.).unwrap().id.as_str(), "rail");
}

#[test]
fn record_order_does_not_decide_depth() {
    // The same three, recorded deepest first. Depth, not position in the list, is what ranks.
    let marks = [
        mark("rail.mode.tasks", 0., 120., 48., 40.),
        mark("rail.modes", 0., 40., 48., 400.),
        mark("rail", 0., 0., 48., 600.),
    ];
    assert_eq!(
        deepest_hit(&marks, 24., 130.).unwrap().id.as_str(),
        "rail.mode.tasks"
    );
}

#[test]
fn between_two_names_at_the_same_depth_the_smaller_one_wins() {
    let marks = [
        mark("titlebar.actions", 200., 0., 400., 32.),
        mark("titlebar.help", 300., 0., 28., 32.),
    ];
    // Both are two levels deep and both contain the point; the button is the answer.
    assert_eq!(
        deepest_hit(&marks, 310., 16.).unwrap().id.as_str(),
        "titlebar.help"
    );
    assert_eq!(
        deepest_hit(&marks, 250., 16.).unwrap().id.as_str(),
        "titlebar.actions"
    );
}

#[test]
fn an_exact_tie_goes_to_the_later_record() {
    // Same depth, same rectangle: layout order is paint order, so the later one is on top.
    let marks = [
        mark("titlebar.search", 0., 0., 32., 32.),
        mark("titlebar.theme", 0., 0., 32., 32.),
    ];
    assert_eq!(
        deepest_hit(&marks, 16., 16.).unwrap().id.as_str(),
        "titlebar.theme"
    );
}

#[test]
fn a_child_drawn_outside_its_parent_still_hits() {
    // An anchored menu is a child by name and elsewhere on screen. Nothing in the lookup assumes
    // containment.
    let marks = [
        mark("titlebar", 0., 0., 800., 32.),
        mark("titlebar.overflow.menu", 600., 32., 200., 300.),
    ];
    assert_eq!(
        deepest_hit(&marks, 650., 100.).unwrap().id.as_str(),
        "titlebar.overflow.menu"
    );
}

#[test]
fn edges_are_half_open_and_empty_rectangles_hit_nothing() {
    let marks = [
        mark("titlebar.search", 0., 0., 32., 32.),
        mark("titlebar.theme", 32., 0., 32., 32.),
    ];
    // The shared edge belongs to exactly one of them.
    assert_eq!(
        deepest_hit(&marks, 32., 16.).unwrap().id.as_str(),
        "titlebar.theme"
    );
    assert_eq!(
        deepest_hit(&marks, 31.9, 16.).unwrap().id.as_str(),
        "titlebar.search"
    );
    // Past the far edge of the last one is nothing at all.
    assert!(deepest_hit(&marks, 64., 16.).is_none());
    // A control laid out to nothing is not a target.
    assert!(deepest_hit(&[mark("titlebar.help", 0., 0., 0., 32.)], 0., 16.).is_none());
}

// --- the registry --------------------------------------------------------

#[test]
fn a_registry_answers_from_the_frame_that_finished() {
    let mut registry = MarkRegistry::new();
    // Nothing recorded yet.
    assert!(registry.hit(10., 10.).is_none());
    assert!(registry.marks().is_empty());

    // Frame one records the rail. It is not readable until the frame is retired — an overlay is
    // built before layout runs, so it always reads the previous frame.
    registry.begin_frame();
    registry.record(ui_id::RAIL, MarkRect::new(0., 0., 48., 600.));
    assert!(registry.hit(10., 10.).is_none());

    registry.begin_frame();
    assert_eq!(registry.marks().len(), 1);
    assert_eq!(registry.hit(10., 10.).unwrap().id, ui_id::RAIL);

    // Frame two draws the titlebar instead. Once retired, the rail is gone rather than lingering.
    registry.record(ui_id::TITLEBAR, MarkRect::new(0., 0., 800., 32.));
    registry.begin_frame();
    assert_eq!(registry.marks().len(), 1);
    assert!(registry.hit(10., 300.).is_none());
    assert_eq!(registry.hit(10., 10.).unwrap().id, ui_id::TITLEBAR);

    registry.clear();
    assert!(registry.marks().is_empty());
}

// --- G316: the coordinate space a real layout reports --------------------

/// A skeleton the size and shape of the window's chrome, with two marks in it.
///
/// This is the one thing the hit-test above cannot assert: every rectangle there is handed in by
/// the test, so nothing checks what GPUI actually puts in one. `.ui_id()` records the bounds its
/// `canvas` prepaint is given, and whether those are *window* coordinates or coordinates relative
/// to some ancestor is the whole question — a rail recorded at `y = 0` instead of `y = 34` would
/// make every highlight land in the wrong place, and no state-level test would notice.
struct Skeleton;

const BAR: f32 = 34.;
const RAIL: f32 = 56.;

impl gpui::Render for Skeleton {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div, px};
        use ubiq::ui::ident::Identified as _;

        ubiq::ui::ident::begin_frame(window);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(BAR)).w_full().ui_id(ui_id::TITLEBAR))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(div().w(px(RAIL)).h_full().ui_id(ui_id::RAIL))
                    .child(div().flex_1().min_w(px(0.))),
            )
    }
}

fn recorded_rect(marks: &[UiMark], id: &UiId) -> MarkRect {
    marks
        .iter()
        .find(|mark| &mark.id == id)
        .unwrap_or_else(|| panic!("{id} was recorded; got {marks:?}"))
        .rect
}

/// `.ui_id()` reports **window** coordinates, and the registry is one frame behind.
#[gpui::test]
fn a_mark_records_where_the_element_really_is(cx: &mut gpui::TestAppContext) {
    use gpui::VisualTestContext;

    let handle = cx.add_window(|_, _| Skeleton);
    let mut vcx = VisualTestContext::from_window(*std::ops::Deref::deref(&handle), cx);
    vcx.run_until_parked();

    // The registry answers from the frame that finished, so one more frame has to be drawn before
    // anything is readable. Draw twice and read; if this were not true the overlay would show an
    // empty window on its first paint rather than one stale frame.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let (marks, viewport) =
        vcx.update(|window, _| (ubiq::ui::ident::recorded(window), window.viewport_size()));
    let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
    assert!(w > RAIL && h > BAR, "the test window has room: {w}x{h}");

    let bar = recorded_rect(&marks, &ui_id::TITLEBAR);
    assert_eq!(
        (bar.x, bar.y, bar.width, bar.height),
        (0., 0., w, BAR),
        "the titlebar fills the top of the window"
    );

    // The one that matters: the rail is the *second* row, so a parent-relative report would put it
    // at y = 0 and a window-relative one at y = BAR.
    let rail = recorded_rect(&marks, &ui_id::RAIL);
    assert_eq!(
        (rail.x, rail.y, rail.width, rail.height),
        (0., BAR, RAIL, h - BAR),
        "the rail is recorded in window coordinates, below the titlebar"
    );

    // And the lookup the overlay runs agrees with both.
    let mut hit = |x, y| {
        vcx.update(|window, _| {
            ubiq::ui::ident::hit_at(window, gpui::point(gpui::px(x), gpui::px(y)))
        })
    };
    assert_eq!(hit(10., 10.), Some(ui_id::TITLEBAR));
    assert_eq!(hit(10., BAR + 10.), Some(ui_id::RAIL));
    assert_eq!(hit(RAIL + 10., BAR + 10.), None, "nothing marked out there");
}

// --- G318: the two shapes phase 3 will actually mark ----------------------

/// A flex row of three children, its own `ui_id` present or absent by `marked`.
///
/// `.ui_id()` adds a `relative` and an absolutely-positioned `canvas` child to whatever receives
/// it. An absolutely-positioned child is supposed to leave the flow untouched, so it should not be
/// able to nudge the flex children the container already has — but the earlier render test only
/// ever marked a leaf, so nothing proved that on a container with children of its own. The rail is
/// exactly this shape.
struct FlexContainer {
    marked: bool,
}

impl gpui::Render for FlexContainer {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{IntoElement as _, ParentElement as _, Styled as _, div, px};
        use ubiq::ui::ident::Identified as _;

        ubiq::ui::ident::begin_frame(window);

        let container = div()
            .size_full()
            .flex()
            .child(div().w(px(50.)).h(px(40.)).ui_id(UiId::new("test.flex.a")))
            .child(div().w(px(60.)).h(px(40.)).ui_id(UiId::new("test.flex.b")))
            .child(div().flex_1().h(px(40.)).ui_id(UiId::new("test.flex.c")));

        if self.marked {
            container
                .ui_id(UiId::new("test.flex.container"))
                .into_any_element()
        } else {
            container.into_any_element()
        }
    }
}

/// Marking a flex container's own bounds does not displace the children it already has.
#[gpui::test]
fn marking_a_flex_container_does_not_move_its_children(cx: &mut gpui::TestAppContext) {
    use gpui::VisualTestContext;

    let child_ids = [
        UiId::new("test.flex.a"),
        UiId::new("test.flex.b"),
        UiId::new("test.flex.c"),
    ];

    let record_for = |cx: &mut gpui::TestAppContext, marked: bool| {
        let handle = cx.add_window(move |_, _| FlexContainer { marked });
        let mut vcx = VisualTestContext::from_window(*std::ops::Deref::deref(&handle), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        let marks = vcx.update(|window, _| ubiq::ui::ident::recorded(window));
        child_ids
            .iter()
            .map(|id| recorded_rect(&marks, id))
            .collect::<Vec<_>>()
    };

    let unmarked = record_for(cx, false);
    let marked = record_for(cx, true);

    assert_eq!(
        unmarked, marked,
        "marking the container displaced its own children: {unmarked:?} vs {marked:?}"
    );
}

/// A container that is itself already `.absolute()`, then marked.
///
/// `ui/ident.rs`'s doc comment calls `.ui_id()` on an already-`.absolute()` element a conflict,
/// because `.ui_id()` forces its receiver `.relative()` and the two are one field in the style.
/// This puts a number on what actually happens rather than trusting the comment.
///
/// A single child at a parent's origin would land at the same `(left, top)` whether it is
/// genuinely out of flow or merely `relative` with the same offsets — that case would prove
/// nothing. The spacer above it is what tells the two apart: if `.ui_id()` silently turned
/// `.absolute()` back into `.relative()`, the marked child would flow *after* the spacer and its
/// `top` offset would stack on top of the spacer's height instead of measuring from the parent.
struct AbsoluteMarked;

const SPACER_HEIGHT: f32 = 15.;

impl gpui::Render for AbsoluteMarked {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div, px};
        use ubiq::ui::ident::Identified as _;

        ubiq::ui::ident::begin_frame(window);

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().h(px(SPACER_HEIGHT)).w_full())
            .child(
                div()
                    .absolute()
                    .top(px(20.))
                    .left(px(30.))
                    .w(px(40.))
                    .h(px(25.))
                    .ui_id(UiId::new("test.absolute.child")),
            )
    }
}

#[gpui::test]
fn marking_an_already_absolute_element(cx: &mut gpui::TestAppContext) {
    use gpui::VisualTestContext;

    let handle = cx.add_window(|_, _| AbsoluteMarked);
    let mut vcx = VisualTestContext::from_window(*std::ops::Deref::deref(&handle), cx);
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let marks = vcx.update(|window, _| ubiq::ui::ident::recorded(window));

    let rect = recorded_rect(&marks, &UiId::new("test.absolute.child"));
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (30., 20., 40., 25.),
        "an already-.absolute() element keeps its own top/left offsets once marked"
    );
}

// --- what a name means: the overlay's one seam ---------------------------

/// Every name in the catalogue has a sentence bound to it.
///
/// `describe` is allowed to answer `None` — marking is incremental and the balloon says so — but
/// the thirty-odd names phase 1 wrote down were all described in phase 2, and a name added
/// without one is the thing this test is here to catch.
#[test]
fn every_catalogued_name_is_described() {
    for id in ui_id::CATALOGUE {
        let info = ui_id::describe(id).unwrap_or_else(|| panic!("{id} has no description"));
        assert!(!info.label.is_empty(), "{id}: an empty label");
        assert!(
            !info.label.ends_with('.'),
            "{id}: a label is a name, not a sentence"
        );
        assert!(
            info.blurb.ends_with('.'),
            "{id}: a blurb is one sentence and ends like one — {:?}",
            info.blurb
        );
        assert!(
            info.blurb.len() > 20,
            "{id}: a blurb says something — {:?}",
            info.blurb
        );
    }
}

/// A name nobody has described is a `None`, not a panic and not an empty string.
#[test]
fn an_undescribed_name_has_no_info() {
    assert!(ui_id::describe(&UiId::new("rail.mode.nothing-like-this")).is_none());
}

// --- the balloon stays on screen -----------------------------------------

use ubiq::theme;
use ubiq::ui::help_target::place;

/// The ordinary case: the balloon sits just under the target and lines up with its left edge.
#[test]
fn a_balloon_sits_under_its_target() {
    let gap = theme::help_balloon_gap();
    let width = theme::help_balloon_width();
    let rect = MarkRect::new(200., 100., 120., 40.);
    let (left, top) = place(rect, width, 1200., 800.);
    assert_eq!(left, 200.);
    assert_eq!(top, 140. + gap);
}

/// A target at the foot of the window gets its sentence *above* it rather than half off the
/// bottom edge — the flip that makes the rail's last badge readable.
#[test]
fn a_balloon_flips_above_a_target_near_the_bottom() {
    let gap = theme::help_balloon_gap();
    let room = theme::help_balloon_room();
    let width = theme::help_balloon_width();
    let vh = 800.;
    let rect = MarkRect::new(200., vh - 60., 120., 40.);
    let (_, top) = place(rect, width, 1200., vh);
    assert!(top < rect.y, "the balloon flipped above the target: {top}");
    assert!(
        top >= gap && top + room <= vh - gap,
        "and is on screen: {top}"
    );
}

/// A target against the right edge pulls the balloon back in, and one against the top leaves it
/// below where it belongs. Whatever the target, the panel is inside the window on every side.
#[test]
fn a_balloon_is_never_drawn_off_the_window() {
    let gap = theme::help_balloon_gap();
    let room = theme::help_balloon_room();
    let width = theme::help_balloon_width();
    let (vw, vh) = (1200., 800.);

    for rect in [
        MarkRect::new(vw - 30., 0., 30., 34.),       // top right corner
        MarkRect::new(0., 0., 56., 34.),             // top left corner
        MarkRect::new(vw - 30., vh - 30., 30., 30.), // bottom right corner
        MarkRect::new(0., vh - 30., 56., 30.),       // bottom left corner
        MarkRect::new(0., 0., vw, vh),               // the whole window
    ] {
        let (left, top) = place(rect, width, vw, vh);
        assert!(left >= gap, "{rect:?}: left edge {left}");
        assert!(left + width <= vw - gap, "{rect:?}: right edge {left}");
        assert!(top >= gap, "{rect:?}: top edge {top}");
        assert!(top + room <= vh - gap, "{rect:?}: bottom edge {top}");
    }
}

/// A window too small for the balloon is clamped rather than given a negative position: the panel
/// overflows, which is visible, instead of being laid out off the top left, which is not.
#[test]
fn a_window_smaller_than_the_balloon_still_places_it() {
    let width = theme::help_balloon_width();
    let (left, top) = place(MarkRect::new(0., 0., 40., 20.), width, 100., 60.);
    assert!(left >= 0. && top >= 0., "({left}, {top})");
}

// --- G317: the rail and the titlebar are actually marked ------------------
//
// Everything above proves the machinery works; none of it proves the real rail and titlebar call
// `.ui_id()` at all. These tests render the genuine `ui::rail` and `ui::titlebar` — through a real
// `AppState`, in a real (headless) window — and read the registry back, the way
// `a_mark_records_where_the_element_really_is` proved the coordinate space for phase 2.

use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// Build a window on two real, open projects and return every mark two drawn frames produced.
///
/// Two projects, not one: `rail::visible_project_order` (and so `RAIL_PROJECTS`'s badges) draws
/// nothing at all for a window holding only one, which is not the shape phase 3a claims to have
/// marked. The titlebar's new-agent and new-terminal actions, and several rail modes, also only
/// draw once `app.project(cx)` answers `Some`. Two frames for the same reason the G316 test above
/// draws twice — the registry is one frame behind by construction, so the first frame's marks are
/// never readable.
fn render_real_shell(cx: &mut gpui::TestAppContext) -> Vec<UiMark> {
    use gpui::AppContext as _;
    use gpui::VisualTestContext;

    let project = ProjectId::generate();
    let other = ProjectId::generate();
    let snapshot = |id: ProjectId, name: &str| ProjectSnapshot {
        record: ProjectRecord {
            id,
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            colour: 0,
            custom_colour: None,
            temporary: false,
            created_at: chrono::Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: format!("/tmp/{name}-workarea"),
    };

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        cx.global_mut::<WindowRegistry>()
            .apply(snapshot(project, "ubiq"));
        cx.global_mut::<WindowRegistry>()
            .apply(snapshot(other, "ubiq-studio"));
        ubiq::app::install_key_bindings(cx);
    });

    let handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();

    let mut vcx = VisualTestContext::from_window(*std::ops::Deref::deref(&handle), cx);
    vcx.run_until_parked();

    // The window's own slot now holds both, the way `take_project` would leave it, but without
    // the host round trip that call makes — nothing here needs it. The registry, not `AppState`,
    // is what `rail::visible_project_order` reads to decide how many badges there is room for.
    let window_id = vcx.update(|window, _| window.window_handle().window_id());
    vcx.update(|_, cx| {
        let registry = cx.global_mut::<WindowRegistry>();
        registry.open_in(window_id, other);
        registry.activate(window_id, project);
    });

    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.update(|window, _| ubiq::ui::ident::recorded(window))
}

fn rect_contains(outer: MarkRect, inner: MarkRect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

/// Every name phase 3a claims to have marked is actually present in the frame the real rail and
/// titlebar draw — not just declared in the catalogue. This is the mechanical check that a
/// `.ui_id()` call was not lost to a typo, an unreachable branch, or a wrapper that swallowed it.
#[gpui::test]
fn the_rail_and_titlebar_mark_every_name_phase_3a_claims(cx: &mut gpui::TestAppContext) {
    let marks = render_real_shell(cx);
    let recorded: std::collections::HashSet<&UiId> = marks.iter().map(|m| &m.id).collect();

    let claimed = [
        ui_id::RAIL,
        ui_id::RAIL_MARK,
        ui_id::RAIL_MODES,
        ui_id::RAIL_PROJECTS,
        ui_id::rail_mode(RailMode::Control),
        ui_id::rail_mode(RailMode::Ide),
        ui_id::rail_mode(RailMode::Git),
        ui_id::rail_mode(RailMode::Agents),
        ui_id::rail_mode(RailMode::Teams),
        ui_id::rail_mode(RailMode::TeamsAll),
        ui_id::rail_mode(RailMode::TeamsOld),
        ui_id::rail_mode(RailMode::Kb),
        ui_id::rail_mode(RailMode::Tasks),
        ui_id::rail_mode(RailMode::Sink),
        ui_id::TITLEBAR,
        ui_id::TITLEBAR_PROJECT,
        ui_id::TITLEBAR_NEW_PROJECT,
        ui_id::TITLEBAR_NEW_PROJECT_MENU,
        ui_id::TITLEBAR_NAV_BACK,
        ui_id::TITLEBAR_NAV_FORWARD,
        ui_id::TITLEBAR_COMMAND,
        ui_id::TITLEBAR_ACTIONS,
        ui_id::TITLEBAR_REGION_LEFT,
        ui_id::TITLEBAR_REGION_BOTTOM,
        ui_id::TITLEBAR_REGION_RIGHT,
        ui_id::TITLEBAR_NEW_AGENT,
        ui_id::TITLEBAR_NEW_TERMINAL,
        ui_id::TITLEBAR_NEW_TERMINAL_MENU,
        ui_id::TITLEBAR_SEARCH,
        ui_id::TITLEBAR_NOTIFICATIONS,
        ui_id::TITLEBAR_FEEDBACK,
        ui_id::TITLEBAR_HELP,
        ui_id::TITLEBAR_REMOTE_HOSTS,
        ui_id::TITLEBAR_OVERFLOW,
        ui_id::TITLEBAR_THEME,
    ];
    for id in claimed {
        assert!(
            recorded.contains(&id),
            "{id} was claimed as marked but never recorded by the real render"
        );
    }

    // Every mark this pass produced has a sane rectangle: on screen, non-zero.
    for m in &marks {
        assert!(
            m.rect.width > 0. && m.rect.height > 0.,
            "{}: zero-area rect {:?}",
            m.id,
            m.rect
        );
    }

    // Nesting that is genuinely true for this shape: the modes column and the project badges both
    // sit inside the rail, and one rail mode sits inside the modes column.
    let rail = recorded_rect(&marks, &ui_id::RAIL);
    let modes = recorded_rect(&marks, &ui_id::RAIL_MODES);
    let projects = recorded_rect(&marks, &ui_id::RAIL_PROJECTS);
    let control = recorded_rect(&marks, &ui_id::rail_mode(RailMode::Control));
    assert!(
        rect_contains(rail, modes),
        "rail.modes should sit inside the rail: {rail:?} / {modes:?}"
    );
    assert!(
        rect_contains(rail, projects),
        "rail.projects should sit inside the rail: {rail:?} / {projects:?}"
    );
    assert!(
        rect_contains(modes, control),
        "a rail mode should sit inside rail.modes: {modes:?} / {control:?}"
    );
}

/// The point of marking each mode separately: a cursor over one resolves to *that* mode, not to
/// the rail it lives in — the same `deepest_hit` ranking phase 1 built, now proven against a real
/// layout rather than hand-written rectangles.
#[gpui::test]
fn a_point_inside_a_rail_mode_resolves_to_the_mode_not_the_rail(cx: &mut gpui::TestAppContext) {
    let marks = render_real_shell(cx);
    let control = recorded_rect(&marks, &ui_id::rail_mode(RailMode::Control));
    let (x, y) = (
        control.x + control.width / 2.,
        control.y + control.height / 2.,
    );

    let hit = deepest_hit(&marks, x, y).expect("the rail mode is under the cursor");
    assert_eq!(
        hit.id,
        ui_id::rail_mode(RailMode::Control),
        "the deepest name won, not `rail` or `rail.modes`"
    );
}
