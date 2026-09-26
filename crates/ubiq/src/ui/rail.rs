//! The activity rail: the app's destinations, grouped, with exactly one active.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;

use gpui::{
    AnyElement, App, Context, ElementId, Image, ImageFormat, ImageSource, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, img,
    px,
};
use gpui_component::{Icon, Sizable as _, Size};

use crate::app::AppState;
use crate::ext::Registry;
use crate::ext::ids;
use crate::ext::rail::{Availability, RailModeSpec};
use crate::state::dock::{PanelKind, Region};
use crate::state::nav::View;
use crate::state::teams::TeamsSelection;
use crate::state::ui_id;
use crate::state::{RailMode, WindowRegistry};
use crate::theme;
use crate::ui::ident::Identified as _;
use crate::ui::kit::{UbiqIcon, section_label};
use crate::ui::project_face::project_face;
use ubiq_proto::ids::ProjectId;

/// The mark's two files: the white logo reads on a dark swatch, the blue on a light one. They are
/// the only assets Ubiq ships, so they are baked in next to the code that draws them.
const LOGO_WHITE: &[u8] = include_bytes!("../../../../assets/logo-white.png");
const LOGO_BLUE: &[u8] = include_bytes!("../../../../assets/logo-blue.png");

/// What the rail spends per mode and per group heading. Fixed rather than measured: the badges
/// under the modes have to know, while the rail is being built, how much room is left over, and
/// nothing is measured until it is painted.
const ITEM_HEIGHT: f32 = 52.0;
/// The rail's own width less its border — what everything inside it is exactly as wide as. Fixed
/// rather than `w_full`, because a mode's label is wider than the rail and a flex child's
/// automatic minimum size would let that label push its row wider than the rows beside it.
fn item_width() -> f32 {
    theme::rail_width() - 1.0
}
const GROUP_HEIGHT: f32 = 42.0;
/// One project badge: the rail's width less its own border, which is what the selected badge
/// fills edge to edge.
fn badge_height() -> f32 {
    theme::rail_width() - 1.0
}
/// What an unselected badge keeps clear of the rail's edges, and how thick its ring is.
const BADGE_MARGIN: f32 = 3.0;

/// The rail's glyph for a mode — the `mode:` registry category, one row per mode.
///
/// The spec's, so a contributed mode brings its own. A mode nothing is registered under gets the
/// IDE's glyph rather than no element at all: it is only ever reached by an empty page naming a
/// mode a second edition removed.
pub fn mode_icon(mode: RailMode) -> Icon {
    match mode.spec() {
        Some(spec) => (spec.icon)(),
        None => Icon::new(UbiqIcon::ModeIde),
    }
}

/// The base's own ten modes, registered into the rail container (`D184`).
///
/// This is `X4` and invariant 1 in one function: the container has a base-side user from its first
/// commit, and the base's own destinations are ordinary registrations rather than a closed enum a
/// second edition cannot join. The order inside each group is this function's order.
pub fn modes(reg: &mut Registry<RailModeSpec>) {
    use crate::ext::rail::register;
    use crate::state::ui_id as uid;

    // ── The application's own: what the window is, not what one project is ──
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_CONTROL,
            group: ids::RAIL_APP,
            label: "Control",
            note: "What this Ubiq is doing, and what its agents have spent.",
            slug: "control",
            icon: || Icon::new(UbiqIcon::ModeControl),
            ui_id: uid::RAIL_MODE_CONTROL,
            availability: Availability::Always,
            // Control reports on the running host rather than on a project's folder, so a pane
            // started here has nowhere to be seen.
            has_pane_region: false,
            needs_project: false,
            opens_left: false,
            opens_right: false,
            centre: Some(|app, _window, cx| crate::ui::stats::render(app, cx)),
            default_layout: None,
            destination: Some(|_app, _project, _cx| Some(View::Control)),
            furniture: None,
            side_furniture: None,
            // The one screen that has to ask for what it draws, and only while it is up.
            on_enter: Some(|app, cx| app.poll_stats(cx)),
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_TEAMS_ALL,
            group: ids::RAIL_APP,
            label: "All Teams",
            note: "Every open project's agents, arranged on one canvas.",
            slug: "teamsall",
            icon: || Icon::new(UbiqIcon::ModeTeams),
            ui_id: uid::RAIL_MODE_TEAMS_ALL,
            availability: Availability::Always,
            has_pane_region: true,
            // In the APP group but still wanting a project: a canvas about every open project has
            // nothing to draw when there are none.
            needs_project: true,
            opens_left: false,
            opens_right: false,
            centre: Some(|app, window, cx| {
                crate::ui::teams::render(app, window, cx).into_any_element()
            }),
            default_layout: None,
            destination: Some(teams_destination),
            furniture: None,
            side_furniture: None,
            on_enter: None,
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_SINK,
            group: ids::RAIL_APP,
            label: "Sink",
            note: "The application's own test bench.",
            slug: "sink",
            icon: || Icon::new(UbiqIcon::ModeSink),
            ui_id: uid::RAIL_MODE_SINK,
            availability: Availability::Always,
            has_pane_region: false,
            needs_project: false,
            opens_left: false,
            opens_right: false,
            centre: Some(|app, window, cx| crate::ui::sink::render(app, window, cx)),
            default_layout: None,
            // The test bench is not a place: there is no project behind it to address.
            destination: None,
            furniture: None,
            side_furniture: None,
            on_enter: None,
        },
    );

    // ── The project's own, in rail order — `ctrl-1` is the first of these ──
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_IDE,
            group: ids::RAIL_PROJECT,
            label: "IDE",
            note: "",
            slug: "ide",
            icon: || Icon::new(UbiqIcon::ModeIde),
            ui_id: uid::RAIL_MODE_IDE,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            opens_left: true,
            opens_right: false,
            centre: Some(|app, _window, cx| crate::ui::editor::render(app, cx)),
            default_layout: None,
            destination: Some(|app, _project, cx| {
                Some(View::Ide {
                    key: app.editor(cx)?.active_file()?.key(),
                })
            }),
            // The explorer is the one piece of furniture that is not optional: it is how the
            // files are reached at all.
            furniture: Some(|app| app.queue_furniture(PanelKind::Explorer)),
            side_furniture: Some(|region| (region == Region::Left).then_some(PanelKind::Explorer)),
            on_enter: None,
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_GIT,
            group: ids::RAIL_PROJECT,
            label: "Git",
            note: "What version control knows about this project.",
            slug: "git",
            icon: || Icon::new(UbiqIcon::ModeGit),
            ui_id: uid::RAIL_MODE_GIT,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            // Git's changes panel and commit box are half the screen (`D119`), so it claims both
            // edges where every other screen claims one or none.
            opens_left: true,
            opens_right: true,
            centre: Some(|app, _window, cx| crate::ui::editor::render(app, cx)),
            default_layout: Some(crate::ui::dock::default_git_layout),
            destination: Some(|_app, _project, _cx| Some(View::Git)),
            furniture: Some(AppState::queue_git_furniture),
            side_furniture: Some(|region| match region {
                Region::Left => Some(PanelKind::GitRefs),
                Region::Right => Some(PanelKind::GitChanges),
                _ => None,
            }),
            on_enter: None,
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_AGENTS,
            group: ids::RAIL_PROJECT,
            label: "Agents",
            note: "The agents running in this project, one column each.",
            slug: "agents",
            // Generic, so it may not borrow the asterisk — that is Claude's own mark.
            icon: || Icon::new(UbiqIcon::ModeAgents),
            ui_id: uid::RAIL_MODE_AGENTS,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            opens_left: true,
            opens_right: false,
            centre: Some(|app, window, cx| {
                crate::ui::agents::render(app, window, cx).into_any_element()
            }),
            default_layout: None,
            destination: Some(|app, _project, cx| {
                let agents = app.agents(cx)?;
                Some(View::Agents {
                    agent: agents.columns.get(agents.focus)?.active_agent()?,
                })
            }),
            furniture: Some(|app| app.queue_furniture(PanelKind::AgentsExplorer)),
            side_furniture: Some(|region| {
                (region == Region::Left).then_some(PanelKind::AgentsExplorer)
            }),
            on_enter: None,
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_TEAMS,
            group: ids::RAIL_PROJECT,
            label: "Teams",
            note: "How the agents are arranged, and which task each serves.",
            slug: "teams",
            icon: || Icon::new(UbiqIcon::ModeTeams),
            ui_id: uid::RAIL_MODE_TEAMS,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            opens_left: false,
            // Teams' right is the inspector its own screen draws inline rather than a dockable
            // panel, so it names no side furniture.
            opens_right: false,
            centre: Some(|app, window, cx| {
                crate::ui::teams::render(app, window, cx).into_any_element()
            }),
            default_layout: None,
            destination: Some(teams_destination),
            furniture: None,
            side_furniture: None,
            on_enter: None,
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_TEAMS_OLD,
            group: ids::RAIL_PROJECT,
            label: "[Teams]",
            note: "How the agents are arranged, and which task each serves.",
            slug: "teamsold",
            icon: || Icon::new(UbiqIcon::ModeTeams),
            ui_id: uid::RAIL_MODE_TEAMS_OLD,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            opens_left: false,
            opens_right: false,
            centre: Some(|app, window, cx| {
                crate::ui::orchestration::render(app, window, cx).into_any_element()
            }),
            default_layout: None,
            destination: Some(|app, _project, cx| {
                let graph = app.graph(cx)?;
                Some(View::Graph {
                    selection: graph.selection?,
                    tab: graph.tab,
                })
            }),
            furniture: None,
            side_furniture: None,
            on_enter: None,
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_KB,
            group: ids::RAIL_PROJECT,
            label: "KB",
            note: "Notes and documents the agents can read.",
            slug: "kb",
            icon: || Icon::new(UbiqIcon::ModeKb),
            ui_id: uid::RAIL_MODE_KB,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            opens_left: true,
            opens_right: false,
            centre: Some(|app, window, cx| crate::ui::kb::centre(app, window, cx)),
            default_layout: Some(crate::ui::dock::default_kb_layout),
            destination: Some(|_app, _project, _cx| Some(View::Kb)),
            furniture: Some(AppState::queue_kb_furniture),
            side_furniture: Some(|region| {
                (region == Region::Left).then_some(PanelKind::KbExplorer)
            }),
            // The configuration behind a blank explorer is asked for on arrival rather than on
            // every frame.
            on_enter: Some(AppState::ask_kb_sources_on_arrival),
        },
    );
    register(
        reg,
        RailModeSpec {
            id: ids::RAIL_TASKS,
            group: ids::RAIL_PROJECT,
            label: "Tasks",
            note: "Work queued for the agents in this session.",
            slug: "tasks",
            icon: || Icon::new(UbiqIcon::ModeTasks),
            ui_id: uid::RAIL_MODE_TASKS,
            availability: Availability::Always,
            has_pane_region: true,
            needs_project: true,
            opens_left: false,
            // The board claims the right for the task being read.
            opens_right: true,
            centre: Some(|app, window, cx| {
                crate::ui::board::render(app, window, cx).into_any_element()
            }),
            default_layout: None,
            destination: Some(|app, _project, cx| {
                Some(View::Tasks {
                    task: app.board(cx)?.selected?,
                })
            }),
            furniture: Some(|app| app.queue_furniture(PanelKind::Task)),
            side_furniture: Some(|region| (region == Region::Right).then_some(PanelKind::Task)),
            on_enter: None,
        },
    );
}

/// Teams and All Teams are the same screen over two spans, so they are the same destination.
///
/// A teams link names the project of what it points at, which under the window span is not the
/// project on screen: the same agent, read on a canvas showing one project and on one showing six,
/// is the same agent, and a link built here has to be followable by a window in the project span.
/// A session selection names no agent, so it asks the sibling that resolves a session.
fn teams_destination(app: &AppState, project: &mut ProjectId, cx: &App) -> Option<View> {
    let teams = app.teams(cx)?;
    let selection = teams.selection.clone()?;
    let tab = teams.tab;
    let owner = match &selection {
        TeamsSelection::Session(session) => app.project_of_session(*session, cx),
        TeamsSelection::Agent(agent) => app.project_of_agent(*agent, cx),
        TeamsSelection::Subagent { agent, .. } => app.project_of_agent(*agent, cx),
        // A mission's record names the project it was minted in, which is the one answer a fence
        // on a window-span canvas can be read back through.
        TeamsSelection::Mission(task) => {
            app.mission_anywhere(*task).map(|record| record.project_id)
        }
    };
    if let Some(owner) = owner {
        *project = owner;
    }
    Some(View::Teams { selection, tab })
}

/// The mark: the project's colour and the logo, sitting in the titlebar row above the rail so the
/// two read as one column. The logo is the pale file on a dark swatch and the blue one on a light
/// swatch, so it stays legible on whatever the project is tinted.
pub fn mark(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let (tint, white) = match app.project_snapshot(cx) {
        Some(project) => {
            let tint = theme::project_tint(
                project.record.temporary,
                project.record.colour,
                project.record.custom_colour,
            );
            (tint, theme::mark_dark(tint))
        }
        None => (theme::border(), theme::mark_dark(theme::border())),
    };
    let logo = Arc::new(Image::from_bytes(
        ImageFormat::Png,
        (if white { LOGO_WHITE } else { LOGO_BLUE }).to_vec(),
    ));
    div()
        .w(px(theme::rail_width()))
        .h(px(theme::titlebar_height()))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(tint)
        .border_r_1()
        // The bottom edge is the titlebar's own line carried across, so the swatch ends where the
        // project chip beside it ends.
        .border_b_1()
        .border_color(theme::border())
        .child(img(ImageSource::Image(logo)).size(px(theme::titlebar_height() - 10.)))
        .ui_id(ui_id::RAIL_MARK)
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let active = app.workbench.rail_mode;

    let mut groups = Vec::new();
    for (label, modes) in RailMode::groups() {
        let mut items = Vec::new();
        for mode in modes {
            if !app.mode_enabled(mode, cx) {
                continue;
            }
            items.push(rail_item(mode, mode == active, cx));
        }
        // A group whose every mode is hidden takes its heading with it.
        if items.is_empty() {
            continue;
        }
        groups.push(
            div()
                .w(px(item_width()))
                .flex()
                .flex_col()
                .items_center()
                .pb_3()
                .child(
                    div()
                        .h(px(GROUP_HEIGHT - 12.))
                        .pt_3()
                        .child(section_label(label)),
                )
                .children(items),
        );
    }

    // The modes come first, always: the badges take whatever whole ones are left over, and none
    // when the window is too short for even one. `project_badges` asks `project_badge_capacity`
    // the same question again rather than being handed the answer, so it stays the single place
    // that decides which badges are on screen.
    div()
        .w(px(theme::rail_width()))
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .ui_id(ui_id::RAIL)
        .child(
            // A thin wrapper, matching the rail's own flex settings, so the whole column of
            // groups can carry one name — `.children` would otherwise splice `groups` in as
            // siblings with nothing to mark.
            div()
                .flex()
                .flex_col()
                .items_center()
                .ui_id(ui_id::RAIL_MODES)
                .children(groups),
        )
        .child(div().flex_1().min_h(px(0.)))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .pb(px(1.))
                .ui_id(ui_id::RAIL_PROJECTS)
                .children(project_badges(app, window, cx)),
        )
}

/// How many project badges fit under the rail's mode list, for the window's current height — the
/// same arithmetic `render` builds `groups` with. Its own function so the digit shortcuts in
/// `app::projects::activate_project_slot` can ask the identical question without building any
/// elements.
fn project_badge_capacity(app: &AppState, window: &Window, cx: &App) -> usize {
    let spent: f32 = RailMode::groups()
        .into_iter()
        .map(|(_, modes)| {
            modes
                .into_iter()
                .filter(|mode| app.mode_enabled(*mode, cx))
                .count()
        })
        .filter(|count| *count > 0)
        .map(|count| GROUP_HEIGHT + ITEM_HEIGHT * count as f32)
        .sum();
    let room = f32::from(window.viewport_size().height)
        - theme::titlebar_height()
        - theme::status_bar_height();
    // The last badge keeps a pixel off the bottom edge as well.
    ((room - spent - 1.) / badge_height()).floor().max(0.) as usize
}

/// The project ids `project_badges` draws, in that order — after the same least-recently-opened
/// trim. Shared with `app::projects::activate_project_slot` so `cmd-1`..`cmd-9` always lands on
/// the project the matching badge shows, including when the window is too short to show them all.
pub fn visible_project_order(app: &AppState, window: &Window, cx: &App) -> Vec<ProjectId> {
    if !app.workbench.settings.ui.rail_projects {
        return Vec::new();
    }
    let fits = project_badge_capacity(app, window, cx);
    if fits == 0 {
        return Vec::new();
    }
    // One project is nothing to pick between, so the badge would only repeat the mark above it.
    let Some(slot) = app.window_slot(cx).filter(|slot| slot.projects.len() > 1) else {
        return Vec::new();
    };
    let registry = WindowRegistry::read(cx);
    let active = slot.active_project();
    // Which ones to keep is a question of recency; where they go is not. The active project is the
    // most recent by definition, so it is the badge that survives when there is room for only one.
    let mut keep: Vec<_> = slot.projects.clone();
    keep.sort_by_key(|id| {
        (
            Some(*id) == active,
            registry.project(*id).map(|p| p.record.last_opened_at),
        )
    });
    keep.drain(..slot.projects.len().saturating_sub(fits));

    slot.projects
        .iter()
        .copied()
        .filter(|id| keep.contains(id))
        .collect()
}

/// The projects this window holds, at the bottom of the rail.
///
/// A reminder rather than a picker: the badge is the project's colour and its initial, the name is
/// the tooltip, and the one the window is pointed at wears a ring inside its own edge. Off by
/// default is not offered — the switch lives in appearance settings, and off means this returns
/// nothing at all.
///
/// **The order never moves.** Badges are drawn in the order the window holds them, whatever the
/// rail has room for; a shortage drops the least recently opened, so the ones that remain stay
/// where the user last saw them.
fn project_badges(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> Vec<AnyElement> {
    let order = visible_project_order(app, window, cx);
    if order.is_empty() {
        return Vec::new();
    }
    let active = app.window_slot(cx).and_then(|slot| slot.active_project());

    order
        .iter()
        // The colour and the characters are `project_face`'s, shared with the chip a Teams card
        // wears under the window span — one resolution, so a project cannot read two ways.
        .filter_map(|id| Some((*id, project_face(*id, cx)?)))
        .map(|(id, face)| {
            let initial = face.initials;
            let tint = face.tint;
            let tooltip = face.name;
            let selected = Some(id) == active;
            div()
                .id(ElementId::Name(format!("rail-project-{id}").into()))
                // The window's own project is the full square, edge to edge; the rest are the
                // same square drawn as a thick ring, inset so the two never read as one block.
                .m(px(if selected { 0. } else { BADGE_MARGIN }))
                .size(px(if selected {
                    badge_height()
                } else {
                    badge_height() - BADGE_MARGIN * 2.
                }))
                .when(!selected, |this| {
                    this.border(px(BADGE_MARGIN)).border_color(tint)
                })
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .when(selected, |this| this.bg(tint))
                .text_color(if selected { theme::on_accent() } else { tint })
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Title))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .cursor_pointer()
                .child(initial)
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.activate_project(id, cx)))
                .into_any_element()
        })
        .collect()
}

fn rail_item(mode: RailMode, active: bool, cx: &mut Context<AppState>) -> AnyElement {
    let (fg, bg) = if active {
        (theme::accent(), theme::accent_soft())
    } else {
        (theme::text_muted(), theme::pane_bg())
    };

    div()
        .id(ElementId::Name(
            format!("rail-{}", mode.label().to_lowercase()).into(),
        ))
        .w(px(item_width()))
        .h(px(ITEM_HEIGHT))
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .gap_1()
        .bg(bg)
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(mode_icon(mode))
                .with_size(Size::Medium)
                .text_color(fg),
        )
        .child(
            div()
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Micro))
                .text_color(fg)
                .child(SharedString::from(mode.label())),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.set_rail_mode(mode, cx)))
        .ui_id(ui_id::rail_mode(mode))
        .into_any_element()
}
