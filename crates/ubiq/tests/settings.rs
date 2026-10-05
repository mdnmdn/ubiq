//! The interface-owned settings blob: the interface versions it, so a schema it does not know is
//! discarded rather than half-applied.
//!
//! The Host layer is a different animal — the host owns its schema, not this file — so its round
//! trip is asserted over the bus rather than over `decode`/`encode`: a toggle writes `SetSettings`
//! on the Host layer, and the checkbox shows whatever the host answers, not a local default.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::input::InputEvent;
use ubiq::app::{AppState, BusHub};
use ubiq::state::editor::{OpenFile, ViewLayout, ViewerKind};
use ubiq::state::layout::Algo;
use ubiq::state::nav::{Destination, Locus, View};
use ubiq::state::new_agent::Target;
use ubiq::state::prefs;
use ubiq::state::settings::{self, MarkdownOpen, TabClose, UiSettings};
use ubiq::state::{HarnessChoice, WindowRegistry};
use ubiq::ui::new_agent::target_control;
use ubiq_proto::assist::{
    AiProvider, AiProviderInfo, AiProviderKind, AssistProvider, ModelRole, SuggestSubject,
};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{AiProviderId, ProjectId, SuggestId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot, Scope};
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};

#[test]
fn a_blob_survives_the_round_trip() {
    let settings = UiSettings {
        schema: settings::SCHEMA,
        explorer_preview: false,
        explorer_hidden: true,
        capture_enabled: false,
        rail_projects: false,
        markdown_open: MarkdownOpen::Source,
        terminal_close: TabClose::Hide,
        agent_terminal_close: TabClose::Hide,
        agent_chat_close: TabClose::Close,
        vim_mode: true,
        show_cache_ring: true,
        last_connection: Some("01J0".to_string()),
        teams_algo: Algo::default(),
        md_minimap: false,
        md_minimap_side: ubiq::theme::MdMinimapSide::Right,
        md_width: ubiq::theme::MdWidth::Wide,
        md_density: ubiq::theme::MdDensity::Compact,
        md_char_scale_default: 1.25,
        md_text_shade_default: ubiq::state::editor::TextShade::Strong,
        md_line_spacing_default: 1.2,
        md_paragraph_spacing_default: 1.5,
        acp_enabled: ["claude-code-acp".to_string()].into_iter().collect(),
    };
    let back = settings::decode(&settings::encode(&settings)).expect("decodes");
    assert_eq!(back, settings);
}

#[test]
fn missing_fields_open_on_defaults() {
    let blob = r#"{"schema":1}"#;
    let back = settings::decode(blob).expect("decodes");
    assert!(back.explorer_preview);
    // A blob written before the dotfile switch existed opens with dotfiles hidden.
    assert!(!back.explorer_hidden);
    assert!(back.capture_enabled);
    assert!(back.rail_projects);
    assert_eq!(back.markdown_open, MarkdownOpen::Preview);
    // A blob written before the × had a setting opens on the defaults: a terminal's × ends the
    // pane, a chat tab's only puts the view away.
    assert_eq!(back.terminal_close, TabClose::Close);
    assert_eq!(back.agent_terminal_close, TabClose::Close);
    assert_eq!(back.agent_chat_close, TabClose::Hide);
    // A blob written before vim mode existed opens with it off, rather than being discarded.
    assert!(!back.vim_mode);
    assert!(!back.show_cache_ring);
    // A blob written before the markdown minimap existed opens with it shown, on the left.
    assert!(back.md_minimap);
    assert_eq!(back.md_minimap_side, ubiq::theme::MdMinimapSide::Left);
    // A blob written before the width/density popover existed opens on Readable/Comfortable.
    assert_eq!(back.md_width, ubiq::theme::MdWidth::Readable);
    assert_eq!(back.md_density, ubiq::theme::MdDensity::Comfortable);
    // A blob written before the ACP switches existed opens with every one of them off — the
    // native wire is the better one (`D95`), so the second id for a tool is opt-in and an absent
    // field is the same answer as an unticked box.
    assert!(back.acp_enabled.is_empty());
}

#[test]
fn a_newer_schema_is_discarded() {
    let blob = r#"{"schema":99,"explorer_preview":false}"#;
    assert!(settings::decode(blob).is_none());
}

#[test]
fn unreadable_is_discarded() {
    assert!(settings::decode("not json").is_none());
}

#[test]
fn markdown_open_picks_the_layout_a_new_tab_starts_in() {
    let preview = OpenFile::opening(
        "README.md",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    assert_eq!(preview.viewer, ViewerKind::Markdown);
    assert_eq!(preview.layout, ViewLayout::Preview);

    let source = OpenFile::opening(
        "README.md",
        ubiq::state::editor::Subject::File,
        ViewLayout::Source,
    );
    assert_eq!(source.layout, ViewLayout::Source);

    // A mermaid file still opens in preview: the setting is markdown's.
    let mermaid = OpenFile::opening(
        "flow.mmd",
        ubiq::state::editor::Subject::File,
        ViewLayout::Source,
    );
    assert_eq!(mermaid.layout, ViewLayout::Preview);

    // Plain text has no preview.
    let rust = OpenFile::opening(
        "main.rs",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    assert_eq!(rust.layout, ViewLayout::Source);
}

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

/// A window on one project, with the host end kept so a test can both answer it and read what the
/// window said. Mirrors `new_pane`'s fixture.
struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<gpui_component::Root>,
    host: bus::HostEnd,
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
            gpui_component::Root::new(state, window, cx)
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
        }
    }

    /// Run `f` against the window's own state, with a `Window` in hand.
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

    /// Everything the window has said so far, in order.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
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

/// The overlay's first write to the Host layer: flipping the checkbox does not touch `UiSettings`
/// at all, because the host is forbidden to parse that blob (`D46`). It writes `SetSettings` on
/// the Host layer instead, carrying the flag it just flipped.
#[gpui::test]
fn flipping_the_isolation_toggle_writes_the_host_layer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // Drain the startup `GetSettings` asks before watching for the toggle's own write.
    let _ = fixture.said();

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.isolate_agents),
        "isolation defaults to on"
    );

    fixture
        .state
        .update(cx, |state, cx| state.toggle_isolate_agents(cx));
    cx.run_until_parked();

    let written = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Host,
                value,
            } => Some(value),
            _ => None,
        })
        .expect("the toggle wrote the host layer");
    let sent: HostSettings = serde_json::from_str(&written).expect("a readable blob");
    assert!(
        !sent.isolate_agents,
        "the toggle flipped the flag it sent, not just the flag it holds"
    );
    assert_eq!(
        sent.schema, HOST_SETTINGS_SCHEMA,
        "the write is stamped with this build's host schema"
    );

    assert!(
        !fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.isolate_agents),
        "the toggle's own state matches what it just sent"
    );
}

/// Every Appearance control persists as it is flipped: there is no apply button, so each of the
/// four theme axes writes the interface blob on the click that changed it.
#[gpui::test]
fn flipping_an_appearance_control_writes_the_interface_blob(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.state.update(cx, |state, cx| {
        state.set_palette(ubiq::theme::ThemeId("ember-dark"), cx);
        state.set_accent(Some(ubiq::theme::AccentId("teal")), cx);
        state.set_ui_scale(0.9, cx);
        state.set_text_ratio(1.1, cx);
        state.set_trim(ubiq::theme::Family::Conversation, 1.2, cx);
    });
    // A size change settles before it is written: re-dressing every emulator emits a
    // `TerminalResize` per pane, so a slider drag must not send two hundred of them. The clock has
    // to be walked past the debounce for the blob to land.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();

    let written = fixture
        .said()
        .into_iter()
        .filter_map(|message| match message {
            Message::SetPreferences {
                scope: Scope::Interface,
                value,
            } => Some(value),
            _ => None,
        })
        .next_back()
        .expect("the flips wrote the interface blob");
    let sent: prefs::InterfacePrefs = prefs::decode(&written).expect("a readable blob");

    assert_eq!(sent.theme.slug(), "ember-dark");
    assert_eq!(sent.accent, Some(ubiq::theme::AccentId("teal")));
    assert_eq!(sent.ui_scale, 0.9);
    assert_eq!(sent.text_ratio, 1.1);
    assert_eq!(sent.conversation_trim, 1.2);
}

/// Naming is on by default and that default costs nothing, because it runs through the provider
/// setting — which is off until somebody picks one. The pair is what makes an upgrade silent.
#[gpui::test]
fn the_naming_toggle_defaults_on_and_writes_the_host_layer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    let (naming, assist) = fixture.state.read_with(cx, |state, _| {
        (
            state.workbench.settings.host.auto_name_conversations,
            state.workbench.settings.host.assist.clone(),
        )
    });
    assert!(naming, "naming defaults to on");
    assert_eq!(
        assist,
        AssistProvider::Off,
        "and calls nothing, because no provider is picked until a user picks one",
    );

    fixture
        .state
        .update(cx, |state, cx| state.toggle_auto_name_conversations(cx));
    cx.run_until_parked();

    let written = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Host,
                value,
            } => Some(value),
            _ => None,
        })
        .expect("the toggle wrote the host layer");
    let sent: HostSettings = serde_json::from_str(&written).expect("a readable blob");
    assert!(
        !sent.auto_name_conversations,
        "the toggle flipped the flag it sent, not just the flag it holds"
    );
    assert_eq!(sent.schema, HOST_SETTINGS_SCHEMA);
}

/// The checkbox shows what the host actually has stored, not this build's default: a `Message::
/// Settings` for the Host layer decodes into the same state the toggle reads.
#[gpui::test]
fn the_hosts_answer_is_what_the_checkbox_shows(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    let stored = HostSettings {
        schema: HOST_SETTINGS_SCHEMA,
        isolate_agents: false,
        ..HostSettings::default()
    };
    fixture.host.send(
        To::Everyone,
        Message::Settings {
            layer: SettingsLayer::Host,
            value: Some(serde_json::to_string(&stored).expect("HostSettings serialises")),
        },
    );
    cx.run_until_parked();

    assert!(
        !fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.isolate_agents),
        "the host's answer is what the checkbox shows, not the default it opened on"
    );
}

/// The two search lists are text fields holding a comma-separated line. They commit on Enter —
/// and on blur — rather than on a keystroke, because every commit makes the host write a file.
#[gpui::test]
fn committing_the_search_lists_writes_the_parsed_lists(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.type_into(
        |state| state.search_excludes_input.clone(),
        "vendor, dist ,,",
        cx,
    );
    fixture.type_into(
        |state| state.search_fallbacks_input.clone(),
        " grep ,ag,",
        cx,
    );

    let sent = fixture
        .said()
        .into_iter()
        .filter_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Host,
                value,
            } => serde_json::from_str::<HostSettings>(&value).ok(),
            _ => None,
        })
        .next_back()
        .expect("both commits wrote the host layer");

    assert_eq!(sent.search_excludes, vec!["vendor", "dist"]);
    assert_eq!(sent.search_fallbacks, vec!["grep", "ag"]);
    assert_eq!(sent.schema, HOST_SETTINGS_SCHEMA);

    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.clone())
            .search_excludes,
        vec!["vendor", "dist"],
        "the window holds what it sent"
    );
}

impl Fixture {
    /// Type into one of the window's fields and commit it, which is what Enter does.
    fn type_into(
        &self,
        pick: impl Fn(&AppState) -> Entity<gpui_component::input::InputState>,
        text: &str,
        cx: &mut TestAppContext,
    ) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    let input = pick(state);
                    input.update(cx, |field, cx| {
                        field.set_value(text, window, cx);
                        cx.emit(InputEvent::PressEnter {
                            shift: false,
                            secondary: false,
                        });
                    });
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// The layout a markdown tab is showing, `None` if it is not open at all.
    fn layout_of(&self, key: &str, cx: &mut TestAppContext) -> Option<ViewLayout> {
        self.state.read_with(cx, |state, cx| {
            state
                .editor(cx)
                .and_then(|editor| editor.open.get(editor.index_of_key(key)?))
                .map(|file| file.layout)
        })
    }
}

/// A destination that names a line is a fact about the bytes, which a preview draws none of: it
/// turns a markdown tab's source half on no matter the open-as setting. An anchor is left alone,
/// because a heading slug is exactly what the preview does draw.
#[gpui::test]
fn a_line_locus_turns_a_preview_markdown_tab_to_source(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture
        .state
        .read_with(cx, |state, cx| state.project(cx))
        .expect("the fixture opens on a project");

    fixture.state.update(cx, |state, cx| {
        state.select_file("README.md".to_string(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.layout_of("README.md", cx),
        Some(ViewLayout::Preview),
        "markdown opens on the default open-as setting"
    );

    fixture.state.update(cx, |state, cx| {
        state.navigate(
            Destination {
                project,
                view: View::Ide {
                    key: "README.md".to_string(),
                },
                locus: Some(Locus::Line { line: 3 }),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.layout_of("README.md", cx),
        Some(ViewLayout::Source),
        "a line names bytes the preview draws none of, so the source half turns on regardless \
         of the open-as setting"
    );

    // Back to preview, then an anchor — a heading slug the preview draws itself — leaves it be.
    fixture.state.update(cx, |state, cx| {
        state.set_view_layout("README.md", ViewLayout::Preview, cx)
    });
    fixture.state.update(cx, |state, cx| {
        state.navigate(
            Destination {
                project,
                view: View::Ide {
                    key: "README.md".to_string(),
                },
                locus: Some(Locus::Anchor {
                    slug: "intro".to_string(),
                }),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.layout_of("README.md", cx),
        Some(ViewLayout::Preview),
        "an anchor is drawn by the preview itself, so it is left alone"
    );
}

// ── API providers ───────────────────────────────────────────────────
//
// Providers are the one part of the host record the interface never writes through `SetSettings`:
// a record and the key filed under its id go together, and only the host can write the key. So
// what these assert is the four provider messages and the list the host broadcasts back.

/// One configured provider, as the host would say it holds one. `has_key` is true because the host
/// writes no record whose key it could not file — a row on screen always has a key behind it.
fn a_provider(id: AiProviderId) -> AiProviderInfo {
    AiProviderInfo {
        provider: AiProvider {
            id,
            kind: AiProviderKind::OpenAiCompatible,
            name: "local".to_string(),
            base_url: Some("http://127.0.0.1:11434/v1".to_string()),
            fast_model: "qwen3:4b".to_string(),
            smart_model: None,
        },
        has_key: true,
    }
}

impl Fixture {
    /// Tell the window the host holds one provider, which is the only way a row or a pill for it
    /// exists: the interface reads no disk and invents no record.
    fn with_a_provider(&self, cx: &mut TestAppContext) -> AiProviderId {
        let provider_id = AiProviderId::generate();
        self.host.send(
            To::Everyone,
            Message::AiProviders {
                providers: vec![a_provider(provider_id)],
            },
        );
        cx.run_until_parked();
        provider_id
    }

    /// Run something that needs the window's own `Window` — every dialog that seeds a text field
    /// does. The same shape [`Fixture::type_into`] uses.
    fn in_window(
        &self,
        f: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>),
        cx: &mut TestAppContext,
    ) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| f(state, window, cx));
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// What the test modal is showing, if one is up.
    fn test_answer(&self, cx: &mut TestAppContext) -> Option<String> {
        self.state.read_with(cx, |state, _| {
            state
                .workbench
                .settings
                .ai_test
                .as_ref()
                .map(|test| test.answer.clone())
        })
    }
}

/// A configured provider is a backend pill beside Off and On-device, and the pill that lights is
/// the one matching `host.assist` — which is the comparison `assist_provider_choice` makes.
#[gpui::test]
fn a_configured_provider_joins_the_backend_pills(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();
    let provider_id = fixture.with_a_provider(cx);

    let held = fixture
        .state
        .read_with(cx, |state, _| state.workbench.settings.ai_providers.clone());
    assert_eq!(
        held.len(),
        1,
        "the pills are drawn from the host's own list"
    );
    assert_eq!(held[0].provider.name, "local");
    assert!(held[0].has_key);

    // Picking that pill is the existing provider-choice path with the provider's id.
    fixture.state.update(cx, |state, cx| {
        state.set_assist_provider(AssistProvider::Api { provider_id }, cx)
    });
    cx.run_until_parked();

    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.host.assist.clone()),
        AssistProvider::Api { provider_id },
        "the lit pill is the one the setting names"
    );

    let said = fixture.said();
    let written = said
        .iter()
        .find_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Host,
                value,
            } => serde_json::from_str::<HostSettings>(value).ok(),
            _ => None,
        })
        .expect("the choice wrote the host layer");
    assert_eq!(written.assist, AssistProvider::Api { provider_id });
    assert!(
        said.iter()
            .any(|message| matches!(message, Message::GetAssist)),
        "which backend is behind assistance is the host's answer, and it has just changed"
    );
}

/// The test modal draws the answer as it arrives: every chunk for the id it is holding is
/// appended, and a chunk for any other id has nowhere to be put.
#[gpui::test]
fn a_chunk_appends_to_the_test_and_a_stranger_is_discarded(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();
    let provider_id = fixture.with_a_provider(cx);

    fixture
        .state
        .update(cx, |state, cx| state.open_ai_test(provider_id, cx));
    fixture.state.update(cx, |state, cx| state.run_ai_test(cx));
    cx.run_until_parked();

    let suggest_id = fixture
        .state
        .read_with(cx, |state, _| {
            state
                .workbench
                .settings
                .ai_test
                .as_ref()
                .and_then(|test| test.suggest_id)
        })
        .expect("the run minted an id and kept it");

    // The subject names the provider and the role and carries no prompt: the prompt is the
    // host's, for this subject as for every other.
    let asked = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::Suggest {
                suggest_id,
                subject,
            } => Some((suggest_id, subject)),
            _ => None,
        })
        .expect("the run asked for a suggestion");
    assert_eq!(asked.0, suggest_id);
    assert_eq!(
        asked.1,
        SuggestSubject::ProviderCheck {
            provider_id,
            role: ModelRole::Fast
        }
    );

    for text in ["It ", "works"] {
        fixture.host.send(
            To::Everyone,
            Message::SuggestChunk {
                suggest_id,
                text: text.to_string(),
            },
        );
    }
    fixture.host.send(
        To::Everyone,
        Message::SuggestChunk {
            suggest_id: SuggestId::generate(),
            text: " not this".to_string(),
        },
    );
    cx.run_until_parked();

    assert_eq!(
        fixture.test_answer(cx).as_deref(),
        Some("It works"),
        "the chunks for the id in flight are drawn, and nothing else is"
    );

    // The whole answer ends the run. It is the chunks concatenated, so nothing on screen moves.
    fixture.host.send(
        To::Everyone,
        Message::Suggestion {
            suggest_id,
            text: "It works".to_string(),
        },
    );
    cx.run_until_parked();

    assert_eq!(fixture.test_answer(cx).as_deref(), Some("It works"));
    assert!(
        fixture.state.read_with(cx, |state, _| {
            state
                .workbench
                .settings
                .ai_test
                .as_ref()
                .is_some_and(|test| test.done && test.suggest_id.is_none())
        }),
        "the run is over, so there is nothing left to cancel"
    );
}

/// An edit asks for the cached model list, and saving it with the key box untouched sends
/// `key: None` — the only way to say "keep the key you already have", since the interface is
/// never sent one and so cannot send one back.
#[gpui::test]
fn an_edit_with_a_blank_key_keeps_the_stored_one(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();
    let provider_id = fixture.with_a_provider(cx);

    fixture.in_window(
        |state, window, cx| state.open_ai_form(Some(provider_id), window, cx),
        cx,
    );

    let asked = fixture.said();
    assert!(
        asked.iter().any(|message| matches!(
            message,
            Message::ListAiModels {
                provider_id: id,
                refresh: false
            } if *id == provider_id
        )),
        "an edit takes the host's cache, which costs no network"
    );

    fixture.in_window(|state, window, cx| state.save_ai_form(window, cx), cx);

    let (draft, key) = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::UpdateAiProvider {
                provider_id: id,
                draft,
                key,
            } if id == provider_id => Some((draft, key)),
            _ => None,
        })
        .expect("the save wrote the provider");
    assert!(
        key.is_none(),
        "a blank key box keeps the key the host already filed"
    );
    assert_eq!(draft.name, "local");
    assert_eq!(draft.kind, AiProviderKind::OpenAiCompatible);
    assert_eq!(draft.fast_model, "qwen3:4b");
    assert_eq!(draft.base_url.as_deref(), Some("http://127.0.0.1:11434/v1"));
    assert_eq!(draft.smart_model, None);

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.ai_form.is_none()),
        "the form closes optimistically; a refusal comes back as a banner"
    );
}

/// An add asks no model question at all, so Save is enabled on a name and a key alone and the
/// draft that goes out carries no model. That is a real record: the host writes it and then lists
/// what the provider can run, which is what fills the pickers Edit shows — so the interface asks
/// for no list of its own here.
#[gpui::test]
fn an_add_needs_only_a_name_and_a_key(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.in_window(|state, window, cx| state.open_ai_form(None, window, cx), cx);
    fixture.type_into(|state| state.ai_name_input.clone(), "local", cx);
    fixture.type_into(|state| state.ai_key_input.clone(), "sk-not-a-real-key", cx);
    let _ = fixture.said();

    fixture.in_window(|state, window, cx| state.save_ai_form(window, cx), cx);
    let said = fixture.said();

    let draft = said
        .iter()
        .find_map(|message| match message {
            Message::AddAiProvider { draft, .. } => Some(draft.clone()),
            _ => None,
        })
        .expect("a name and a key are enough to write a provider");
    assert_eq!(draft.name, "local");
    assert_eq!(draft.kind, AiProviderKind::OpenAiCompatible);
    assert!(
        draft.fast_model.is_empty(),
        "the add branch asks for no model, so it sends none"
    );
    assert_eq!(draft.smart_model, None);
    assert!(
        !said
            .iter()
            .any(|message| matches!(message, Message::ListAiModels { .. })),
        "the host lists a new provider's models itself; asking again would be a second call"
    );

    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.ai_form.is_none()),
        "the form closes optimistically; a refusal comes back as a banner"
    );
}

/// A provider whose models cannot be listed is still configurable, because the model is a field
/// and the picker only fills it. `GET {base}/models` is near-universal and not universal — Azure
/// OpenAI addresses deployments and wants an `api-version`, and a listing can fail at a proxy or
/// on a key scoped to inference — so a typed id has to reach the host with no cached list behind
/// it at all.
#[gpui::test]
fn a_typed_model_reaches_the_host_with_no_list_to_pick_from(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    // The model-less record the host now writes: a key is filed, nothing answered `ListAiModels`.
    let provider_id = AiProviderId::generate();
    let mut info = a_provider(provider_id);
    info.provider.fast_model = String::new();
    fixture.host.send(
        To::Everyone,
        Message::AiProviders {
            providers: vec![info],
        },
    );
    cx.run_until_parked();

    fixture.in_window(
        |state, window, cx| state.open_ai_form(Some(provider_id), window, cx),
        cx,
    );
    assert!(
        fixture
            .state
            .read_with(cx, |state, _| state.workbench.settings.ai_models.is_empty()),
        "no list was ever answered, so the picker has no row to offer"
    );

    fixture.type_into(|state| state.ai_fast_model_input.clone(), "gpt-4o-mine", cx);
    let _ = fixture.said();

    fixture.in_window(|state, window, cx| state.save_ai_form(window, cx), cx);

    let draft = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::UpdateAiProvider {
                provider_id: id,
                draft,
                ..
            } if id == provider_id => Some(draft),
            _ => None,
        })
        .expect("the typed model was sent");
    assert_eq!(
        draft.fast_model, "gpt-4o-mine",
        "the field is the store, so what was typed is what travels"
    );
    assert_eq!(
        draft.smart_model, None,
        "an empty smart box is no smart model, not an empty one"
    );
}

/// The Drones row's uptime is read at the granularity a person reads a process's age at, not
/// formatted to the second.
#[test]
fn drone_uptime_reads_at_the_right_granularity() {
    assert_eq!(settings::drone_uptime(100, 130), "30s");
    assert_eq!(settings::drone_uptime(0, 125), "2m");
    assert_eq!(settings::drone_uptime(0, 3 * 3600 + 61 * 60), "4h 1m");
    assert_eq!(settings::drone_uptime(0, 2 * 86_400 + 3600), "2d 1h");
    // A clock skew or a stale `started_at` must not underflow and panic.
    assert_eq!(settings::drone_uptime(500, 100), "0s");
}

/// The one fact a version-skew row needs: whether the listed drone's own version matches this
/// build's. A managed drone outliving the Ubiq that deployed it is the expected case, not an
/// error — the row says so and names the remedy, this function only says whether to.
#[test]
fn drone_version_skew_compares_against_this_build() {
    assert!(!settings::drone_version_skew(env!("CARGO_PKG_VERSION")));
    assert!(settings::drone_version_skew(
        "0.0.1-definitely-not-this-build"
    ));
}

/// A definition the host answered with, in the scope it was found in.
fn a_definition(id: &str, project: Option<ProjectId>) -> ubiq_proto::messages::AgentDefinition {
    ubiq_proto::messages::AgentDefinition {
        id: id.to_string(),
        description: None,
        agent_type: "claude-code".to_string(),
        account: None,
        model: None,
        mode: None,
        thinking: None,
        max_subagents: None,
        prompt: None,
        mcps: Vec::new(),
        skills: Vec::new(),
        mission_assistant: None,
        mission_coordinator: false,
        mission_worker: false,
        disabled: false,
        project,
    }
}

/// A project's own definitions are offered in that project and listed nowhere else.
///
/// The two scopes ride in one `AgentDefinitions`, and the window splits them on arrival: the app-wide
/// settings screen draws `settings.definitions`, which stays global, and anything that means "what
/// can start here" asks `definitions_in`. A definition of another project is never on offer, and a
/// project definition of the same name as a global one shadows it — the rule the host resolves the
/// launch by, so the picker and the launch cannot disagree.
#[gpui::test]
fn project_definitions_are_offered_in_their_project_and_not_globally(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture
        .state
        .read_with(cx, |state, cx| state.project(cx))
        .expect("the window holds a project");
    let elsewhere = ProjectId::generate();

    fixture.host.send(
        To::Everyone,
        Message::AgentDefinitions {
            definitions: vec![
                a_definition("standard", None),
                a_definition("shared", None),
                a_definition("reviewer", Some(project)),
                a_definition("shared", Some(project)),
                a_definition("theirs", Some(elsewhere)),
            ],
        },
    );
    cx.run_until_parked();

    let (global, offered, outside, other) = fixture.state.read_with(cx, |state, _| {
        let settings = &state.workbench.settings;
        let ids = |list: Vec<ubiq_proto::messages::AgentDefinition>| {
            list.into_iter().map(|it| it.id).collect::<Vec<_>>()
        };
        (
            ids(settings.definitions.clone()),
            ids(settings.definitions_in(Some(project))),
            ids(settings.definitions_in(None)),
            ids(settings.definitions_in(Some(elsewhere))),
        )
    });

    assert_eq!(
        global,
        vec!["standard".to_string(), "shared".to_string()],
        "the global list is global: no project's definition is in it"
    );
    assert_eq!(
        offered,
        vec![
            "reviewer".to_string(),
            "shared".to_string(),
            "standard".to_string()
        ],
        "inside the project: its own, then the global ones it does not shadow"
    );
    assert_eq!(
        outside,
        vec!["standard".to_string(), "shared".to_string()],
        "with no project in hand, only the global ones"
    );
    assert!(
        !other.contains(&"reviewer".to_string()),
        "another project is offered none of this one's setups"
    );
}

/// A switched-off definition stays on the settings screens and is offered nowhere a run begins,
/// and `Clone` names the copy itself rather than sending one the host would refuse.
#[gpui::test]
fn a_disabled_definition_is_listed_and_not_offered(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture
        .state
        .read_with(cx, |state, cx| state.project(cx))
        .expect("the window holds a project");

    let mut off = a_definition("retired", None);
    off.disabled = true;
    fixture.host.send(
        To::Everyone,
        Message::AgentDefinitions {
            definitions: vec![a_definition("standard", None), off, {
                let mut copy = a_definition("standard copy", None);
                copy.account = Some("work".to_string());
                copy
            }],
        },
    );
    cx.run_until_parked();

    let (listed, startable, free) = fixture.state.read_with(cx, |state, _| {
        let settings = &state.workbench.settings;
        (
            settings.definitions.len(),
            settings
                .startable_definitions_in(Some(project))
                .into_iter()
                .map(|it| it.id)
                .collect::<Vec<_>>(),
            settings.free_definition_name("standard", None),
        )
    });

    assert_eq!(listed, 3, "every definition is still listed");
    assert_eq!(
        startable,
        vec!["standard".to_string(), "standard copy".to_string()],
        "the switched-off one is offered nowhere a run begins"
    );
    assert_eq!(
        free, "standard copy 2",
        "a clone never overwrites, so the interface picks a name that is free"
    );
}

/// Add definition inside a project writes a project definition; the settings screen's own writes a
/// global one. The scope is the surface the form was opened from, never a pick inside it — and
/// an edit keeps the scope its definition was found in.
#[gpui::test]
fn the_definition_form_carries_the_scope_it_was_opened_from(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let project = fixture
        .state
        .read_with(cx, |state, cx| state.project(cx))
        .expect("the window holds a project");

    fixture.with(cx, |state, window, cx| {
        state.open_definition_form(None, Some(project), window, cx)
    });
    let scoped = fixture.state.read_with(cx, |state, _| {
        let form = state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is up");
        form.as_definition("reviewer".to_string()).project
    });
    assert_eq!(
        scoped,
        Some(project),
        "a project's form writes a project definition"
    );

    fixture.with(cx, |state, window, cx| {
        state.open_definition_form(None, None, window, cx)
    });
    let global = fixture.state.read_with(cx, |state, _| {
        state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is up")
            .as_definition("standard".to_string())
            .project
    });
    assert_eq!(
        global, None,
        "the settings screen writes a global definition"
    );

    // Editing keeps the scope the definition was found in, whatever the surface passes.
    fixture.with(cx, |state, window, cx| {
        state.open_definition_form(
            Some(a_definition("reviewer", Some(project))),
            None,
            window,
            cx,
        )
    });
    let edited = fixture.state.read_with(cx, |state, _| {
        state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is up")
            .as_definition("reviewer".to_string())
            .project
    });
    assert_eq!(
        edited,
        Some(project),
        "an edit saves the definition back where it came from"
    );
}

/// A harness that signs into an account's own config home (`D194`).
fn a_shared_home_harness(id: &str, label: &str) -> ubiq_proto::messages::AgentTypeInfo {
    ubiq_proto::messages::AgentTypeInfo {
        id: id.to_string(),
        label: label.to_string(),
        command: id.to_string(),
        available: true,
        chat: true,
        acp: false,
        modes: Vec::new(),
        unattended_mode: None,
        keeps_sessions: true,
        quota: Default::default(),
        shares_home: true,
    }
}

/// The whole of the restored sign-in, end to end: the modal names the identity, the send carries
/// the harness and that name, and the account the host answers with is the one the block draws.
///
/// The point of the assertion on `Accounts`: under `D194` the account is *made* by a sign-in that
/// ends cleanly, so a flow that reported success and left nothing listed would be the bug.
#[gpui::test]
fn a_sign_in_that_ends_cleanly_leaves_the_account_listed(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![a_shared_home_harness("claude-code", "Claude Code")],
        },
    );
    cx.run_until_parked();
    let _ = fixture.said();

    fixture.with(cx, |state, window, cx| {
        state.open_harness_login(window, cx);
        state
            .login_account_input
            .update(cx, |field, cx| field.set_value("work", window, cx));
        state.pick_login_harness("claude-code".to_string(), window, cx);
        state.start_harness_login(cx);
    });

    let begun = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::BeginHarnessLogin {
                agent_type,
                account,
            } => Some((agent_type, account)),
            _ => None,
        })
        .expect("the sign-in was sent");
    assert_eq!(
        begun,
        ("claude-code".to_string(), "work".to_string()),
        "the send carries the harness picked and the name typed, and the name is never empty"
    );

    fixture.host.send(
        To::Everyone,
        Message::HarnessHomeSignedIn {
            agent_type: "claude-code".to_string(),
            account: "work".to_string(),
        },
    );
    fixture.host.send(
        To::Everyone,
        Message::Accounts {
            accounts: vec![ubiq_proto::messages::AccountInfo {
                id: "work".to_string(),
                logged_in: vec!["claude-code".to_string()],
            }],
        },
    );
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        let login = state
            .workbench
            .settings
            .login
            .as_ref()
            .expect("the modal stays up to say how it went");
        assert!(
            matches!(
                login.step,
                ubiq::state::settings::LoginStep::Done {
                    signed_in: true,
                    ..
                }
            ),
            "a clean end draws the signed-in outcome"
        );
        assert_eq!(
            state.workbench.settings.accounts_for("claude-code")[0].id,
            "work",
            "the account exists from that moment, and can start the harness it signed in to"
        );
    });
}

/// Signing one harness out leaves the identity and its other harnesses alone: the send names the
/// pair, and the shorter `logged_in` the host answers with is what the block draws.
#[gpui::test]
fn signing_out_shortens_logged_in(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::Accounts {
            accounts: vec![ubiq_proto::messages::AccountInfo {
                id: "work".to_string(),
                logged_in: vec!["claude-code".to_string(), "codex".to_string()],
            }],
        },
    );
    cx.run_until_parked();
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| {
        state.open_sign_out("codex".to_string(), "work".to_string(), cx);
        state.confirm_sign_out(cx);
    });

    let signed_out = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::DeleteHarnessLogin {
                agent_type,
                account,
            } => Some((agent_type, account)),
            _ => None,
        })
        .expect("the sign-out was sent");
    assert_eq!(
        signed_out,
        ("codex".to_string(), "work".to_string()),
        "the send names the one home to remove, not the account"
    );

    fixture.host.send(
        To::Everyone,
        Message::Accounts {
            accounts: vec![ubiq_proto::messages::AccountInfo {
                id: "work".to_string(),
                logged_in: vec!["claude-code".to_string()],
            }],
        },
    );
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            state.workbench.settings.accounts[0].logged_in,
            vec!["claude-code".to_string()],
            "the account survives, one harness shorter"
        );
        assert!(
            state.workbench.settings.accounts_for("codex").is_empty(),
            "and no picker offers it for the harness it signed out of"
        );
    });
}

/// A harness that is installed but that nobody has signed an account into is still offered — on
/// its own default configuration, which is what it runs on and the only answer it has.
///
/// Before `T-276` it appeared in no group at all: the list was built from `logged_in` alone, so an
/// installed tool with no account was unreachable from the New agent dialog. The row it gets is
/// `HarnessChoice::Harness`, under its own `Default` heading, **after** `Configured` — a signed-in
/// pairing stays the first thing offered.
///
/// `Default` carries *every* offered harness, not only the ones nobody signed into: running a
/// harness as whatever identity it would use itself is an answer it has whether or not accounts
/// exist for it, and withholding the row made that answer unreachable for a signed-in tool. So
/// `claude-code` is both a `Configured` pair and a `Default` row here; the two groups do not
/// partition the list.
#[gpui::test]
fn an_installed_harness_with_no_account_is_offered(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![
                a_shared_home_harness("claude-code", "Claude Code"),
                a_shared_home_harness("codex", "Codex"),
            ],
        },
    );
    fixture.host.send(
        To::Everyone,
        Message::Accounts {
            accounts: vec![ubiq_proto::messages::AccountInfo {
                id: "work".to_string(),
                logged_in: vec!["claude-code".to_string()],
            }],
        },
    );
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        let accounts = state.workbench.settings.accounts.clone();
        assert_eq!(
            state.workbench.harness_choices(&accounts, &[], None),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "work".to_string(),
                },
                HarnessChoice::Separator,
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
                HarnessChoice::Harness(1),
            ],
            "the signed-in pairing first, then every offered harness on its own configuration"
        );
    });
}

/// An ACP harness that is a second wire onto a tool with a native one is offered nowhere while its
/// switch is off, and offered like any other harness once it is on — and the switch is persisted
/// on the interface's own layer, the way every other UI setting is.
///
/// The gate is one question (`WorkbenchState::harness_offered`) asked by every surface that offers
/// a harness to run, so the list a frame draws and the list a pick resolves against cannot
/// disagree. The harness's **row on the Harnesses settings list is gated too** — reversing the
/// earlier call to keep it: `ui::settings::harness_list` draws only where
/// `!acp_sibling_gated`, so a gated sibling disappears from Installed as well and only its
/// `Enable <tool> ACP` switch still names it — see
/// [`installed_hides_a_gated_acp_sibling_and_shows_it_once_enabled`].
///
/// The four surfaces are `harness_choices` (the target picker, and `ui::new_agent::pair_rows`
/// behind `Customize`, both of which read nothing else), the sign-in pill picker — which filters
/// on `harness_offered` directly — `last_start_target`, and `can_write_definition`. The two UI row
/// builders are render functions and are asserted through the one list they are built from; what
/// `Add agent` does about all this is
/// [`a_definition_is_still_writable_beside_a_gated_sibling`] and
/// [`add_agent_is_unavailable_where_the_only_harness_is_gated`].
#[gpui::test]
fn an_acp_sibling_is_gated_out_of_every_selection_surface(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![a_shared_home_harness("claude-code", "Claude Code"), {
                let mut it = a_shared_home_harness("claude-code-acp", "Claude Code (ACP)");
                it.acp = true;
                it
            }],
        },
    );
    cx.run_until_parked();
    let _ = fixture.said();

    fixture.state.read_with(cx, |state, _| {
        assert!(
            state.workbench.settings.ui.acp_enabled.is_empty(),
            "off is the default, with no settings file having said so"
        );
        assert_eq!(
            state.workbench.harness_choices(&[], &[], None),
            vec![
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
            ],
            "the native wire is the only row; the ACP one is not even drawn disabled"
        );
        // The sign-in pill picker's own filter, and the one `last_start_target` asks before it
        // reopens a remembered start on a harness.
        assert!(
            !state
                .workbench
                .harness_offered(&state.workbench.agent_types[1]),
            "nothing that offers a harness to run may list it"
        );
        assert!(
            state
                .workbench
                .harness_offered(&state.workbench.agent_types[0]),
            "the native wire is offered everywhere"
        );
        assert_eq!(
            state
                .workbench
                .acp_siblings()
                .iter()
                .map(|it| it.id.clone())
                .collect::<Vec<_>>(),
            vec!["claude-code-acp".to_string()],
            "but it still has a row on the Harnesses list, which is where its switch is"
        );
    });

    fixture.state.update(cx, |state, cx| {
        state.toggle_acp_harness("claude-code-acp".to_string(), cx)
    });
    cx.run_until_parked();

    let written = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::SetSettings {
                layer: SettingsLayer::Ui,
                value,
            } => Some(value),
            _ => None,
        })
        .expect("the switch wrote the interface layer");
    let sent = settings::decode(&written).expect("a readable blob");
    assert!(
        sent.acp_enabled.contains("claude-code-acp"),
        "the switch persisted the harness it turned on, not just the flag it holds"
    );

    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            state.workbench.harness_choices(&[], &[], None),
            vec![
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
                HarnessChoice::Harness(1),
            ],
            "on, it is a harness like any other"
        );
        assert!(
            state
                .workbench
                .harness_offered(&state.workbench.agent_types[1]),
            "and the pill picker and the remembered start see it too"
        );
    });
}

/// The Harnesses settings page's `Installed` list draws no row for an ACP sibling whose switch is
/// off, and the row comes back the moment the switch is turned on.
///
/// `ui::settings::harness_list` is a private render function, so this asserts the exact predicate
/// it filters on (`!WorkbenchState::acp_sibling_gated`) rather than the rendered tree — the same
/// level every other test in this file works at, and the one the `ubiq-ui` skill asks for. The
/// switch itself is `acp_switches`, which draws one row per sibling regardless of its switch
/// (`WorkbenchState::acp_siblings`, asserted above) so that turning a hidden one back on stays
/// possible; that list is untouched by this change.
#[gpui::test]
fn installed_hides_a_gated_acp_sibling_and_shows_it_once_enabled(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![a_shared_home_harness("claude-code", "Claude Code"), {
                let mut it = a_shared_home_harness("claude-code-acp", "Claude Code (ACP)");
                it.acp = true;
                it
            }],
        },
    );
    cx.run_until_parked();
    let _ = fixture.said();

    let installed_rows = |state: &AppState| {
        state
            .workbench
            .agent_types
            .iter()
            .filter(|info| !state.workbench.acp_sibling_gated(info))
            .map(|info| info.id.clone())
            .collect::<Vec<_>>()
    };

    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            installed_rows(state),
            vec!["claude-code".to_string()],
            "off, the ACP sibling draws no row under Installed"
        );
        // Its switch still draws, whatever the row above does.
        assert_eq!(
            state
                .workbench
                .acp_siblings()
                .iter()
                .map(|it| it.id.clone())
                .collect::<Vec<_>>(),
            vec!["claude-code-acp".to_string()],
            "the Enable ... ACP switch keeps drawing so turning it back on stays possible"
        );
    });

    fixture.state.update(cx, |state, cx| {
        state.toggle_acp_harness("claude-code-acp".to_string(), cx)
    });
    cx.run_until_parked();
    let _ = fixture.said();

    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            installed_rows(state),
            vec!["claude-code".to_string(), "claude-code-acp".to_string()],
            "on, the row is back under Installed"
        );
    });
}

/// A definition already written against a gated ACP harness opens its form **on that harness**,
/// not on the placeholder.
///
/// Gating removes the harness from the list a new answer is chosen from, and keeping such a
/// definition working is the deliberate half of that. But the form's trigger is resolved by
/// finding its value among the rows it drew, so a value no row carried fell back to "Choose a
/// harness…" — the form reporting itself empty while `agent_type` still held the harness, and a
/// user filling the apparently-empty row silently re-targeting the definition. `harness_choices`
/// pins what the form holds under `Already chosen`, so draw and pick stay one list.
///
/// Asserted where the form stands: `ui::new_agent::target_control` is what the definition form's
/// harness control is drawn from — `pair_rows` is the *second* control, drawn only behind
/// `Customize` on a start — and what it says on the trigger is the thing that was wrong.
#[gpui::test]
fn a_definition_on_a_gated_harness_opens_on_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![a_shared_home_harness("claude-code", "Claude Code"), {
                let mut it = a_shared_home_harness("claude-code-acp", "Claude Code (ACP)");
                it.acp = true;
                it
            }],
        },
    );
    cx.run_until_parked();

    let mut definition = a_definition("reviewer", None);
    definition.agent_type = "claude-code-acp".to_string();
    fixture.with(cx, |state, window, cx| {
        state.open_definition_form(Some(definition), None, window, cx)
    });

    fixture.state.read_with(cx, |state, cx| {
        let form = state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is up");
        assert_eq!(
            form.agent_type, "claude-code-acp",
            "the form still holds what the definition named"
        );
        let (rows, trigger) = target_control(state, form, cx);
        assert_eq!(
            rows.iter()
                .map(|(label, _)| label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Default",
                "Claude Code",
                "",
                "Already chosen",
                "Claude Code (ACP)"
            ],
            "the gated harness is a row again, under a heading that says it is not on offer"
        );
        assert_eq!(
            trigger, "Claude Code (ACP)",
            "and the control names it instead of falling back to the placeholder"
        );
    });

    // Nothing is pinned twice: with the switch on, the harness is an ordinary row and the
    // `Already chosen` group does not appear beside it.
    fixture.state.update(cx, |state, cx| {
        state.toggle_acp_harness("claude-code-acp".to_string(), cx)
    });
    cx.run_until_parked();
    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            state
                .workbench
                .harness_choices(&[], &[], Some(("claude-code-acp", None))),
            vec![
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
                HarnessChoice::Harness(1),
            ],
            "a held answer the list already offers is not pinned a second time"
        );
    });
}

/// `Add agent` is drawn unavailable where the only installed harness is one no picker offers.
///
/// The predicate behind it answered "any harness is available", which a gated ACP sibling
/// satisfies — so the button stayed live and opened a form with an empty harness picker and
/// nothing saying why. It asks `harness_offered` now, the same question the pickers ask, and the
/// tooltip names the switch that is one section up on the screen the button is on.
#[gpui::test]
fn add_agent_is_unavailable_where_the_only_harness_is_gated(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![
                {
                    // Listed, so the ACP one is a *sibling* and gated, but not installed here.
                    let mut it = a_shared_home_harness("claude-code", "Claude Code");
                    it.available = false;
                    it
                },
                {
                    let mut it = a_shared_home_harness("claude-code-acp", "Claude Code (ACP)");
                    it.acp = true;
                    it
                },
            ],
        },
    );
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        assert!(
            !state.can_write_definition(),
            "the one installed harness is offered nowhere, so the form has nothing to name"
        );
    });

    fixture.state.update(cx, |state, cx| {
        state.toggle_acp_harness("claude-code-acp".to_string(), cx)
    });
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        assert!(
            state.can_write_definition(),
            "switched on, it is a harness a definition may name"
        );
    });
}

/// A definition naming a harness this machine has **not installed** opens on it too, and says so
/// on the control rather than on the placeholder (`G386`).
///
/// The row stays unpickable — "not installed" is what a disabled row says, and writing the
/// definition back onto a harness that is not there is not an answer worth offering — so the value
/// behind it is `None` and no amount of searching the rows' values will find it. The trigger is
/// resolved from what the *form* holds instead, which is the only thing that knows.
#[gpui::test]
fn a_definition_on_an_uninstalled_harness_names_it_on_the_trigger(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![a_shared_home_harness("codex", "Codex"), {
                let mut it = a_shared_home_harness("claude-code", "Claude Code");
                it.available = false;
                it
            }],
        },
    );
    cx.run_until_parked();

    fixture.with(cx, |state, window, cx| {
        state.open_definition_form(Some(a_definition("reviewer", None)), None, window, cx)
    });

    fixture.state.read_with(cx, |state, cx| {
        let form = state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is up");
        let (rows, trigger) = target_control(state, form, cx);
        assert_eq!(
            rows,
            vec![
                ("Default".to_string(), None),
                (
                    "Codex".to_string(),
                    Some(Target::Harness {
                        agent_type: "codex".to_string(),
                        account: None,
                    })
                ),
                ("Claude Code".to_string(), None),
            ],
            "the uninstalled harness is drawn, disabled, in the group it belongs to"
        );
        assert_eq!(
            trigger, "Claude Code",
            "and the control names what the form holds, unpickable row and all"
        );
    });
}

/// Picking the harness the form already holds changes nothing else.
///
/// It reads as a harmless click — the `Already chosen` row exists to be the one you already hold —
/// and it rebuilt the form from defaults: the model, the level, the mode, the subagent cap, the
/// MCPs, the skills, the prompt, the description, the scope and the switched-off flag all went,
/// and a definition form then saved that over the definition being edited. Picking a *different*
/// harness still drops them, which is the other half of the same guard.
#[gpui::test]
fn picking_the_harness_the_form_holds_leaves_every_other_answer_alone(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![
                a_shared_home_harness("claude-code", "Claude Code"),
                a_shared_home_harness("codex", "Codex"),
            ],
        },
    );
    cx.run_until_parked();

    let mut definition = a_definition("reviewer", None);
    definition.model = Some("opus".to_string());
    definition.thinking = Some("high".to_string());
    definition.mode = Some("plan".to_string());
    definition.max_subagents = Some(3);
    definition.mcps = vec!["files".to_string()];
    definition.skills = vec!["review".to_string()];
    definition.prompt = Some("Review the diff.".to_string());
    definition.description = Some("The reviewer.".to_string());
    definition.disabled = true;
    fixture.with(cx, |state, window, cx| {
        state.open_definition_form(Some(definition), None, window, cx)
    });

    let held = Target::Harness {
        agent_type: "claude-code".to_string(),
        account: None,
    };
    fixture.with(cx, |state, window, cx| {
        state.pick_new_agent_target(held.clone(), window, cx)
    });

    fixture.state.read_with(cx, |state, _| {
        let form = state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is still up");
        assert_eq!(
            form.target.as_ref(),
            Some(&held),
            "still on the same answer"
        );
        assert_eq!(form.model.as_deref(), Some("opus"));
        assert_eq!(form.thinking.as_deref(), Some("high"));
        assert_eq!(form.mode.as_deref(), Some("plan"));
        assert_eq!(form.max_subagents, Some(3));
        assert_eq!(form.mcps, vec!["files".to_string()]);
        assert_eq!(form.skills, vec!["review".to_string()]);
        assert_eq!(form.prompt, "Review the diff.");
        assert_eq!(form.description, "The reviewer.");
        assert!(form.disabled, "and it is still switched off");
    });

    // The other half: a different harness owns none of those answers, so they go.
    fixture.with(cx, |state, window, cx| {
        state.pick_new_agent_target(
            Target::Harness {
                agent_type: "codex".to_string(),
                account: None,
            },
            window,
            cx,
        )
    });
    fixture.state.read_with(cx, |state, _| {
        let form = state
            .workbench
            .settings
            .definition_form
            .as_ref()
            .expect("the form is still up");
        assert_eq!(form.agent_type, "codex");
        assert_eq!(
            form.model, None,
            "a model belongs to the harness that offered it"
        );
        assert_eq!(
            form.prompt, "",
            "and the form is a fresh answer to every other question"
        );
    });
}

/// A gated ACP sibling beside an installed native wire takes nothing away from `Add agent`.
///
/// The companion to [`add_agent_is_unavailable_where_the_only_harness_is_gated`]: gating is about
/// which harness may be *named*, and one that may be is enough.
#[gpui::test]
fn a_definition_is_still_writable_beside_a_gated_sibling(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![a_shared_home_harness("claude-code", "Claude Code"), {
                let mut it = a_shared_home_harness("claude-code-acp", "Claude Code (ACP)");
                it.acp = true;
                it
            }],
        },
    );
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        assert!(
            !state
                .workbench
                .harness_offered(&state.workbench.agent_types[1]),
            "the ACP wire is gated"
        );
        assert!(
            state.can_write_definition(),
            "a definition can still be written \u{2014} the native wire is installed"
        );
    });
}

/// `Add agent` is drawn unavailable where the only installed harness cannot hold a conversation,
/// and the sentence on it says that rather than naming a fix that does not exist.
///
/// `harness_offered` answers only the ACP question, so the predicate that asked it alone still
/// said yes for a harness with no structured bridge — `harness_choices` drops those, so the form
/// opened on an empty picker under a note telling the user to install a harness or switch on an
/// ACP wire, both impossible: it *is* installed and it has no ACP wire. The predicate is the
/// list's own now, terms and all.
#[gpui::test]
fn add_agent_is_unavailable_where_the_only_harness_cannot_converse(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![{
                let mut it = a_shared_home_harness("grok", "Grok");
                it.chat = false;
                it
            }],
        },
    );
    cx.run_until_parked();

    fixture.state.read_with(cx, |state, _| {
        assert!(
            state.workbench.harness_choices(&[], &[], None).is_empty(),
            "nothing is on offer, which is what the button has to agree with"
        );
        assert!(
            !state.can_write_definition(),
            "so the form would open on an empty picker"
        );
        assert!(
            state
                .no_harness_reason()
                .contains("cannot hold a conversation"),
            "and the sentence says the true thing, not \u{201c}install one\u{201d}: {}",
            state.no_harness_reason()
        );
    });
}
