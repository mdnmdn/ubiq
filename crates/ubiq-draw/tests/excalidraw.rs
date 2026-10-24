//! Excalidraw onto the drawing model: the group tree, the bound label, the geometry map.

use ubiq_draw::excalidraw::Scene;
use ubiq_draw::{ArrowHead, Drawing, Geometry, Node};

fn drawing(elements: &str) -> Drawing {
    let json = format!(r#"{{"type":"excalidraw","version":2,"elements":[{elements}]}}"#);
    Scene::parse(json.as_bytes()).unwrap().to_drawing()
}

/// The tree a test can read: `id` for a shape, `[...]` for a group.
fn outline(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| match n {
            Node::Shape(s) => s.id.clone(),
            Node::Group(g) => format!("{}[{}]", g.id, outline(&g.children)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Nested groups become nested [`Node::Group`]s, each where its first member is, and a bound
/// label rides behind its container inside the group.
#[test]
fn groups_are_a_tree_in_z_order() {
    let d = drawing(
        r#"{"id":"a1","type":"rectangle","groupIds":["in","out"]},
           {"id":"mid","type":"ellipse"},
           {"id":"b","type":"rectangle","groupIds":["out"]},
           {"id":"lbl","type":"text","text":"b","containerId":"b","groupIds":["out"]},
           {"id":"a2","type":"rectangle","groupIds":["in","out"]},
           {"id":"top","type":"diamond"}"#,
    );
    assert_eq!(outline(&d.root.children), "out[in[a1 a2] b lbl] mid top");
}

/// A `groupIds` list thousands deep is cut to the outermost few, so the stack's recursion is
/// bounded and the element still draws.
#[test]
fn a_huge_group_chain_is_truncated_not_recursed() {
    let ids: Vec<String> = (0..100_000).map(|i| format!("\"g{i}\"")).collect();
    let d = drawing(&format!(
        r#"{{"id":"deep","type":"rectangle","groupIds":[{}]}},{{"id":"flat","type":"ellipse"}}"#,
        ids.join(",")
    ));
    let shapes: Vec<&str> = d.shapes().into_iter().map(|s| s.id.as_str()).collect();
    assert_eq!(shapes, ["deep", "flat"]);
    // The outermost group (the last id) is kept.
    let Node::Group(outer) = &d.root.children[0] else {
        panic!("the deep element is in a group")
    };
    assert_eq!(outer.id, "g99999");
}

/// A number past `f32` (or anywhere past the model's limit) is clamped, never an infinity.
#[test]
fn huge_numbers_are_clamped_to_finite() {
    let d = drawing(
        r#"{"id":"r","type":"rectangle","x":1e300,"y":-1e300,"width":1e39,"height":5},
           {"id":"t","type":"text","text":"x","fontSize":1e300},
           {"id":"l","type":"line","points":[[0,0],[1e300,1e300]]}"#,
    );
    for shape in d.shapes() {
        let f = shape.frame;
        assert!(
            [f.x, f.y, f.w, f.h].iter().all(|v| v.is_finite()),
            "{}",
            shape.id
        );
        if let Some(text) = &shape.text {
            assert!(
                text.font_size.is_finite() && text.font_size <= ubiq_draw::model::MAX_FONT_SIZE
            );
        }
        if let Geometry::Polyline { points, .. }
        | Geometry::Connector(ubiq_draw::Connector { points, .. }) = &shape.geometry
        {
            assert!(points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        }
    }
    assert!(d.bounds.width().is_finite() && d.bounds.height().is_finite());
}

#[test]
fn a_frame_is_a_group_with_itself_first() {
    let d = drawing(
        r#"{"id":"kid","type":"rectangle","frameId":"f"},
           {"id":"f","type":"frame","name":"Screen","x":0,"y":0,"width":100,"height":50}"#,
    );
    assert_eq!(outline(&d.root.children), "f[f kid]");
    let frame = d.shapes()[0];
    assert_eq!(frame.geometry, Geometry::Rect);
    assert_eq!(frame.text.as_ref().unwrap().text, "Screen");
    assert!(
        frame.text.as_ref().unwrap().rect.y < 0.0,
        "the name sits above"
    );
}

#[test]
fn elements_map_onto_geometry() {
    let d = drawing(
        r#"{"id":"r","type":"rectangle","roundness":{"type":3}},
           {"id":"arr","type":"arrow","x":10,"y":20,"points":[[0,0],[5,5]],
            "startBinding":{"elementId":"r"},"endBinding":null},
           {"id":"poly","type":"line","x":0,"y":0,"points":[[0,0],[10,0],[10,10],[0,0]]},
           {"id":"t","type":"text","text":"hi","fontSize":16,"x":1,"y":2,"width":30,"height":20},
           {"id":"pic","type":"image","fileId":"nope"}"#,
    );
    let shapes = d.shapes();
    assert!(matches!(shapes[0].geometry, Geometry::RoundedRect { .. }));
    let Geometry::Connector(c) = &shapes[1].geometry else {
        panic!("an arrow is a connector")
    };
    assert_eq!(c.end, ArrowHead::Filled);
    assert_eq!(c.source.as_deref(), Some("r"));
    assert_eq!(
        (c.points[1].x, c.points[1].y),
        (15.0, 25.0),
        "points are absolute"
    );
    assert!(matches!(
        shapes[2].geometry,
        Geometry::Polyline { closed: true, .. }
    ));
    assert_eq!(shapes[3].geometry, Geometry::Text);
    assert_eq!(shapes[3].text.as_ref().unwrap().font_size, 16.0);
    assert_eq!(
        shapes[4].geometry,
        Geometry::Image {
            image: Some("nope".into())
        }
    );
}
