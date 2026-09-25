//! Switching rail modes restores that mode's arrangement.
//!
//! Each mode keeps its own record of which regions were on screen and its own dock blob. Coming
//! back to a mode must put the side panels back where they were, whatever non-IDE mode the window
//! sat in meanwhile. That is hiding-not-removing: a non-IDE mode leaves the IDE's explorer — and
//! any chat tab that has claimed the right region — in the tree, shut, and returning to IDE opens
//! them again. A project with nothing attached has no chat panel to hide: the IDE mints none, so
//! the explorer is the side panel this fixture has.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use gpui_component::dock::InsertTarget;
use ubiq::app::{AppState, BusHub};
use ubiq::state::RailMode;
use ubiq::state::WindowRegistry;
use ubiq::state::dock::Region;
use ubiq::state::editor::ViewLayout;
use ubiq::state::prefs;
use ubiq::ui::dock::placement_of;
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot, Scope};

/// A window on one project, with a bus nobody answers on. Mirrors `panel_reentrancy`'s fixture.
struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    project: ProjectId,
    host: bus::HostEnd,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        Self::start(true, cx)
    }

    /// A window pointed at nothing — the state Ubiq opens in before a project is picked, and the
    /// one the log console, help and the rail are all usable in.
    fn open_without_project(cx: &mut TestAppContext) -> Self {
        Self::start(false, cx)
    }

    fn start(pointed: bool, cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let pointed_at = pointed.then_some(project);
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(pointed_at, 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            window,
            project,
            host,
        }
    }

    /// Do something to the state that needs a window — every gesture the titlebar makes does.
    fn gesture(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>),
    ) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| act(state, window, cx));
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// A click on one of the titlebar's region switches.
    fn toggle_region(&self, region: Region, cx: &mut TestAppContext) {
        self.gesture(cx, |state, window, cx| {
            state.toggle_region(region, window, cx)
        });
    }

    fn reveal_console(&self, cx: &mut TestAppContext) {
        self.gesture(cx, |state, window, cx| state.reveal_console(window, cx));
    }

    fn reveal_search(&self, cx: &mut TestAppContext) {
        self.gesture(cx, |state, window, cx| state.reveal_search(window, cx));
    }

    fn reveal_help(&self, cx: &mut TestAppContext) {
        self.gesture(cx, |state, window, cx| state.reveal_help(window, cx));
    }

    /// Drag the one panel a region holds into the tab group another region already has — the
    /// mouse gesture, made through the same tree edit the drop performs.
    fn drag_panel(&self, from: Region, to: Region, cx: &mut TestAppContext) {
        self.gesture(cx, move |state, window, cx| {
            state.dock().clone().update(cx, |dock, cx| {
                let panel = dock
                    .layout(placement_of(from))
                    .and_then(|tree| tree.panels().next())
                    .expect("the region dragged from holds a panel");
                let node = dock
                    .layout(placement_of(to))
                    .and_then(|tree| {
                        let target = tree.panels().next()?;
                        tree.find_panel_node(target)
                    })
                    .expect("the region dropped into has a group to join");
                dock.move_panel(
                    panel,
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                );
            });
        });
    }

    /// Put a second project in this window's hands, the way `clipboard`'s fixture does.
    fn also_holding(&self, cx: &mut TestAppContext) -> ProjectId {
        let snapshot = a_project();
        let id = snapshot.record.id;
        cx.update(|cx| {
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });
        self.gesture(cx, move |state, _, cx| state.take_project(id, cx));
        id
    }

    /// Point the window at a project it already holds.
    fn activate(&self, project: ProjectId, cx: &mut TestAppContext) {
        self.gesture(cx, move |state, _, cx| state.activate_project(project, cx));
    }

    /// Every leaf panel in the tree, by permanent name.
    fn panels(&self, cx: &mut TestAppContext) -> Vec<String> {
        names(&self.dump(cx))
    }

    fn holds(&self, name: &str, cx: &mut TestAppContext) -> bool {
        self.panels(cx).iter().any(|leaf| leaf == name)
    }

    /// Say something to every window, the way the host does.
    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(bus::To::Everyone, message);
        cx.run_until_parked();
    }

    fn switch_to(&self, mode: RailMode, cx: &mut TestAppContext) {
        let state = self.state.clone();
        state.update(cx, |state, cx| state.set_rail_mode(mode, cx));
        cx.run_until_parked();
    }

    fn mode(&self, cx: &mut TestAppContext) -> RailMode {
        self.state
            .read_with(cx, |state, _| state.workbench.rail_mode)
    }

    fn regions_open(&self, cx: &mut TestAppContext) -> (bool, bool, bool) {
        self.state.read_with(cx, |state, cx| state.regions_open(cx))
    }

    /// The full dump of the window's arrangement, as `remember_view` would write it.
    fn dump(&self, cx: &mut TestAppContext) -> String {
        self.state.read_with(cx, |state, cx| {
            serde_json::to_string(&state.dock().read(cx).dump(cx)).unwrap_or_default()
        })
    }

    /// The gesture that opens a file in the centre, and the frame that puts its panel in the dock —
    /// mirrors `panel_reentrancy`'s fixture.
    fn open_file(&self, path: &str, cx: &mut TestAppContext) {
        let path = path.to_string();
        self.state
            .update(cx, |state, cx| state.select_file(path, cx));
        cx.run_until_parked();
    }

    /// Which tab the editor says is active, for the project on screen.
    fn active_file(&self, cx: &mut TestAppContext) -> Option<String> {
        self.state.read_with(cx, |state, cx| {
            state
                .editor(cx)
                .and_then(|editor| editor.active_file())
                .map(|file| file.key())
        })
    }

    /// The layout a given open file's viewer is currently in.
    fn file_layout(&self, key: &str, cx: &mut TestAppContext) -> Option<ViewLayout> {
        self.state
            .read_with(cx, |state, cx| state.file(key, cx).map(|file| file.layout))
    }

    /// Put a file's viewer into a layout, the way its toggle does.
    fn set_layout(&self, key: &str, layout: ViewLayout, cx: &mut TestAppContext) {
        let key = key.to_string();
        self.gesture(cx, move |state, _, cx| {
            state.set_view_layout(&key, layout, cx)
        });
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
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
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// Every leaf panel's permanent name, in the order it appears in a dump.
fn names(blob: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_names(&serde_json::from_str(blob).unwrap_or_default(), &mut out);
    out
}

fn collect_names(value: &serde_json::Value, into: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(name) = map.get("panel_name").and_then(|n| n.as_str()) {
                into.push(name.to_string());
            }
            for child in map.values() {
                collect_names(child, into);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_names(item, into);
            }
        }
        _ => {}
    }
}

/// Leaving IDE for another mode hides its side panels — they stay in the tree, shut — and coming
/// back restores whatever regions IDE was left with. That is the invariant behind the restore, and
/// it must hold for any non-IDE mode: a project mode (Tasks) and a non-project one (Control) both
/// hide, neither removes.
///
/// **The round trip has to have something to lose, or it is satisfied by a restore that restores
/// nothing** (`G223`). The IDE's own left region is open from the start — the explorer is how the
/// files are reached and is never closable (P4) — and the console is revealed into the bottom
/// here so two of the three regions, and a panel that is the window's rather than the mode's, are
/// on the line. The console is swept on the way out (`D156`) and comes back only because the IDE's
/// blob named it, which is the contract this fixture is worth testing at all.
#[gpui::test]
fn returning_from_any_non_ide_mode_restores_the_side_panels(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.mode(cx), RailMode::Ide);
    assert_eq!(
        fixture.regions_open(cx),
        (true, false, false),
        "a first visit to the IDE with no blob opens the explorer's left region (P4)"
    );
    assert!(
        fixture.holds("ubiq.explorer", cx),
        "and puts the explorer in it: {:?}",
        fixture.panels(cx)
    );

    fixture.reveal_console(cx);
    assert_eq!(
        fixture.regions_open(cx),
        (true, true, false),
        "the console's reveal is what opens the bottom"
    );

    for mode in [RailMode::Tasks, RailMode::Control] {
        // Leave IDE: the side panels go, but nothing is removed from the arrangement.
        let before = fixture.dump(cx);
        let regions_before = fixture.regions_open(cx);
        assert_eq!(
            regions_before,
            (true, true, false),
            "the IDE is left with two regions on screen, so the round trip has something to lose"
        );
        fixture.switch_to(mode, cx);
        assert_eq!(fixture.mode(cx), mode);
        let in_non_ide = names(&fixture.dump(cx));
        assert!(
            in_non_ide.contains(&"ubiq.explorer".to_string()),
            "{mode:?} hides the side panels in place, it does not remove them: {in_non_ide:?}"
        );
        assert!(
            !in_non_ide.contains(&"ubiq.chat".to_string()),
            "an agent panel attached to nothing is never in the tree: {in_non_ide:?}"
        );
        assert!(
            !in_non_ide.contains(&"ubiq.logs".to_string()),
            "the console is the window's furniture and does not follow the switch into {mode:?} \
             (D156): {in_non_ide:?}"
        );

        // Back to IDE: the arrangement is restored whole, side panels and the three regions.
        fixture.switch_to(RailMode::Ide, cx);
        assert_eq!(fixture.mode(cx), RailMode::Ide);
        let after = fixture.dump(cx);
        assert_eq!(
            names(&after),
            names(&before),
            "coming back from {mode:?} restores the arrangement left behind — the console \
             included, because the IDE's own blob named it"
        );
        assert_eq!(
            fixture.regions_open(cx),
            regions_before,
            "coming back from {mode:?} restores the regions IDE was left with"
        );
        assert_eq!(
            fixture.state.read_with(cx, |state, cx| state.project(cx)),
            Some(fixture.project),
            "the window is still pointed at the project it opened on"
        );
    }
}

/// **Search and the log are asked for, never inherited** (`D156`). Both are opened by a gesture,
/// for one project on one screen; a panel that followed the switch would be one the next thing the
/// dock says writes down as the mode being entered. The mode they were open in named them in its
/// own blob, and that restore is the only way either comes back.
#[gpui::test]
fn search_and_the_log_do_not_follow_a_mode_switch(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.reveal_search(cx);
    fixture.reveal_console(cx);
    assert!(fixture.holds("ubiq.search", cx));
    assert!(fixture.holds("ubiq.logs", cx));

    fixture.switch_to(RailMode::Tasks, cx);
    let in_tasks = fixture.panels(cx);
    assert!(
        !in_tasks.contains(&"ubiq.search".to_string()),
        "search does not follow the mode switch: {in_tasks:?}"
    );
    assert!(
        !in_tasks.contains(&"ubiq.logs".to_string()),
        "neither does the log: {in_tasks:?}"
    );

    // And the blob the IDE was left in is what brings them both back.
    fixture.switch_to(RailMode::Ide, cx);
    let home = fixture.panels(cx);
    assert!(
        home.contains(&"ubiq.search".to_string()),
        "the IDE's own blob named search, so returning restores it: {home:?}"
    );
    assert!(
        home.contains(&"ubiq.logs".to_string()),
        "and the log with it: {home:?}"
    );
}

/// Help is the one piece of window furniture that travels, and only in follow mode — the panel
/// whose whole contract is to keep up with where the reader is standing (`D156`). With follow off
/// it is swept like search and the log.
///
/// **Asserted on the regions, not on the leaf names** (`G223`). A panel is in the tree whether or
/// not anything draws it, so `holds` passes for help that followed the switch into a region the
/// incoming arrangement has shut — which is help off screen, and is what follow mode is for. The
/// knowledge base is the mode this goes through because its right region is shut both times: on
/// the first visit by its defaults, and on the second by the blob the first left behind.
#[gpui::test]
fn help_follows_a_mode_switch_only_when_it_is_asked_to(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    // Follow off, which is how the panel opens.
    fixture.reveal_help(cx);
    assert!(fixture.holds("ubiq.help", cx));
    assert!(
        fixture.regions_open(cx).2,
        "revealing help is what opens the right region"
    );

    fixture.switch_to(RailMode::Kb, cx);
    assert!(
        !fixture.holds("ubiq.help", cx),
        "with follow off, help is the mode's and stays behind: {:?}",
        fixture.panels(cx)
    );
    assert!(
        !fixture.regions_open(cx).2,
        "the knowledge base wears its own right region, which is shut"
    );

    // Follow on, and the same switch carries it — on screen, which is the whole of the contract.
    fixture.switch_to(RailMode::Ide, cx);
    fixture.reveal_help(cx);
    fixture
        .state
        .update(cx, |state, cx| state.toggle_help_follow(cx));
    cx.run_until_parked();
    assert!(fixture.holds("ubiq.help", cx));
    assert!(fixture.regions_open(cx).2);

    fixture.switch_to(RailMode::Kb, cx);
    assert!(
        fixture.holds("ubiq.help", cx),
        "following the reader is what follow mode means: {:?}",
        fixture.panels(cx)
    );
    assert!(
        fixture.regions_open(cx).2,
        "and following means on screen: the knowledge base's own blob has this region shut, so \
         help carried into it as a leftover would be in the tree and invisible: {:?}",
        fixture.panels(cx)
    );
}

/// **A window pointed at no project sweeps nothing** (`D156`). The sweep's whole justification is
/// that the mode being left named its furniture in its own blob and the restore puts it back —
/// and with no project there is no blob, nothing is written down, and nothing is restored. The
/// rail still draws every mode and the log console is still usable, so a switch that took the
/// console away would take it away for good.
#[gpui::test]
fn a_projectless_window_keeps_its_furniture_across_a_mode_switch(cx: &mut TestAppContext) {
    let fixture = Fixture::open_without_project(cx);
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state.project(cx)),
        None,
        "this window is pointed at nothing"
    );

    fixture.reveal_console(cx);
    fixture.reveal_help(cx);
    assert!(fixture.holds("ubiq.logs", cx));
    assert!(fixture.holds("ubiq.help", cx));
    let (_, bottom, right) = fixture.regions_open(cx);
    assert!(bottom && right, "both were revealed onto their own edges");

    for mode in [RailMode::Control, RailMode::Sink, RailMode::Ide] {
        fixture.switch_to(mode, cx);
        assert!(
            fixture.holds("ubiq.logs", cx),
            "the console survives the switch into {mode:?}, because nothing could put it back: \
             {:?}",
            fixture.panels(cx)
        );
        assert!(
            fixture.holds("ubiq.help", cx),
            "and so does help, follow mode or not: {:?}",
            fixture.panels(cx)
        );
        assert_eq!(
            (fixture.regions_open(cx).1, fixture.regions_open(cx).2),
            (true, true),
            "and both stay on screen in {mode:?}"
        );
    }
}

/// **A chat tab is placed by the user, never by a mode switch** (`D156`) — the rule search and the
/// log answer to, one kind along. A conversation is drawn in every mode about a project, so a tab
/// open in the IDE is a leftover in Git; adding it there would put a chat beside the changes panel
/// that nobody opened, and the dock's own `LayoutChanged` would write it into Git's blob.
///
/// The IDE's own blob is what brings it back, and the panel is kept for exactly that: a chat id is
/// minted fresh every process, so a tab whose panel this window has dropped can never be rebuilt
/// from a saved leaf.
#[gpui::test]
fn a_mode_switch_places_no_chat_tab(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // Git has to have a blob of its own, or the switch keeps the tree rather than restoring one
    // and there is no leftover loop to get wrong.
    fixture.switch_to(RailMode::Git, cx);
    fixture.switch_to(RailMode::Ide, cx);

    // The user's own click on the right switch is what opens a chat tab there.
    fixture.toggle_region(Region::Right, cx);
    assert!(
        fixture.holds("ubiq.chat", cx),
        "the switch opens the right region onto a fresh chat tab: {:?}",
        fixture.panels(cx)
    );

    fixture.switch_to(RailMode::Git, cx);
    let in_git = fixture.panels(cx);
    assert!(
        !in_git.contains(&"ubiq.chat".to_string()),
        "the chat the user opened in the IDE is not placed in Git: {in_git:?}"
    );
    assert!(
        in_git.contains(&"ubiq.git.changes".to_string()),
        "Git's own right-hand panel is what is there instead: {in_git:?}"
    );

    fixture.switch_to(RailMode::Ide, cx);
    assert!(
        fixture.holds("ubiq.chat", cx),
        "and the IDE's blob is what brings it back where the user left it: {:?}",
        fixture.panels(cx)
    );
    assert!(
        fixture.regions_open(cx).2,
        "in the region the IDE was left with on screen"
    );
}

/// A region switch fills an empty edge with the mode's own furniture — **unless that furniture is
/// already on screen somewhere else**. The explorer dragged to the right is where it was put, and
/// `settle_panels` skips a kind the tree already holds, so asking for it would leave an open left
/// bar with nothing in it that the dock writes down as the user's own arrangement.
#[gpui::test]
fn the_left_switch_opens_no_empty_bar_when_the_explorer_is_elsewhere(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.regions_open(cx), (true, false, false));

    // Somewhere to drag it to: the right switch opens its region onto a fresh chat tab.
    fixture.toggle_region(Region::Right, cx);
    fixture.drag_panel(Region::Left, Region::Right, cx);
    assert_eq!(
        fixture.regions_open(cx),
        (false, false, true),
        "the left region is emptied by the drag and put away"
    );
    assert!(fixture.holds("ubiq.explorer", cx));

    fixture.toggle_region(Region::Left, cx);
    assert!(
        !fixture.regions_open(cx).0,
        "the explorer is on the other edge, so the left switch has nothing to show and leaves \
         the arrangement as it found it: {:?}",
        fixture.panels(cx)
    );
    assert!(
        fixture.holds("ubiq.explorer", cx),
        "and where the user dragged it is where it stays: {:?}",
        fixture.panels(cx)
    );
}

/// A region the user put away in a mode is still away when that mode comes back. The mode's own
/// blob is what says so, and nothing on the way in overrules it with the mode's defaults.
#[gpui::test]
fn a_region_closed_in_a_mode_is_still_closed_on_the_way_back(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.regions_open(cx), (true, false, false));

    fixture.toggle_region(Region::Left, cx);
    assert_eq!(
        fixture.regions_open(cx),
        (false, false, false),
        "the switch puts the explorer's region away"
    );

    fixture.switch_to(RailMode::Tasks, cx);
    fixture.switch_to(RailMode::Ide, cx);
    assert_eq!(
        fixture.regions_open(cx),
        (false, false, false),
        "the IDE comes back as it was left, not as its defaults would have opened it"
    );
}

/// **An open, empty side region is closed rather than filled** (`D156`). The window used to top one
/// up with whatever furniture the mode had; now only the region switch does that, because a click
/// on it is a user gesture and a mode switch is not.
///
/// Control is the mode with no side furniture at all, so its left region opened is an empty one,
/// and what it is written down wearing is what makes the return trip say something.
#[gpui::test]
fn an_open_and_empty_side_region_is_closed_rather_than_filled(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.switch_to(RailMode::Control, cx);
    assert_eq!(fixture.regions_open(cx), (false, false, false));

    fixture.toggle_region(Region::Left, cx);
    assert!(
        fixture.regions_open(cx).0,
        "the user's own click opens the edge, whatever is or is not in it"
    );

    fixture.switch_to(RailMode::Ide, cx);
    fixture.switch_to(RailMode::Control, cx);
    assert!(
        !fixture.regions_open(cx).0,
        "coming back, the empty edge is put away rather than topped up with furniture"
    );
}

/// **A project switch opens no region** (`D156`). The incoming project wears its own mode's
/// arrangement and nothing else: neither the regions the project just left had open, nor an edge
/// made by putting a leftover panel back — a leftover is added, never revealed.
#[gpui::test]
fn a_project_switch_opens_no_region(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.reveal_search(cx);
    assert_eq!(
        fixture.regions_open(cx),
        (true, true, false),
        "the first project is left with its explorer and the search panel on screen"
    );

    let second = fixture.also_holding(cx);
    fixture.activate(second, cx);

    assert_eq!(
        fixture.regions_open(cx),
        (true, false, false),
        "the incoming project wears its own mode's regions — the IDE's explorer, and nothing the \
         project just left had open"
    );
    assert!(
        !fixture.holds("ubiq.search", cx),
        "search is one project's on one screen and does not follow the switch: {:?}",
        fixture.panels(cx)
    );
}

/// Git's first visit opens the left and right regions onto the refs explorer and the changes
/// panel. They are the screen, not furniture: a mode's edges are its own, and a return to IDE
/// restores whatever that mode was left with — here its explorer's left region and nothing else.
#[gpui::test]
fn git_opens_with_its_side_panels(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.regions_open(cx), (true, false, false));

    fixture.switch_to(RailMode::Git, cx);
    assert_eq!(fixture.mode(cx), RailMode::Git);
    assert_eq!(
        fixture.regions_open(cx),
        (true, false, true),
        "Git's refs and changes open with the mode"
    );
    let in_git = names(&fixture.dump(cx));
    assert!(
        in_git.contains(&"ubiq.git.refs".to_string()),
        "the refs explorer is in the left region: {in_git:?}"
    );
    assert!(
        in_git.contains(&"ubiq.git.changes".to_string()),
        "the changes panel is in the right region: {in_git:?}"
    );
    assert!(
        in_git.contains(&"ubiq.git.history".to_string()),
        "the commit list is the centre: {in_git:?}"
    );

    fixture.switch_to(RailMode::Ide, cx);
    assert_eq!(
        fixture.regions_open(cx),
        (true, false, false),
        "coming back to IDE restores the regions it was left with"
    );
}

/// A project whose stored view says Git opens on Git's own screen, side panels and all.
///
/// The window opens on the IDE's default tree and learns which mode the project was left in only
/// when the host answers with its blob — so the answer has to apply that mode's defaults the way
/// a switch does, or a start in Git would wear the IDE's regions and none of Git's panels.
#[gpui::test]
fn a_project_stored_in_git_starts_with_its_side_panels(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.mode(cx), RailMode::Ide);

    let view = prefs::ViewPrefs {
        rail_mode: RailMode::Git,
        ..prefs::ViewPrefs::default()
    };
    fixture.deliver(
        Message::Preferences {
            scope: Scope::Project(fixture.project),
            value: Some(prefs::encode(&view)),
        },
        cx,
    );

    assert_eq!(fixture.mode(cx), RailMode::Git);
    assert_eq!(
        fixture.regions_open(cx),
        (true, false, true),
        "Git's refs and changes are on screen, the pane region shut"
    );
    let on_screen = names(&fixture.dump(cx));
    for panel in ["ubiq.git.refs", "ubiq.git.changes", "ubiq.git.history"] {
        assert!(
            on_screen.contains(&panel.to_string()),
            "{panel} is in the tree a start in Git opens on: {on_screen:?}"
        );
    }
}

/// Hiding a mode takes it off the rail; hiding the mode the window is in moves the window on, and
/// the last visible mode cannot be hidden at all.
#[gpui::test]
fn hiding_modes_never_empties_the_rail(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let state = fixture.state.clone();

    state.update(cx, |state, cx| {
        assert!(state.mode_enabled(RailMode::Git, cx));
        state.toggle_mode(RailMode::Git, cx);
        assert!(!state.mode_enabled(RailMode::Git, cx));

        // The window is in IDE: hiding it has to leave the window somewhere else.
        state.toggle_mode(RailMode::Ide, cx);
        assert!(!state.mode_enabled(RailMode::Ide, cx));
        assert!(state.mode_enabled(state.workbench.rail_mode, cx));

        // Everything but the last one goes; the last one stays whatever is asked.
        for mode in RailMode::every() {
            if state.mode_enabled(mode, cx) {
                state.toggle_mode(mode, cx);
            }
        }
        let left: Vec<_> = RailMode::every()
            .filter(|mode| state.mode_enabled(*mode, cx))
            .collect();
        assert_eq!(left.len(), 1, "one mode always survives: {left:?}");
    });
    cx.run_until_parked();
}

/// T-196: a document open in the IDE — which file is active and what layout its viewer is in — is
/// exactly the kind of state a mode switch must not disturb.
///
/// This is not new machinery: `OpenFile` lives on the project (`OpenProject::editor`), not on the
/// mode, so it is never rebuilt by a rail-mode switch — only the dock's *tree* is, and that tree is
/// what `ModeLayout::layout` already remembers per mode (`file_payload` writes the tab's key and its
/// `ViewLayout` into the same blob `returning_from_any_non_ide_mode_restores_the_side_panels`
/// exercises for regions). This fixture is the round trip for the document half of that same
/// mechanism: open two files, put one in `Preview`, leave IDE for another mode entirely and come
/// back, and both which file was in front and which layout it was left in must still be true.
#[gpui::test]
fn a_document_and_its_view_mode_survive_a_trip_away_from_ide(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(fixture.mode(cx), RailMode::Ide);

    fixture.open_file("README.md", cx);
    fixture.open_file("notes.md", cx);
    let active = fixture
        .active_file(cx)
        .expect("the most recently opened file is in front");
    assert!(
        active.ends_with("notes.md"),
        "the second file opened is the one in front: {active}"
    );

    // Put the front file's viewer into `Preview` — a real toggle, not a default: the markdown
    // default here is `Preview` already (see `ViewLayout::default`), so prove the round trip on
    // `Source` instead, which nothing defaults to.
    fixture.set_layout(&active, ViewLayout::Source, cx);
    assert_eq!(fixture.file_layout(&active, cx), Some(ViewLayout::Source));

    // Leave the IDE for a mode with nothing to do with files, and come back.
    fixture.switch_to(RailMode::Agents, cx);
    assert_ne!(fixture.mode(cx), RailMode::Ide);
    fixture.switch_to(RailMode::Ide, cx);
    assert_eq!(fixture.mode(cx), RailMode::Ide);

    assert_eq!(
        fixture.active_file(cx).as_deref(),
        Some(active.as_str()),
        "the file that was in front is still in front"
    );
    assert_eq!(
        fixture.file_layout(&active, cx),
        Some(ViewLayout::Source),
        "the layout the viewer was left in is still the one it is in"
    );
}
