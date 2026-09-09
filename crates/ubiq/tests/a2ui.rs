use ubiq::state::a2ui;

#[test]
fn every_example_parses_and_covers_the_catalog() {
    let mut seen = std::collections::BTreeSet::new();
    for example in a2ui::EXAMPLES {
        let surface = a2ui::parse(example.source)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", example.name));
        assert!(surface.get(&surface.root).is_some(), "{}", example.name);
        for component in surface.by_id.values() {
            assert_ne!(
                component.kind.tag(),
                "unknown",
                "{}: component `{}` did not match the catalog",
                example.name,
                component.id
            );
            seen.insert(component.kind.tag());
        }
    }
    assert_eq!(seen.len(), 18, "covered: {seen:?}");
}

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
    let surface = a2ui::parse(cyclic).expect("a cycle is a valid payload");

    // The walk the renderer does, with the same bound.
    let mut depth = 0;
    let mut at = surface.root.clone();
    while depth <= a2ui::MAX_DEPTH {
        let Some(component) = surface.get(&at) else {
            break;
        };
        let a2ui::Kind::Column { children, .. } = &component.kind else {
            break;
        };
        match children.ids().first() {
            Some(next) => at = next.clone(),
            None => break,
        }
        depth += 1;
    }
    assert!(
        depth > a2ui::MAX_DEPTH,
        "the cycle stopped early at {depth}"
    );

    let dangling = r#"{"createSurface":{"components":[
        {"id":"root","component":"Card","child":"gone"}]}}"#;
    let surface = a2ui::parse(dangling).expect("a dangling id is a valid payload");
    assert!(surface.get("gone").is_none());
}

/// A bound property keeps its pointer rather than resolving it: nothing here parses a data model,
/// and a renderer that quietly drew an empty string would hide that.
#[test]
fn a_bound_property_keeps_its_pointer() {
    let bound = r#"{"createSurface":{"components":[
        {"id":"root","component":"Text","text":{"path":"/user/name"}}]}}"#;
    let surface = a2ui::parse(bound).unwrap();
    let a2ui::Kind::Text { text, .. } = &surface.get("root").unwrap().kind else {
        panic!("not a Text");
    };
    let text = text.as_ref().expect("Text carries its property");
    assert_eq!(text.binding(), Some("/user/name"));
    assert_eq!(text.literal(), None);
    assert_eq!(text.text(), "/user/name");
}
