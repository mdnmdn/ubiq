//! The settings container, and the one thing its conversion had to buy: nothing moved.
//!
//! `SettingsSection` and `ProjectNav` were closed enums matched for a label, an icon, a body, a
//! count, an on-arrival refresh and an enabled test; they are now `SlotId` newtypes over one
//! registry (`D180`). The gate on that conversion is that the base's own screens draw exactly what
//! they drew before, so the rows and their order are pinned here rather than left to be noticed.

use ubiq::ext::ids;
use ubiq::ext::settings::{SectionGate, SettingsContainer, app_sections, project_sections};
use ubiq::ui::sink::project::Form;

#[test]
fn the_overlay_draws_the_same_fifteen_rows_in_the_same_order() {
    // M4 appends the kitchen sink's own demo section (`X11`) after the base's original fifteen —
    // the container's first genuine contribution, not a conversion, registered through
    // `ext::settings::register` with nothing here touched to add it.
    let rows: Vec<&str> = app_sections().iter().map(|spec| spec.label).collect();
    assert_eq!(
        rows,
        [
            "Appearance",
            "Size",
            "File explorer",
            "Editor",
            "Search",
            "Harnesses",
            "Agent definitions",
            "Isolation",
            "Assistance",
            "Connectors",
            "Hosts",
            "SSH profiles",
            "Drones",
            "Tools",
            "Command line",
            "Extensions demo",
        ]
    );
}

#[test]
fn the_project_dialog_draws_the_same_eight_rows_in_the_same_order() {
    // M7 appends Task sync to the core group, the container's second genuine contribution — in
    // through `ext::settings::register` from `ui::tasksrc`, with nothing in either container
    // module touched to add it. The base's original eight are untouched and still in order, which
    // is what this test was written to hold.
    let rows: Vec<&str> = project_sections().iter().map(|spec| spec.label).collect();
    assert_eq!(
        rows,
        [
            "General",
            "Tools",
            "Agent definitions",
            "Tasks",
            "Remote",
            "Task sync",
            "Knowledge base",
            "Documentation",
            "Integrations",
        ]
    );
}

/// Every section lands in a group its own container declared, and in the right container.
#[test]
fn every_section_is_in_a_group_its_container_declares() {
    for spec in app_sections() {
        assert_eq!(spec.container, SettingsContainer::App, "{}", spec.id);
        assert!(
            SettingsContainer::App.groups().contains(&spec.group),
            "{} is in {}",
            spec.id,
            spec.group
        );
    }
    for spec in project_sections() {
        assert_eq!(spec.container, SettingsContainer::Project, "{}", spec.id);
        assert!(
            SettingsContainer::Project.groups().contains(&spec.group),
            "{} is in {}",
            spec.id,
            spec.group
        );
    }
}

/// The enablement rule the two call sites used to write out separately: the sink's fixture answers
/// to everything, and the live dialog answers to General always, to the five record-backed
/// sections once there is a record, and never to the two fixture-only pages.
#[test]
fn the_live_dialog_offers_general_always_and_the_five_only_with_a_record() {
    let gate = |id| {
        project_sections()
            .iter()
            .find(|spec| spec.id == id)
            .expect("registered")
            .gate
    };

    let with_record = [
        ids::PROJECT_TOOLS,
        ids::PROJECT_AGENT_DEFINITIONS,
        ids::PROJECT_TASKS,
        ids::PROJECT_REMOTE,
        ids::PROJECT_KB,
    ];
    let sink_only = [ids::PROJECT_DOCUMENTATION, ids::PROJECT_INTEGRATIONS];

    assert_eq!(gate(ids::PROJECT_GENERAL), SectionGate::Always);
    for id in with_record {
        assert_eq!(gate(id), SectionGate::WithRecord, "{id}");
    }
    for id in sink_only {
        assert_eq!(gate(id), SectionGate::SinkOnly, "{id}");
    }

    // The fixture page is a fixture: every row is reachable on it.
    for spec in project_sections() {
        assert!(spec.gate.enabled(Form::Sink, false), "{}", spec.id);
    }
    // Creating a folder that is not in the catalogue yet: General only.
    for spec in project_sections() {
        assert_eq!(
            spec.gate.enabled(Form::Live, false),
            spec.id == ids::PROJECT_GENERAL,
            "{}",
            spec.id
        );
    }
    // Editing an existing project: everything but the two fixture-only pages.
    for spec in project_sections() {
        assert_eq!(
            spec.gate.enabled(Form::Live, true),
            !sink_only.contains(&spec.id),
            "{}",
            spec.id
        );
    }
}

/// The five sections that ask the host something on arrival, and only those five. The chain of
/// `if nav == …` this replaced is what a contributed section could not have joined.
#[test]
fn five_overlay_sections_ask_something_on_arrival() {
    let asking: Vec<&str> = app_sections()
        .iter()
        .filter(|spec| spec.on_show.is_some())
        .map(|spec| spec.id.0)
        .collect();
    assert_eq!(
        asking,
        [
            ids::HARNESSES.0,
            ids::ASSIST.0,
            ids::CONNECTORS.0,
            ids::TOOLS.0,
            ids::COMMAND_LINE.0,
        ]
    );
}

/// The nav prints a count beside exactly four rows, all in the project dialog.
///
/// Task sync's is how many tasks on this board are linked — a live fact about the project, the
/// same kind as the knowledge base's source count and unlike the two fixture counts below it.
#[test]
fn only_the_project_dialogs_last_three_rows_carry_a_count() {
    let counted: Vec<&str> = project_sections()
        .iter()
        .filter(|spec| spec.count.is_some())
        .map(|spec| spec.id.0)
        .collect();
    assert_eq!(
        counted,
        [
            ids::PROJECT_TASK_SYNC.0,
            ids::PROJECT_KB.0,
            ids::PROJECT_DOCUMENTATION.0,
            ids::PROJECT_INTEGRATIONS.0,
        ]
    );
    assert!(app_sections().iter().all(|spec| spec.count.is_none()));
}
