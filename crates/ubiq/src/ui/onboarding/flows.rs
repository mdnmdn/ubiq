//! The onboarding flows the base ships, and their words.
//!
//! A flow is data (`crate::state::onboarding`); this is the one list of them, built on demand so a
//! flow can depend on the window — the IDE walk visits exactly the modes the rail is drawing.
//! Adding a flow or a step is an entry here; nothing else changes.
//!
//! The markdown is authored in the help tree, under `help/onboarding/`, as `hidden: true` pages:
//! the packer validates them and keeps them out of the nav, the catalogue and the bundle, and they
//! are compiled in here, so a step never waits for — or lacks — a help bundle. The dialog drops
//! their frontmatter.

use gpui::App;

use crate::app::AppState;
use crate::state::RailMode;
use crate::state::onboarding::{Flow, FormKind, IDE, Step, StepAction, TOUR, Trigger, WELCOME};
use crate::state::ui_id;

const WELCOME_MD: &str = include_str!("../../../../../help/onboarding/welcome.md");
const TOUR_MD: &str = include_str!("../../../../../help/onboarding/tour.md");
const DONE_MD: &str = include_str!("../../../../../help/onboarding/done.md");
const IDE_MD: &str = include_str!("../../../../../help/onboarding/ide.md");
const IDE_DONE_MD: &str = include_str!("../../../../../help/onboarding/ide-done.md");

/// Every flow, in the order the launch check considers them.
pub fn all(app: &AppState, cx: &App) -> Vec<Flow> {
    vec![welcome(), tour(), ide(app, cx)]
}

/// The first run: what Ubiq is, the colours, the tour as one step, and the way on.
fn welcome() -> Flow {
    Flow {
        id: WELCOME,
        title: "Welcome".into(),
        trigger: Trigger::Start,
        then: None,
        steps: vec![
            Step::markdown("Welcome", WELCOME_MD),
            Step::form("Your colours", FormKind::Palette),
            Step::sub_flow("A quick tour", TOUR, TOUR_MD, "Start the tour"),
            Step::markdown("You're set", DONE_MD),
        ],
    }
}

/// The main screen's areas, one highlight each — run as a sub-flow of the welcome.
fn tour() -> Flow {
    Flow {
        id: TOUR,
        title: "Tour".into(),
        trigger: Trigger::Manual,
        then: None,
        steps: vec![
            Step::highlight(
                "The rail",
                ui_id::RAIL,
                "Modes at the top change what the window shows; the badges under them are the \
                 projects open in this window.",
            ),
            Step::highlight(
                "Projects",
                ui_id::TITLEBAR_PROJECT,
                "The project on screen. Click it to switch to another, or to open one that is not \
                 here yet.",
            ),
            Step::highlight(
                "The command field",
                ui_id::TITLEBAR_COMMAND,
                "One field for finding a file, searching the project and running a command — ⌘P \
                 to go to a file, ⌘⏎ to search.",
            ),
            Step::highlight(
                "Actions",
                ui_id::TITLEBAR_ACTIONS,
                "Start an agent or a terminal, search, open help and settings, switch light and \
                 dark — and the overflow menu for the rest.",
            ),
            Step::highlight(
                "Notifications",
                ui_id::TITLEBAR_NOTIFICATIONS,
                "What agents and the host have reported. A count means something is waiting for \
                 you.",
            ),
            Step::highlight(
                "The workspace",
                ui_id::WORKSPACE,
                "The mode's own screen, with panels docked around it. Drag a panel's tab to move \
                 it; the titlebar's region switches show and hide the edges.",
            ),
            Step::highlight(
                "The status bar",
                ui_id::STATUS_BAR,
                "Counts for the mode you are in, the update when one is ready, and the size \
                 control for the whole window.",
            ),
        ],
    }
}

/// The first visit to the IDE with a project: the rail, then every project mode it draws.
fn ide(app: &AppState, cx: &App) -> Flow {
    let mut steps = vec![
        Step::markdown("Project modes", IDE_MD),
        Step::highlight(
            "Modes",
            ui_id::RAIL_MODES,
            "Every screen this project offers. Click one, or press ⌃ and its number.",
        )
        .entering(StepAction::EnterMode(RailMode::IDE)),
    ];
    steps.extend(
        RailMode::project_modes()
            .filter(|mode| {
                mode.spec().is_some_and(|spec| spec.needs_project) && app.mode_enabled(*mode, cx)
            })
            .map(|mode| {
                let id = ui_id::rail_mode(mode);
                let text = ui_id::describe(&id)
                    .map(|info| info.blurb)
                    .unwrap_or_else(|| mode.note());
                Step::highlight(mode.label(), id, text).entering(StepAction::EnterMode(mode))
            }),
    );
    steps.push(
        Step::markdown("Back to the IDE", IDE_DONE_MD)
            .entering(StepAction::EnterMode(RailMode::IDE)),
    );
    Flow {
        id: IDE,
        title: "Project modes".into(),
        trigger: Trigger::EnterMode(RailMode::IDE),
        then: None,
        steps,
    }
}
