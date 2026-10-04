//! Wire back-compat for the annotation marks, addressing and highlights.

use ubiq_proto::ids::{AnnotationId, BlockId};
use ubiq_proto::plan::{Annotation, AnnotationMark, BlockHighlight, HighlightColour};
use ubiq_proto::work::{Addressee, Comment, CommentAuthor};

#[test]
fn old_comment_json_loads_without_to() {
    let c = Comment::new(CommentAuthor::User, "hi".into(), chrono::Utc::now());
    let mut v = serde_json::to_value(&c).unwrap();
    assert!(v.get("to").is_none(), "None is not serialised");
    v.as_object_mut().unwrap().remove("to");
    let back: Comment = serde_json::from_value(v).unwrap();
    assert_eq!(back.to, None);
}

#[test]
fn addressed_comment_roundtrips() {
    let c = Comment::addressed(
        CommentAuthor::User,
        "hi".into(),
        chrono::Utc::now(),
        Some(Addressee::Agent),
    );
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["to"], "agent");
    assert_eq!(serde_json::from_value::<Comment>(v).unwrap(), c);
}

#[test]
fn old_annotation_json_loads_without_marks() {
    let a = Annotation::new(
        BlockId::generate(),
        None,
        CommentAuthor::User,
        "x".into(),
        chrono::Utc::now(),
    );
    let v = serde_json::to_value(&a).unwrap();
    assert!(v.get("marks").is_none(), "empty marks are not serialised");
    let back: Annotation = serde_json::from_value(v).unwrap();
    assert!(back.marks.is_empty());
    let _ = AnnotationId::generate();
}

#[test]
fn marks_set_once_and_roundtrip() {
    let mut a = Annotation::new(
        BlockId::generate(),
        None,
        CommentAuthor::User,
        "x".into(),
        chrono::Utc::now(),
    );
    assert!(a.set_mark(AnnotationMark::Todo, true));
    assert!(!a.set_mark(AnnotationMark::Todo, true));
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["marks"][0], "todo");
    assert_eq!(serde_json::from_value::<Annotation>(v).unwrap(), a);
    assert!(a.set_mark(AnnotationMark::Todo, false));
    assert!(!a.set_mark(AnnotationMark::Todo, false));
}

#[test]
fn highlight_roundtrips() {
    let h = BlockHighlight { block_id: BlockId::generate(), colour: HighlightColour::Purple };
    let v = serde_json::to_value(h).unwrap();
    assert_eq!(v["colour"], "purple");
    assert_eq!(serde_json::from_value::<BlockHighlight>(v).unwrap(), h);
}
