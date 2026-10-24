//! draw.io onto the drawing model: the containers, the styles, the routing, the failure rules.

use std::path::PathBuf;

use ubiq_draw::drawio::{self, DrawioError};
use ubiq_draw::{ArrowHead, Drawing, Geometry, Node, Rgba8, Shape, StrokeStyle, VAlign};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).expect("the fixture is in the tree")
}

fn shape<'a>(drawing: &'a Drawing, id: &str) -> &'a Shape {
    drawing
        .shapes()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("no shape {id}"))
}

fn ids(drawing: &Drawing) -> Vec<&str> {
    drawing
        .shapes()
        .into_iter()
        .map(|s| s.id.as_str())
        .collect()
}

#[test]
fn the_fixture_maps_its_common_styles() {
    let drawing = drawio::parse(&fixture("flow.drawio")).expect("the fixture parses");
    assert_eq!(drawing.background, Some(Rgba8::opaque(0xfa, 0xfa, 0xfa)));

    let start = shape(&drawing, "start");
    assert_eq!(start.geometry, Geometry::Ellipse);
    assert_eq!(start.style.fill, Some(Rgba8::opaque(0xda, 0xe8, 0xfc)));
    assert_eq!(start.style.stroke, Some(Rgba8::opaque(0x6c, 0x8e, 0xbf)));
    assert_eq!(start.text.as_ref().unwrap().text, "Start");
    assert_eq!(start.text.as_ref().unwrap().valign, VAlign::Middle);

    let boxed = shape(&drawing, "box");
    assert!(matches!(boxed.geometry, Geometry::RoundedRect { radius } if radius > 0.0));
    assert_eq!(boxed.style.stroke_style, StrokeStyle::Dashed);
    assert!((boxed.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    // Defaults: a vertex is white with a black outline.
    assert_eq!(boxed.style.fill, Some(Rgba8::WHITE));
    assert_eq!(boxed.style.stroke, Some(Rgba8::BLACK));
    // html=1: tags gone, <br> a line break, entities decoded.
    assert_eq!(boxed.text.as_ref().unwrap().text, "Do it\nnow & then");

    let choice = shape(&drawing, "choice");
    assert_eq!(choice.geometry, Geometry::Diamond);
    assert_eq!(choice.style.fill, None, "fillColor=none is hollow");

    let note = shape(&drawing, "note");
    assert_eq!(note.geometry, Geometry::Text);
    assert_eq!(note.style.stroke, None);
}

#[test]
fn edges_route_between_their_shapes_with_their_heads() {
    let drawing = drawio::parse(&fixture("flow.drawio")).unwrap();

    let Geometry::Connector(e1) = &shape(&drawing, "e1").geometry else {
        panic!("e1 is a connector");
    };
    assert_eq!(e1.source.as_deref(), Some("start"));
    assert_eq!(e1.target.as_deref(), Some("box"));
    assert_eq!(e1.end, ArrowHead::Filled);
    assert_eq!(e1.start, ArrowHead::None);
    // Orthogonal: every segment is horizontal or vertical, leaving start's right side.
    assert!(
        e1.points
            .windows(2)
            .all(|w| (w[0].x - w[1].x).abs() < 0.01 || (w[0].y - w[1].y).abs() < 0.01)
    );
    assert!((e1.points[0].x - 160.0).abs() < 0.01);
    assert!((e1.points.last().unwrap().x - 240.0).abs() < 0.01);
    assert_eq!(shape(&drawing, "e1").text.as_ref().unwrap().text, "go");

    let e2 = shape(&drawing, "e2");
    let Geometry::Connector(c2) = &e2.geometry else {
        panic!("e2 is a connector");
    };
    assert_eq!(c2.end, ArrowHead::Open);
    assert_eq!(c2.start, ArrowHead::Filled);
    assert_eq!(e2.style.stroke, Some(Rgba8::opaque(0xff, 0, 0)));
    // Straight through its waypoint, clipped to box's bottom and choice's top.
    assert_eq!(c2.points.len(), 3);
    assert!((c2.points[0].y - 100.0).abs() < 0.01);
    assert!((c2.points[1].y - 130.0).abs() < 0.01);
    assert!((c2.points[2].y - 160.0).abs() < 0.5);
}

#[test]
fn a_group_nests_its_children_at_its_offset_and_unknown_shapes_are_placeholders() {
    let drawing = drawio::parse(&fixture("flow.drawio")).unwrap();

    let layer = match &drawing.root.children[..] {
        [Node::Group(layer)] => layer,
        other => panic!("one layer group, got {other:?}"),
    };
    let group = layer
        .children
        .iter()
        .find_map(|n| match n {
            Node::Group(g) if g.id == "grp" => Some(g),
            _ => None,
        })
        .expect("the group is a group");
    // An unlabelled `group` container draws nothing of its own.
    assert_eq!(group.children.len(), 2);

    let a = shape(&drawing, "inner-a");
    assert_eq!((a.frame.x, a.frame.y), (410.0, 210.0));

    let b = shape(&drawing, "inner-b");
    assert_eq!(b.geometry, Geometry::Placeholder);
    assert_eq!(b.text.as_ref().unwrap().text, "B");

    // Cell order is the paint order.
    assert_eq!(
        ids(&drawing),
        [
            "start", "box", "choice", "e1", "e2", "inner-a", "inner-b", "note"
        ]
    );
    assert!(!drawing.bounds.is_empty());
}

#[test]
fn pages_are_listed_and_selectable() {
    let bytes = fixture("flow.drawio");
    let pages = drawio::pages(&bytes).unwrap();
    let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Flow", "Second"]);

    let second = drawio::parse_page(&bytes, 1).unwrap();
    assert_eq!(ids(&second), ["only"]);
    assert_eq!(
        drawio::parse_page(&bytes, 5),
        Err(DrawioError::NoSuchPage(5))
    );
}

#[test]
fn a_compressed_page_decodes() {
    let drawing = drawio::parse(&fixture("compressed.drawio")).expect("inflates and parses");
    assert_eq!(ids(&drawing), ["a", "b", "ab"]);
    assert_eq!(
        shape(&drawing, "a").text.as_ref().unwrap().text,
        "Zipped & ok"
    );
    let Geometry::Connector(edge) = &shape(&drawing, "ab").geometry else {
        panic!("ab is a connector");
    };
    // Clipped to the ellipse's right edge and the box's left.
    assert!((edge.points[0].x - 110.0).abs() < 0.5);
    assert!((edge.points[1].x - 200.0).abs() < 0.5);
}

#[test]
fn a_drawio_svg_carries_its_file_in_content() {
    let svg = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" content="&lt;mxfile&gt;&lt;diagram name=&quot;S&quot;&gt;&lt;mxGraphModel&gt;&lt;root&gt;&lt;mxCell id=&quot;0&quot;/&gt;&lt;mxCell id=&quot;1&quot; parent=&quot;0&quot;/&gt;&lt;mxCell id=&quot;v&quot; value=&quot;in svg&quot; vertex=&quot;1&quot; parent=&quot;1&quot;&gt;&lt;mxGeometry width=&quot;10&quot; height=&quot;10&quot; as=&quot;geometry&quot;/&gt;&lt;/mxCell&gt;&lt;/root&gt;&lt;/mxGraphModel&gt;&lt;/diagram&gt;&lt;/mxfile&gt;"><g/></svg>"#;
    let drawing = drawio::parse(svg.as_bytes()).expect("the embedded file parses");
    assert_eq!(ids(&drawing), ["v"]);
}

#[test]
fn a_bare_model_is_one_page() {
    let xml = r#"<mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/>
        <mxCell id="v" vertex="1" parent="1"><mxGeometry width="5" height="5" as="geometry"/></mxCell>
        </root></mxGraphModel>"#;
    assert_eq!(ids(&drawio::parse(xml.as_bytes()).unwrap()), ["v"]);
}

#[test]
fn nothing_a_file_can_be_panics() {
    assert_eq!(drawio::parse(&[0xff, 0xfe]), Err(DrawioError::NotUtf8));
    assert!(matches!(
        drawio::parse(b"<mxfile"),
        Err(DrawioError::Xml(_))
    ));
    assert_eq!(drawio::parse(b"<html/>"), Err(DrawioError::NotADiagram));
    assert_eq!(
        drawio::parse(b"<mxfile><diagram>not base64 at all!</diagram></mxfile>"),
        Err(DrawioError::Undecodable)
    );
    // Dangling references, a self-parent, an edge with no ends: drawn around, never fatal.
    let odd = r#"<mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/>
        <mxCell id="s" vertex="1" parent="s"/>
        <mxCell id="e" edge="1" parent="1" source="nope"/>
        <mxCell id="v" vertex="1" parent="1" style="shape=mxgraph.aws.thing"><mxGeometry width="5" height="5" as="geometry"/></mxCell>
        </root></mxGraphModel>"#;
    let drawing = drawio::parse(odd.as_bytes()).unwrap();
    assert_eq!(ids(&drawing), ["v"]);
    assert_eq!(shape(&drawing, "v").geometry, Geometry::Placeholder);
    assert_eq!(
        shape(&drawing, "v").text.as_ref().unwrap().text,
        "mxgraph.aws.thing"
    );
}

#[test]
fn html_labels_strip_to_text() {
    assert_eq!(
        drawio::html_to_text("<div>one</div><div>two&nbsp;&lt;3&gt;</div>"),
        "one\ntwo <3>"
    );
    assert_eq!(
        drawio::html_to_text("a  <span style='x'>b</span>\n c"),
        "a b c"
    );
    assert_eq!(drawio::html_to_text("&#65;&#x42;&bogus;"), "AB&bogus;");
}

/// Standard base64, for building a compressed page in a test.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
        for k in 0..4 {
            out.push(if k <= chunk.len() {
                ABC[(n >> (18 - 6 * k) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// A deflate bomb — a few kilobytes that inflate past the cap — is refused, not allocated.
#[test]
fn a_deflate_bomb_is_refused() {
    use std::io::Write;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    let zeros = vec![b'a'; 1024 * 1024];
    for _ in 0..40 {
        encoder.write_all(&zeros).unwrap();
    }
    let packed = base64(&encoder.finish().unwrap());
    assert!(packed.len() < 1024 * 1024, "the bomb is small");
    assert_eq!(drawio::decode_diagram(&packed), None);
    let file = format!("<mxfile><diagram>{packed}</diagram></mxfile>");
    assert_eq!(
        drawio::parse(file.as_bytes()),
        Err(DrawioError::Undecodable)
    );

    // A small page still decodes.
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(b"<mxGraphModel/>").unwrap();
    let small = base64(&encoder.finish().unwrap());
    assert_eq!(
        drawio::decode_diagram(&small).as_deref(),
        Some("<mxGraphModel/>")
    );
}

/// Repeated ids, each level's copies parenting the next level, would build every subtree once per
/// copy — 2^40 here. The first cell keeps an id and the rest are dropped.
#[test]
fn duplicate_ids_build_once() {
    let mut cells = String::from(r#"<mxCell id="0"/><mxCell id="1" parent="0"/>"#);
    for level in 0..40 {
        let parent = if level == 0 {
            "1".to_string()
        } else {
            format!("n{}", level - 1)
        };
        for _ in 0..2 {
            cells.push_str(&format!(
                r#"<mxCell id="n{level}" vertex="1" parent="{parent}"><mxGeometry width="5" height="5" as="geometry"/></mxCell>"#
            ));
        }
    }
    let file = format!("<mxGraphModel><root>{cells}</root></mxGraphModel>");
    let drawing = drawio::parse(file.as_bytes()).expect("parses");
    assert_eq!(drawing.shapes().len(), 40);
}

/// NaN, infinities and numbers past `f32` in a style or a geometry never reach the model.
#[test]
fn non_finite_numbers_are_dropped_or_clamped() {
    let file = r#"<mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/>
        <mxCell id="v" value="x" vertex="1" parent="1"
            style="fontSize=1e300;strokeWidth=inf;rotation=NaN;opacity=NaN;arcSize=1e40;rounded=1">
          <mxGeometry x="1e40" y="-1e20" width="NaN" height="1e30" as="geometry"/>
        </mxCell>
        <mxCell id="e" vertex="0" edge="1" parent="1" source="v" style="fontSize=inf" value="e">
          <mxGeometry relative="1" as="geometry">
            <mxPoint x="inf" y="0" as="sourcePoint"/><mxPoint x="1e39" y="1e39" as="targetPoint"/>
          </mxGeometry>
        </mxCell>
        </root></mxGraphModel>"#;
    let drawing = drawio::parse(file.as_bytes()).expect("parses");
    for s in drawing.shapes() {
        let f = s.frame;
        assert!(
            [
                f.x,
                f.y,
                f.w,
                f.h,
                s.rotation,
                s.style.stroke_width,
                s.style.opacity
            ]
            .iter()
            .all(|v| v.is_finite()),
            "{}",
            s.id
        );
        if let Some(text) = &s.text {
            assert!(text.font_size <= ubiq_draw::model::MAX_FONT_SIZE);
            assert!(
                [text.rect.x, text.rect.y, text.rect.w, text.rect.h]
                    .iter()
                    .all(|v| v.is_finite())
            );
        }
        if let Geometry::Connector(c) = &s.geometry {
            assert!(c.points.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
        }
    }
    assert!(drawing.bounds.width().is_finite());
}
