//! The skills and MCP servers catalog's app-level wiring: a `Catalog` replaces its layer whole,
//! the forms that offer skills and servers ask for both layers as they open, and the MCP form
//! writes its record, waits for the layer, and fills itself from what the host read of a paste.
//! `tests/new_mission.rs`'s `Fixture` is the model this one follows.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::catalog::{CatalogMcp, McpKind, SkillInfo, SkillOriginInfo};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

const PATIENCE: Duration = Duration::from_millis(300);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
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
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
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
            host,
            project,
        }
    }

    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>) -> R,
    ) -> R {
        let out = self
            .window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| f(state, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        out
    }

    fn layer(
        &self,
        scope: Option<ProjectId>,
        skills: &[&str],
        mcps: &[&str],
        cx: &mut TestAppContext,
    ) {
        self.deliver(
            Message::Catalog {
                scope,
                skills: skills.iter().map(|id| a_skill(id)).collect(),
                mcps: mcps.iter().map(|id| a_server(id)).collect(),
                skill_folders: Vec::new(),
                sources: Vec::new(),
            },
            cx,
        );
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
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn a_skill(id: &str) -> SkillInfo {
    SkillInfo {
        id: id.to_string(),
        name: None,
        description: None,
        origin: SkillOriginInfo::Installed { url: None },
    }
}

fn a_server(id: &str) -> CatalogMcp {
    CatalogMcp {
        id: id.to_string(),
        kind: McpKind::Stdio,
        command: Some("npx".to_string()),
        args: vec!["-y".to_string(), "pkg".to_string()],
        env: BTreeMap::new(),
        url: None,
        headers: BTreeMap::new(),
        description: None,
    }
}

#[gpui::test]
fn a_catalog_message_replaces_its_layer_whole(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.layer(None, &["pdf", "docx"], &["files"], cx);
    fixture.layer(None, &["docx"], &[], cx);
    fixture.layer(Some(fixture.project), &["local"], &[], cx);

    fixture.state.read_with(cx, |state, _| {
        let catalog = &state.workbench.settings.catalog;
        let ids = |skills: &[SkillInfo]| skills.iter().map(|it| it.id.clone()).collect::<Vec<_>>();
        assert_eq!(
            ids(&catalog.global.skills),
            ["docx"],
            "the second replaced the first"
        );
        assert!(catalog.global.mcps.is_empty());
        assert_eq!(
            ids(&catalog.layer(Some(fixture.project)).unwrap().skills),
            ["local"],
            "a project's layer is held apart from the application's"
        );
    });
}

#[gpui::test]
fn the_start_dialog_asks_for_both_layers_and_offers_them_merged(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.with(cx, |state, window, cx| state.open_new_agent(Default::default(), window, cx));
    let asked: Vec<Option<ProjectId>> = fixture
        .said()
        .into_iter()
        .filter_map(|message| match message {
            Message::ListCatalog { scope } => Some(scope),
            _ => None,
        })
        .collect();
    assert!(asked.contains(&None), "the application's layer");
    assert!(
        asked.contains(&Some(fixture.project)),
        "the window's project's layer"
    );

    fixture.layer(None, &["pdf", "shared"], &["files"], cx);
    fixture.layer(
        Some(fixture.project),
        &["shared", "own"],
        &["files", "db"],
        cx,
    );
    fixture.state.update(cx, |state, cx| {
        let skills = state.form_skills(cx);
        let ids: Vec<&str> = skills.iter().map(|it| it.id.as_str()).collect();
        assert_eq!(
            ids,
            ["shared", "own", "pdf"],
            "the project's first, and shadowing by id"
        );
        assert_eq!(state.form_catalog_mcps(cx).len(), 2, "files is not doubled");

        state.toggle_new_agent_skill("own".to_string(), cx);
        state.toggle_new_agent_mcp("db".to_string(), cx);
        let form = state.new_agent_form().expect("the form is up");
        assert_eq!(form.skills, ["own"]);
        assert!(
            form.wants_mcp("db"),
            "a catalog server is ticked like a built-in one"
        );
    });
}

#[gpui::test]
fn saving_the_mcp_form_writes_the_record_and_waits_for_the_layer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| {
        state.open_mcp_form(None, None, window, cx)
    });
    // Drawn once, which writes the fields a fill left waiting.
    fixture.with(cx, |state, window, cx| {
        for (input, text) in [
            (&state.catalog_inputs.mcp_id, "files"),
            (&state.catalog_inputs.mcp_command, "npx"),
        ] {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        state
            .catalog_inputs
            .mcp_args
            .update(cx, |input, cx| input.set_value("-y\npkg", window, cx));
    });
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| state.save_mcp_form(cx));
    let sent = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SaveCatalogMcp {
                scope,
                mcp,
                previous_id,
            } => Some((scope, *mcp, previous_id)),
            _ => None,
        })
        .expect("Save wrote the record");
    assert_eq!(sent.0, None);
    assert_eq!(sent.1, a_server("files"));
    assert_eq!(sent.2, None);
    fixture.state.read_with(cx, |state, _| {
        assert!(
            state.workbench.settings.catalog.form.is_some(),
            "the form waits for the host's answer"
        );
    });

    // A refusal keeps the form up with the host's sentence on it.
    fixture.deliver(
        Message::CatalogError {
            error: "no".to_string(),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let catalog = &state.workbench.settings.catalog;
        assert_eq!(catalog.error.as_deref(), Some("no"));
        assert_eq!(
            catalog.form.as_ref().and_then(|it| it.error.as_deref()),
            Some("no")
        );
    });

    // The layer arriving is what says the write landed.
    fixture.with(cx, |state, _, cx| state.save_mcp_form(cx));
    fixture.layer(None, &[], &["files"], cx);
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.settings.catalog.form.is_none());
        assert_eq!(state.workbench.settings.catalog.global.mcps.len(), 1);
    });
}

#[gpui::test]
fn a_parsed_paste_fills_the_form_and_several_are_offered(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| {
        state.open_mcp_form(Some(fixture.project), None, window, cx)
    });

    fixture.deliver(
        Message::McpConfigParsed {
            servers: vec![a_server("one")],
            error: None,
        },
        cx,
    );
    // The next frame writes the fields.
    fixture.with(cx, |_, _, _| {});
    fixture.state.read_with(cx, |state, cx| {
        let record = state.mcp_form_record(cx).expect("the form is up");
        assert_eq!(record, a_server("one"), "one server fills the form");
    });

    fixture.deliver(
        Message::McpConfigParsed {
            servers: vec![a_server("a"), a_server("b")],
            error: None,
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let form = state.workbench.settings.catalog.form.as_ref().unwrap();
        assert_eq!(form.picks.len(), 2, "several are offered to pick from");
    });
    fixture
        .state
        .update(cx, |state, cx| state.use_mcp_pick(1, cx));
    fixture.with(cx, |_, _, _| {});
    fixture.state.read_with(cx, |state, cx| {
        assert_eq!(state.mcp_form_record(cx).unwrap().id, "b");
        assert!(
            state
                .workbench
                .settings
                .catalog
                .form
                .as_ref()
                .unwrap()
                .picks
                .is_empty()
        );
    });

    // Unreadable text is the host's sentence on the form.
    fixture.deliver(
        Message::McpConfigParsed {
            servers: Vec::new(),
            error: Some("not JSON".to_string()),
        },
        cx,
    );
    fixture.state.read_with(cx, |state, _| {
        let form = state.workbench.settings.catalog.form.as_ref().unwrap();
        assert_eq!(form.error.as_deref(), Some("not JSON"));
    });
}

#[gpui::test]
fn the_catalog_modals_close_with_escape_one_layer_at_a_time(cx: &mut TestAppContext) {
    use ubiq::app::DialogCancel;
    cx.update(ubiq::app::install_key_bindings);
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, window, cx| {
        state.workbench.settings.open = true;
        state.open_mcp_form(None, None, window, cx);
        state.open_mcp_registry(None, window, cx);
    });
    let escape = |cx: &mut TestAppContext| {
        fixture.with(cx, |state, window, cx| {
            state.cancel_dialog(&DialogCancel, window, cx)
        })
    };
    escape(cx);
    fixture.state.read_with(cx, |state, _| {
        let catalog = &state.workbench.settings.catalog;
        assert!(catalog.registry.is_none(), "the search goes first");
        assert!(
            catalog.form.is_some(),
            "and leaves the form it was raised over"
        );
    });
    escape(cx);
    fixture.state.read_with(cx, |state, _| {
        assert!(state.workbench.settings.catalog.form.is_none());
        assert!(
            state.workbench.settings.open,
            "the form did not take settings with it"
        );
    });
}
