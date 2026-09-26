//! The rail container, and the two things its conversion had to buy.
//!
//! `RailMode` was a closed enum matched in nine places for an icon, a `UiId`, a destination, a
//! centre screen, a default layout, a default pair of regions and two tables of furniture; it is
//! now a `SlotId` newtype over one registry (`D184`). Five of those nine sites were
//! fallback-guarded — a `_ =>` arm that gave a mode a plausible-looking wrong default rather than
//! an error — so what is pinned here is both halves:
//!
//! 1. **The rail draws what it drew before** — the same ten modes, the same two groups, the same
//!    order, the same labels and slugs, and the same regions on a first visit.
//! 2. **A saved arrangement survives a mode this build has never heard of.** `RailMode` is a
//!    serialized map key, and dropping an unregistered one silently loses a person's layout for a
//!    mode a second edition contributes. That is `X10`'s seventh step, and it is the reason this
//!    file exists rather than the ordering alone.

use ubiq::ext::ids;
use ubiq::ext::rail::{self, Availability};
use ubiq::state::RailMode;
use ubiq::state::prefs::{self, ModeLayout, ViewPrefs};

// ── 1. The rail is byte-identical ───────────────────────────────────

#[test]
fn the_rail_draws_the_same_ten_modes_in_the_same_order() {
    // M4 lands the kitchen sink's own demo mode (`X11`) alongside the base's original ten — the
    // first genuine contribution on the container, not a conversion, so it is asserted here as an
    // addition rather than folded silently into "the same ten". Its `Availability::When` and its
    // lack of a legacy name are pinned by `every_base_mode_is_always_available` and
    // `the_legacy_table_covers_every_one_of_the_ten_original_modes` (in `ext::rail`'s own tests).
    let rows: Vec<&str> = rail::modes().iter().map(|spec| spec.label).collect();
    assert_eq!(
        rows,
        [
            // APP
            "Control",
            "All Teams",
            "Sink",
            "Extensions demo",
            // PROJECT
            "IDE",
            "Git",
            "Agents",
            "Teams",
            "[Teams]",
            "KB",
            "Tasks",
        ]
    );
}

#[test]
fn the_two_groups_hold_what_they_always_held() {
    let groups = RailMode::groups();
    let names: Vec<&str> = groups.iter().map(|(label, _)| *label).collect();
    assert_eq!(names, ["APP", "PROJECT"], "the headings and their order");

    let app: Vec<&str> = groups[0].1.iter().map(|m| m.label()).collect();
    assert_eq!(app, ["Control", "All Teams", "Sink", "Extensions demo"]);

    // `ctrl-1` counts these, so the order is a keyboard contract and not only a drawing one.
    let project: Vec<&str> = groups[1].1.iter().map(|m| m.label()).collect();
    assert_eq!(
        project,
        ["IDE", "Git", "Agents", "Teams", "[Teams]", "KB", "Tasks"]
    );
    let slots: Vec<&str> = RailMode::project_modes().map(|m| m.label()).collect();
    assert_eq!(slots, project, "`ctrl-1` is still IDE");
}

/// The help keys. A slug is what `rail.<slug>` binds a page to, so a conversion that changed one
/// would quietly unbind the page that claimed the old spelling.
#[test]
fn every_slug_is_the_one_a_help_page_already_claims() {
    let slugs: Vec<&str> = rail::modes().iter().map(|spec| spec.slug).collect();
    assert_eq!(
        slugs,
        [
            "control", "teamsall", "sink", "ext-demo", "ide", "git", "agents", "teams", "teamsold",
            "kb", "tasks",
        ]
    );
}

/// `ModeLayout::default_for` was one of the five silent sites: a `_ => (false, false)` arm. These
/// are the four modes that answered something else, and they still do.
#[test]
fn a_first_visit_opens_the_same_regions() {
    let regions = |mode: RailMode| {
        let layout = ModeLayout::default_for(mode);
        (layout.show_left, layout.show_bottom, layout.show_right)
    };
    assert_eq!(
        regions(RailMode::GIT),
        (true, false, true),
        "Git claims both"
    );
    assert_eq!(regions(RailMode::IDE), (true, false, false));
    assert_eq!(regions(RailMode::KB), (true, false, false));
    assert_eq!(regions(RailMode::AGENTS), (true, false, false));
    assert_eq!(regions(RailMode::TASKS), (false, false, true));
    for mode in [
        RailMode::CONTROL,
        RailMode::TEAMS,
        RailMode::TEAMS_ALL,
        RailMode::TEAMS_OLD,
        RailMode::SINK,
    ] {
        assert_eq!(regions(mode), (false, false, false), "{}", mode.as_str());
    }
    // And a mode nothing is registered under: the centre alone, which is what the old `_ =>` arm
    // gave it — the difference is that it is now an answer rather than a fall-through.
    assert_eq!(
        regions(RailMode(ubiq::ext::SlotId::new("studio.rail.portfolio"))),
        (false, false, false)
    );
}

/// Control and Sink are the two *base* modes that report on the application rather than on a
/// project's folder, so a pane started from either has nowhere to be seen and the window moves to
/// the IDE first. The kitchen sink's own demo mode (M4) answers the same way, for the same reason
/// — it is about the extension mechanism, not about a project's folder either.
#[test]
fn only_the_application_modes_have_no_pane_region() {
    let without: Vec<&str> = rail::modes()
        .iter()
        .filter(|spec| !spec.has_pane_region)
        .map(|spec| spec.label)
        .collect();
    assert_eq!(without, ["Control", "Sink", "Extensions demo"]);
}

/// The sink is the base's one mode that is not a place: there is no project behind it to address.
/// The demo mode beside it answers the same way — it is not a place either.
#[test]
fn the_sink_and_the_demo_mode_are_not_a_destination() {
    let placeless: Vec<&str> = rail::modes()
        .iter()
        .filter(|spec| spec.destination.is_none())
        .map(|spec| spec.label)
        .collect();
    assert_eq!(placeless, ["Sink", "Extensions demo"]);
}

/// All ten of the base's modes are built, and so is the kitchen sink's own demo (M4), so the
/// `not_built` page is reachable only by a contribution that leaves `centre` at `None` — which is
/// what it now *says*, rather than being the `mode => not_built(mode)` arm every unconverted mode
/// fell into.
#[test]
fn every_registered_mode_has_a_screen() {
    for spec in rail::modes() {
        assert!(spec.centre.is_some(), "`{}` has no centre", spec.id);
    }
}

/// None of the base's own ten is conditional. `OptIn` and `When` exist for contributions, and a
/// base mode acquiring one would change what the rail draws for everybody. The kitchen sink's own
/// demo mode (M4) is the one exception on purpose: it is `When`, which is the whole point of it —
/// the base's ten never exercised that arm before this milestone (`D184`'s finding).
#[test]
fn every_base_mode_is_always_available() {
    for spec in rail::modes() {
        if spec.id == ids::EXT_DEMO_RAIL {
            assert!(
                matches!(spec.availability, Availability::When(_)),
                "the demo mode is `When`"
            );
            continue;
        }
        assert!(
            matches!(spec.availability, Availability::Always),
            "`{}` is not `Always`",
            spec.id
        );
    }
}

#[test]
fn every_mode_is_in_a_group_the_rail_declares() {
    for spec in rail::modes() {
        assert!(
            rail::GROUPS.iter().any(|(id, _)| *id == spec.group),
            "`{}` is in undeclared group `{}`",
            spec.id,
            spec.group
        );
    }
}

// ── 2. The serialization step (`X10`) ───────────────────────────────

/// **The test this milestone exists for.**
///
/// A blob holding a layout for a mode this build has no registration for — a second edition's,
/// read by a base launch — keeps that key and that layout, and writes both back out. Dropping it
/// is silent and permanent: the person's arrangement for that mode is gone the first time the
/// other edition is not running.
#[test]
fn an_unregistered_mode_keeps_its_saved_layout_across_a_round_trip() {
    let contributed = RailMode(rail::decode_id("studio.rail.portfolio"));
    assert!(
        contributed.spec().is_none(),
        "the premise: nothing is registered under it"
    );

    let mut view = ViewPrefs::default();
    view.rail_mode = RailMode::AGENTS;
    view.modes.insert(
        contributed,
        ModeLayout {
            show_left: true,
            show_bottom: false,
            show_right: true,
            layout: Some(serde_json::json!({"version": prefs::SCHEMA, "center": "portfolio"})),
        },
    );
    view.hidden_modes.push(contributed);
    view.opted_in_modes.push(contributed);

    let back: ViewPrefs = prefs::decode(&prefs::encode(&view)).expect("a blob this build wrote");

    let kept = back
        .modes
        .get(&contributed)
        .expect("the unregistered mode's layout is still there");
    assert!(kept.show_left && kept.show_right, "and it is unchanged");
    assert_eq!(
        kept.layout, view.modes[&contributed].layout,
        "the dock blob with it"
    );
    assert_eq!(back.hidden_modes, [contributed]);
    assert_eq!(back.opted_in_modes, [contributed]);
}

/// The wire form is the id, not the `Debug` name it used to be.
#[test]
fn a_mode_is_written_down_as_its_id() {
    let blob = prefs::encode(&RailMode::TEAMS_ALL);
    assert_eq!(blob, "\"ubiq.rail.teams-all\"");
}

/// The decoder aliases. A blob written at schema 5 names the ten old variants, and every one of
/// them still lands on the mode it always meant.
#[test]
fn a_schema_five_blob_reads_its_modes_under_the_old_names() {
    let blob = r#"{
        "schema": 5,
        "rail_mode": "TeamsOld",
        "modes": {
            "Git": {"show_left": true, "show_bottom": false, "show_right": true},
            "Kb": {"show_left": true, "show_bottom": false, "show_right": false}
        },
        "hidden_modes": ["Sink", "TeamsAll"]
    }"#;

    let view: ViewPrefs = prefs::decode(blob).expect("a schema-5 blob is read, not discarded");
    assert_eq!(view.schema, prefs::SCHEMA, "and comes back at schema 6");
    assert_eq!(view.rail_mode, RailMode::TEAMS_OLD);
    assert!(view.modes.contains_key(&RailMode::GIT));
    assert!(view.modes.contains_key(&RailMode::KB));
    assert_eq!(view.hidden_modes, [RailMode::SINK, RailMode::TEAMS_ALL]);

    // Every one of the ten, so no mode loses its arrangement at the schema step. The kitchen
    // sink's own demo mode (M4) is excluded on purpose: it never had a schema-5 variant name,
    // being a contribution rather than a conversion.
    for spec in rail::modes() {
        if spec.id == ids::EXT_DEMO_RAIL {
            continue;
        }
        let legacy = format!("{{\"schema\":5,\"rail_mode\":{:?}}}", legacy_name(spec.id));
        let view: ViewPrefs = prefs::decode(&legacy).expect("every old name is read");
        assert_eq!(view.rail_mode, RailMode(spec.id), "{}", spec.id);
    }
}

/// The name each id used to serialize under — the other half of the alias table, written out here
/// so the test is not reading the table it is checking.
fn legacy_name(id: ubiq::ext::SlotId) -> &'static str {
    match id {
        i if i == ids::RAIL_CONTROL => "Control",
        i if i == ids::RAIL_IDE => "Ide",
        i if i == ids::RAIL_GIT => "Git",
        i if i == ids::RAIL_AGENTS => "Agents",
        i if i == ids::RAIL_TEAMS => "Teams",
        i if i == ids::RAIL_TEAMS_ALL => "TeamsAll",
        i if i == ids::RAIL_TEAMS_OLD => "TeamsOld",
        i if i == ids::RAIL_KB => "Kb",
        i if i == ids::RAIL_TASKS => "Tasks",
        i if i == ids::RAIL_SINK => "Sink",
        other => panic!("`{other}` is a base mode with no legacy name"),
    }
}

/// A blob from a build *above* this one is still discarded whole rather than half-applied — the
/// schema bump did not turn the probe into a best-effort read.
#[test]
fn a_blob_from_a_newer_build_is_still_discarded() {
    let blob = r#"{"schema": 99, "rail_mode": "ubiq.rail.ide"}"#;
    assert!(prefs::decode::<ViewPrefs>(blob).is_none());
}
