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
use ubiq::state::WindowRegistry;
use ubiq::state::editor::{OpenFile, ViewLayout, ViewerKind};
use ubiq::state::nav::{Destination, Locus, View};
use ubiq::state::prefs;
use ubiq::state::settings::{self, MarkdownOpen, UiSettings};
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
        capture_enabled: false,
        rail_projects: false,
        markdown_open: MarkdownOpen::Source,
        vim_mode: true,
        show_cache_ring: true,
        last_connection: Some("01J0".to_string()),
    };
    let back = settings::decode(&settings::encode(&settings)).expect("decodes");
    assert_eq!(back, settings);
}

#[test]
fn missing_fields_open_on_defaults() {
    let blob = r#"{"schema":1}"#;
    let back = settings::decode(blob).expect("decodes");
    assert!(back.explorer_preview);
    assert!(back.capture_enabled);
    assert!(back.rail_projects);
    assert_eq!(back.markdown_open, MarkdownOpen::Preview);
    // A blob written before vim mode existed opens with it off, rather than being discarded.
    assert!(!back.vim_mode);
    assert!(!back.show_cache_ring);
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
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
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
        state.set_density(ubiq::theme::Density::Compact, cx);
        state.set_chrome_font_size(13.5, cx);
        state.set_conversation_font_size(15.0, cx);
    });
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
    assert_eq!(sent.density, ubiq::theme::Density::Compact);
    assert_eq!(sent.chrome_font_size, Some(13.5));
    assert_eq!(sent.conversation_font_size, Some(15.0));
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
