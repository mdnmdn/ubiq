//! The A2UI renderer's half of a protocol Ubiq does not own.
//!
//! Everything here runs with no window. That is not an accident of what was easy to test: the
//! parse, the pointers, the function library, the template arithmetic, the action envelope and the
//! SVG sanitizer are all in `state/`, precisely so that what a surface *means* can be asserted
//! without drawing one. The module-level tests inside `state/a2ui/{value,eval,svg,path}.rs` cover
//! each piece on its own; what is here is the seams between them, and the fixtures the page ships.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui::{AppContext as _, Context, Entity, TestAppContext, Window, WindowHandle};
use gpui_component::Root;
use serde_json::{Value, json};
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::a2ui::{self, BASIC, Kind, Scope, action, eval, live, value};
use ubiq::ui::a2ui::registry;
use ubiq_proto::bus;

/// Every example the picker offers parses, and between them they use every component this build
/// claims to draw — the eighteen of the basic catalog, and everything Ubiq's own catalog adds.
///
/// A registered component with no example fails the same way an uncovered basic one does, because
/// a component nobody ever draws is a component nobody notices breaking.
#[test]
fn every_example_parses_and_covers_both_catalogs() {
    let mut basic = BTreeSet::new();
    let mut extensions = BTreeSet::new();

    for example in a2ui::EXAMPLES {
        let parsed = a2ui::parse(example.source)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", example.name));
        assert!(
            parsed.surface.get(&parsed.surface.root).is_some(),
            "{}",
            example.name
        );
        for component in parsed.surface.by_id.values() {
            match &component.kind {
                Kind::Extension {
                    component: name, ..
                } => {
                    assert!(
                        registry::lookup(name).is_some(),
                        "{}: `{name}` is in neither catalog",
                        example.name
                    );
                    extensions.insert(name.clone());
                }
                kind => {
                    basic.insert(kind.tag().to_string());
                }
            }
        }
    }

    assert_eq!(
        basic,
        BASIC.iter().map(|name| name.to_string()).collect(),
        "the basic catalog is not covered"
    );
    assert_eq!(
        extensions,
        registry::names().map(String::from).collect(),
        "Ubiq's own catalog is not covered"
    );
}

/// The drawing registry and the catalog document name the same components.
///
/// Two sources of truth, because only one of them can be handed to an agent and only the other can
/// draw anything. This is what keeps them from drifting — the same discipline `just icons-check`
/// keeps over the icon set.
#[test]
fn the_registry_and_the_catalog_name_the_same_components() {
    let catalog: Value =
        serde_json::from_str(a2ui::UBIQ_CATALOG).expect("the Ubiq catalog is valid JSON");

    let declared: BTreeSet<String> = catalog["components"]
        .as_object()
        .expect("the catalog declares components")
        .keys()
        .cloned()
        .collect();
    let drawn: BTreeSet<String> = registry::names().map(String::from).collect();
    assert_eq!(declared, drawn);

    for name in &declared {
        // The specification requires UAX #31 identifiers; the ASCII slice of it is what a catalog
        // written here will ever use, and is not worth a Unicode property table to check.
        assert!(
            name.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "`{name}` is not a usable catalog identifier"
        );
        assert_ne!(name, "Surface", "`Surface` is reserved by the protocol");
    }
}

/// A payload that says nothing usable says so, rather than drawing nothing without a reason.
#[test]
fn a_broken_payload_is_an_error_not_a_panic() {
    assert!(a2ui::parse("{").is_err());
    assert!(a2ui::parse(r#"{"createSurface":{"components":[]}}"#).is_err());
    assert!(
        a2ui::parse(r#"{"createSurface":{"components":[{"id":"x","component":"Text"}]}}"#).is_err()
    );
}

/// A payload may name a child it does not carry, or point two components at each other. Neither is
/// a parse error — the graph is only discovered by walking it — so the walk is what has to survive
/// them, and `MAX_DEPTH` is what makes that true without a visited set.
#[test]
fn a_cycle_and_a_missing_child_are_both_walkable() {
    let cyclic = r#"{"createSurface":{"components":[
        {"id":"root","component":"Column","children":["loop"]},
        {"id":"loop","component":"Column","children":["root"]}]}}"#;
    let parsed = a2ui::parse(cyclic).expect("a cycle is a valid payload");

    // The shared walk is the renderer's own, so surviving a cycle here is surviving one on screen.
    let mut seen = 0usize;
    a2ui::walk(&parsed, &mut |_, _| seen += 1);
    assert!(seen > 0 && seen <= a2ui::MAX_DEPTH as usize + 2);

    let dangling = r#"{"createSurface":{"components":[
        {"id":"root","component":"Card","child":"gone"}]}}"#;
    let parsed = a2ui::parse(dangling).expect("a dangling id is a valid payload");
    assert!(parsed.surface.get("gone").is_none());
}

/// A bound property keeps the pointer it named, which is what an input writes back through and
/// what the page draws under one.
#[test]
fn a_bound_property_keeps_its_pointer() {
    let bound = r#"{"createSurface":{"components":[
        {"id":"root","component":"Text","text":{"path":"/user/name"}}],
        "dataModel":{"user":{"name":"Ada"}}}}"#;
    let parsed = a2ui::parse(bound).unwrap();
    let Kind::Text { text, .. } = &parsed.surface.get("root").unwrap().kind else {
        panic!("not a Text");
    };
    let text = text.as_ref().expect("Text carries its property");
    assert_eq!(text.path(), Some("/user/name"));
    // And now it resolves, which is the whole difference this change makes.
    assert_eq!(
        text.as_display_string(&parsed.model, Scope::default()),
        "Ada"
    );
}

/// A function call is not a literal.
///
/// The regression this names is real and was shipping: the old `#[serde(untagged)]` property swallowed
/// `{"call": …}` into its catch-all arm and drew it as an empty string, so every `formatString` in
/// a payload rendered as nothing at all.
#[test]
fn a_function_call_is_not_swallowed() {
    let payload = r#"{"createSurface":{"components":[
        {"id":"root","component":"Text",
         "text":{"call":"formatString","args":{"value":"Hello ${/who}"}}}],
        "dataModel":{"who":"world"}}}"#;
    let parsed = a2ui::parse(payload).unwrap();
    let Kind::Text { text, .. } = &parsed.surface.get("root").unwrap().kind else {
        panic!("not a Text");
    };
    assert_eq!(
        text.as_ref()
            .unwrap()
            .as_display_string(&parsed.model, Scope::default()),
        "Hello world"
    );
}

/// A template is one component drawn once per item, and an input inside it writes to *that item's*
/// pointer.
///
/// This is the arithmetic everything two-way depends on: a field bound to the relative `qty` in a
/// row over `/items` must write `/items/2/qty` and not `/qty`, and two repeats of one component
/// must not share a buffer.
#[test]
fn a_template_expands_to_absolute_write_paths() {
    let parsed = a2ui::parse(
        a2ui::EXAMPLES
            .iter()
            .find(|example| example.name == "Bindings and actions")
            .expect("the bindings example is registered")
            .source,
    )
    .expect("the bindings example parses");

    let slots = live::input_slots(&parsed);
    let qty: Vec<&live::Slot> = slots
        .iter()
        .filter(|slot| slot.component == "item_qty")
        .collect();
    assert_eq!(qty.len(), 3, "one per item in /items");
    assert_eq!(qty[0].path, "/items/0/qty");
    assert_eq!(qty[2].path, "/items/2/qty");
    assert_eq!(qty[2].seed, "5");

    let keys: BTreeSet<&String> = slots.iter().map(|slot| &slot.key).collect();
    assert_eq!(keys.len(), slots.len(), "two repeats share a key");

    // A template whose pointer finds nothing repeats nothing, rather than drawing one of it.
    let empty = r#"{"createSurface":{"components":[
        {"id":"root","component":"List","children":{"path":"/nope","componentId":"tpl"}},
        {"id":"tpl","component":"TextField","value":{"path":"x"}}]}}"#;
    let parsed = a2ui::parse(empty).unwrap();
    assert!(live::input_slots(&parsed).is_empty());
}

/// A relative path outside a template is read from the root, and a pointer that finds nothing is
/// an empty string rather than a failure.
#[test]
fn loose_paths_degrade_rather_than_fail() {
    assert_eq!(Scope::default().resolve("who"), "/who");
    let model = json!({"who": "world"});
    let bare: eval::Dynamic = serde_json::from_value(json!({"path": "who"})).unwrap();
    assert_eq!(bare.as_display_string(&model, Scope::default()), "world");

    let missing: eval::Dynamic = serde_json::from_value(json!({"path": "/nowhere"})).unwrap();
    assert_eq!(missing.as_display_string(&model, Scope::default()), "");
}

/// A fired event carries what the protocol requires, with its context resolved through the model —
/// and the model itself only when the surface asked for it.
#[test]
fn an_action_envelope_carries_the_required_keys() {
    let parsed = a2ui::parse(
        a2ui::EXAMPLES
            .iter()
            .find(|example| example.name == "Bindings and actions")
            .unwrap()
            .source,
    )
    .unwrap();

    let Some(action::Action::Event(event)) = parsed
        .surface
        .get("submitBtn")
        .and_then(|button| button.kind.button_action())
    else {
        panic!("submitBtn carries an event");
    };

    let envelope = action::envelope(
        &parsed.surface_id,
        "submitBtn",
        event,
        &parsed.model,
        Scope::default(),
        "2026-09-12T10:00:00Z",
        parsed.send_data_model,
    );

    assert_eq!(envelope["version"], "v1.0");
    let fired = &envelope["action"];
    assert_eq!(fired["name"], "submitOrder");
    assert_eq!(fired["surfaceId"], parsed.surface_id.as_str());
    assert_eq!(fired["sourceComponentId"], "submitBtn");
    assert_eq!(fired["timestamp"], "2026-09-12T10:00:00Z");
    // A literal stays itself; a binding arrives as the value behind it, never as the pointer.
    assert_eq!(fired["context"]["source"], "bindings_demo");
    assert_eq!(fired["context"]["email"], "jane@example.com");
    assert_eq!(fired["userMessage"], "Submit my order");
    assert!(
        envelope.get("dataModel").is_some(),
        "this surface set sendDataModel"
    );

    let quiet = action::envelope(
        "s",
        "b",
        event,
        &parsed.model,
        Scope::default(),
        "2026-09-12T10:00:00Z",
        false,
    );
    assert!(quiet.get("dataModel").is_none());
}

/// A failing check blocks the action, and filling the value in unblocks it.
///
/// The rules live on the inputs and the consequence shows on the button, which is the catalog's own
/// arrangement — so this asks the surface, not the button.
#[test]
fn a_failing_check_blocks_the_action() {
    let mut parsed = a2ui::parse(
        a2ui::EXAMPLES
            .iter()
            .find(|example| example.name == "Checkable inputs")
            .unwrap()
            .source,
    )
    .unwrap();

    let failing: BTreeSet<String> = action::blocking_checks(&parsed)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert!(
        failing.iter().any(|key| key.starts_with("tf1@")),
        "an empty email fails its required check: {failing:?}"
    );

    // Every message is a sentence somebody wrote, not a type name.
    for (_, message) in action::blocking_checks(&parsed) {
        assert!(!message.is_empty());
    }

    value::set(
        &mut parsed.model,
        "/formData/email",
        Value::String("ada@example.com".into()),
    );
    let after: BTreeSet<String> = action::blocking_checks(&parsed)
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert!(
        !after.iter().any(|key| key.starts_with("tf1@")),
        "a filled email still fails: {after:?}"
    );
}

/// A click writes where the payload said, and everything bound to that pointer sees it.
#[test]
fn a_write_reaches_every_reader_of_its_pointer() {
    let mut model = json!({});
    value::set(
        &mut model,
        "/form/email",
        Value::String("ada@lovelace".into()),
    );
    assert_eq!(
        value::get(&model, "/form/email").and_then(Value::as_str),
        Some("ada@lovelace")
    );

    let bound: eval::Dynamic = serde_json::from_value(json!({"path": "/form/email"})).unwrap();
    assert_eq!(
        bound.as_display_string(&model, Scope::default()),
        "ada@lovelace"
    );
    assert_eq!(
        bound.write_target(Scope::default()).as_deref(),
        Some("/form/email")
    );
    // A literal has nowhere to write, which is how the renderer tells a live field from a dead one.
    let literal: eval::Dynamic = serde_json::from_value(json!("fixed")).unwrap();
    assert!(literal.write_target(Scope::default()).is_none());
}

/// A component from no catalog keeps its properties rather than being thrown away, which is what
/// makes an extension possible at all.
#[test]
fn an_unknown_component_keeps_its_properties() {
    let payload = r#"{"createSurface":{"components":[
        {"id":"root","component":"Sparkline","points":[1,2,3],"weight":2}]}}"#;
    let parsed = a2ui::parse(payload).unwrap();
    let component = parsed.surface.get("root").unwrap();
    let Kind::Extension {
        component: name,
        props,
    } = &component.kind
    else {
        panic!("not an extension");
    };
    assert_eq!(name, "Sparkline");
    assert_eq!(props["points"], json!([1, 2, 3]));
    assert_eq!(component.weight, Some(2.0));
    assert!(registry::lookup("Sparkline").is_none());
}

/// Markup that reaches outside itself is refused, and the refusal says what it refused.
///
/// The sanitizer's own tests cover each construct; this is the seam — that the component's payload
/// path reaches it at all, and that a clean picture still gets through.
#[test]
fn the_svg_component_refuses_markup_that_reaches_outside_itself() {
    use ubiq::state::a2ui::svg;

    let clean = svg::sanitize(
        r#"<svg viewBox="0 0 24 12"><path d="M0 12 L24 0" stroke="currentColor"/></svg>"#,
    )
    .expect("a self-contained picture is drawn");
    assert_eq!((clean.width, clean.height), (24.0, 12.0));

    for markup in [
        r#"<svg viewBox="0 0 10 10"><script>alert(1)</script></svg>"#,
        r#"<svg viewBox="0 0 10 10"><image href="http://x/y.png"/></svg>"#,
        r##"<svg viewBox="0 0 10 10"><use href="#a"/></svg>"##,
        r#"<svg viewBox="0 0 10 10"><rect onclick="x()"/></svg>"#,
        r#"<!DOCTYPE svg><svg viewBox="0 0 10 10"><rect/></svg>"#,
        r#"<svg viewBox="0 0 10 10"><rect fill="url(http://x/y)"/></svg>"#,
        r#"<svg><rect/></svg>"#,
        // Casing is not a way past an allow-list, and that is the property rather than a case
        // table: SVG is XML, so names are case-sensitive, and anything that is not exactly a
        // permitted name is refused for being unknown rather than for being dangerous.
        r#"<svg viewBox="0 0 10 10"><SCRIPT>alert(1)</SCRIPT></svg>"#,
        r#"<svg viewBox="0 0 10 10"><rect ONCLICK="x()"/></svg>"#,
    ] {
        let refused = svg::sanitize(markup);
        assert!(refused.is_err(), "not refused: {markup}");
        assert!(
            !refused.unwrap_err().is_empty(),
            "refused without saying why: {markup}"
        );
    }

    // The refusal the Ubiq catalog example ships on purpose, so a reader sees one.
    let demo = a2ui::EXAMPLES
        .iter()
        .find(|example| example.name == "Ubiq catalog")
        .unwrap();
    assert!(
        demo.source.contains("<script>"),
        "the catalog example no longer carries the payload it exists to refuse"
    );
}

/// Every `svgPath` glyph the fixtures ship actually parses into something paintable.
///
/// The basic catalog lets an `Icon`'s name be a path string instead of one of its 59 names, and
/// that route is painted rather than rasterised — so a path the parser chokes on is a glyph that
/// silently does not draw. The fixtures are the only place this build has real ones.
#[test]
fn every_shipped_svg_path_parses() {
    use ubiq::state::a2ui::path;

    let mut found = 0;
    for example in a2ui::EXAMPLES {
        let parsed = a2ui::parse(example.source).unwrap();
        for component in parsed.surface.by_id.values() {
            let Kind::Icon { name: Some(name) } = &component.kind else {
                continue;
            };
            let Some(d) = name
                .eval(&parsed.model, Scope::default())
                .get("svgPath")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            let segments = path::parse(&d)
                .unwrap_or_else(|e| panic!("{}: {} — {e}", example.name, component.id));
            assert!(
                !segments.is_empty(),
                "{}: {} parsed to nothing",
                example.name,
                component.id
            );
            found += 1;
        }
    }
    assert!(found >= 2, "the fixtures stopped exercising svgPath");
}

// ── with a window ───────────────────────────────────────────────────
//
// Everything above is pure. These two are not, because what they assert cannot be: a field's
// buffer is a real `Entity<InputState>` allocated against a real window, and whether one exists
// for every input the renderer will draw is exactly the question with no pure answer.

/// A window opens with the first example already parsed into a live surface, and a buffer behind
/// every one of its text inputs.
///
/// This is the boot path — the one that allocates entities from inside the window's own
/// construction — and it earns a test precisely because a mistake there is a panic on launch
/// rather than a wrong pixel.
#[gpui::test]
fn a_window_opens_with_a_live_surface(cx: &mut TestAppContext) {
    let page = Page::open(cx);

    page.state.read_with(cx, |app, _| {
        let live = &app.sink.a2ui.live;
        assert!(live.error.is_none(), "{:?}", live.error);
        let surface = live.surface.as_ref().expect("the first example is drawn");
        assert!(surface.get(&surface.root).is_some());
        assert!(
            live.model.is_object(),
            "the data model came with the payload"
        );

        // One buffer per input the renderer will reach, and not one more.
        let parsed = live.parsed().expect("a live surface parses back");
        let expected: BTreeSet<String> = live::input_slots(&parsed)
            .into_iter()
            .map(|slot| slot.key)
            .collect();
        let held: BTreeSet<String> = live.fields.keys().cloned().collect();
        assert_eq!(held, expected);
        assert!(!held.is_empty(), "the contact form has fields");
    });
}

/// A value written reaches the model, an action quotes what is there rather than what the payload
/// said, `openUrl` opens nothing, and a payload broken while it is typed over keeps the drawing.
#[gpui::test]
fn a_surface_writes_fires_and_survives_a_broken_payload(cx: &mut TestAppContext) {
    let page = Page::open(cx);

    let bindings = a2ui::EXAMPLES
        .iter()
        .position(|example| example.name == "Bindings and actions")
        .expect("the bindings example is registered");
    page.act(cx, |app, window, cx| {
        app.pick_a2ui_example(bindings, window, cx)
    });

    // A field inside the templated list writes to that row's pointer, not the template's.
    page.act(cx, |app, _, cx| {
        app.set_a2ui_value("/items/1/qty".into(), json!(9), cx)
    });
    page.state.read_with(cx, |app, _| {
        let model = &app.sink.a2ui.live.model;
        assert_eq!(value::get(model, "/items/1/qty"), Some(&json!(9)));
        // The first row is untouched, which is what a shared buffer would have broken.
        assert_eq!(value::get(model, "/items/0/qty"), Some(&json!(2)));
    });

    // A button fires, and what it sent quotes the model.
    page.act(cx, |app, _, cx| {
        app.fire_a2ui_action("submitBtn@".into(), cx)
    });
    page.state.read_with(cx, |app, _| {
        let sent = app.sink.a2ui.live.log.first().expect("something was sent");
        assert!(sent.contains("submitOrder"), "{sent}");
        assert!(sent.contains("jane@example.com"), "{sent}");
        assert!(sent.contains("\"dataModel\""), "this surface sends it");
    });

    // `openUrl` is reported and not performed. Nothing in a payload may move this window.
    page.act(cx, |app, _, cx| app.fire_a2ui_action("linkBtn@".into(), cx));
    page.state.read_with(cx, |app, _| {
        let sent = app.sink.a2ui.live.log.first().unwrap();
        assert!(sent.contains("openUrl"), "{sent}");
        assert!(sent.contains("not performed"), "{sent}");
    });

    // A payload broken mid-keystroke keeps the surface and says why, rather than blanking. The
    // alternative would drop every field buffer on every intermediate keystroke.
    //
    // `set_value` is silent by design — it turns the editor's events off around the replacement —
    // so the reload a keystroke's subscription would have run is run here instead. That silence is
    // also why `pick_a2ui_example` reloads by hand rather than trusting the subscription.
    page.act(cx, |app, window, cx| {
        app.a2ui_buffer
            .update(cx, |buffer, cx| buffer.set_value("{ not json", window, cx));
        app.reload_a2ui(window, cx);
    });
    page.state.read_with(cx, |app, _| {
        let live = &app.sink.a2ui.live;
        assert!(live.error.is_some(), "a broken payload is reported");
        assert!(
            live.surface.is_some(),
            "the last payload that parsed is still drawn"
        );
        assert!(
            !live.log.is_empty(),
            "the log is a record, not surface state"
        );
    });
}

/// A window over no project, which is where the kitchen sink lives.
struct Page {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
}

impl Page {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(bus::hub().0, cx);
            WindowRegistry::install(cx);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();
        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self { state, window }
    }

    /// Do something to the page the way the page's own handlers do it — inside the window, so a
    /// mutator that allocates an entity has one to allocate against.
    fn act(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppState, &mut Window, &mut Context<AppState>),
    ) {
        self.window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| f(state, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
    }
}
