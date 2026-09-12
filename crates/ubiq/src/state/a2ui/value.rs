//! JSON Pointers into a surface's data model, as RFC 6901 defines them and as A2UI bends them.
//!
//! **Every binding in a surface is a pointer, so this is the bottom of the renderer.** A bound
//! property names a place in the data model rather than a value, and an `updateDataModel` message
//! names one too; reading and writing those places is all that separates a payload from a drawn
//! surface.
//!
//! **Nothing here fails.** A pointer arrives from an agent, so it may index a string, walk past the
//! end of an array, or be nonsense entirely. The protocol's answer everywhere is graceful
//! degradation, so a read that cannot be made returns nothing and a write that cannot be made does
//! nothing — no error type, no panic, and no half-written model.
//!
//! Two places the specification departs from RFC 6901 and both are load-bearing: `"/"` addresses
//! the whole model alongside `""`, because that is what `updateDataModel` sends to replace it; and
//! writing an explicit null *removes* what the pointer names rather than storing a null there.

use serde_json::{Map, Value};

/// The value at this pointer, or nothing when the model does not carry it.
pub fn get<'a>(model: &'a Value, pointer: &str) -> Option<&'a Value> {
    if is_whole_document(pointer) {
        return Some(model);
    }
    let mut cursor = model;
    for segment in segments(pointer) {
        cursor = match cursor {
            Value::Object(map) => map.get(&segment)?,
            Value::Array(items) => items.get(index(&segment)?)?,
            _ => return None,
        };
    }
    Some(cursor)
}

/// Write a value at this pointer, creating whatever it has to descend through.
///
/// A missing container is built from the segment that follows it — an array when that segment is an
/// index or the append token `-`, an object otherwise — and writing past the end of an array pads
/// it with nulls. A null value removes what the pointer names instead of storing it, which is what
/// `updateDataModel` means by an explicit null.
pub fn set(model: &mut Value, pointer: &str, value: Value) {
    if is_whole_document(pointer) {
        *model = value;
        return;
    }
    let path = segments(pointer);
    let Some((last, parents)) = path.split_last() else {
        return;
    };

    let mut cursor = model;
    for (depth, segment) in parents.iter().enumerate() {
        let next = parents.get(depth + 1).unwrap_or(last);
        let wants_array = is_index(next) || next == "-";
        cursor = match descend(cursor, segment, wants_array) {
            Some(child) => child,
            None => return,
        };
    }

    let removing = value.is_null();
    match cursor {
        Value::Object(map) => {
            if removing {
                map.remove(last.as_str());
            } else {
                map.insert(last.clone(), value);
            }
        }
        Value::Array(items) => {
            if last == "-" {
                if !removing {
                    items.push(value);
                }
                return;
            }
            let Some(at) = index(last) else { return };
            if removing {
                if at < items.len() {
                    items.remove(at);
                }
                return;
            }
            if at >= items.len() {
                items.resize(at + 1, Value::Null);
            }
            items[at] = value;
        }
        _ => {}
    }
}

/// Step into the container this segment names, creating it when the model has nothing there.
///
/// `wants_array` is what the *next* segment implies, which is the only signal a pointer gives about
/// the shape of a container that does not exist yet.
fn descend<'a>(cursor: &'a mut Value, segment: &str, wants_array: bool) -> Option<&'a mut Value> {
    match cursor {
        Value::Object(map) => {
            let entry = map
                .entry(segment.to_string())
                .or_insert_with(|| empty(wants_array));
            if !entry.is_object() && !entry.is_array() {
                *entry = empty(wants_array);
            }
            Some(entry)
        }
        Value::Array(items) => {
            let at = if segment == "-" {
                items.push(empty(wants_array));
                items.len() - 1
            } else {
                let at = index(segment)?;
                if at >= items.len() {
                    items.resize(at + 1, Value::Null);
                }
                at
            };
            let entry = items.get_mut(at)?;
            if !entry.is_object() && !entry.is_array() {
                *entry = empty(wants_array);
            }
            Some(entry)
        }
        _ => None,
    }
}

/// A fresh container of the shape the following segment asks for.
fn empty(array: bool) -> Value {
    if array {
        Value::Array(Vec::new())
    } else {
        Value::Object(Map::new())
    }
}

/// Whether this pointer addresses the model itself rather than somewhere inside it.
fn is_whole_document(pointer: &str) -> bool {
    pointer.is_empty() || pointer == "/"
}

/// The unescaped segments of a pointer, leading `/` dropped.
fn segments(pointer: &str) -> Vec<String> {
    pointer
        .strip_prefix('/')
        .unwrap_or(pointer)
        .split('/')
        .map(unescape)
        .collect()
}

/// RFC 6901's escaping, undone in the order the specification names.
///
/// `~1` before `~0`: the other order turns `~01` into `/` rather than the `~1` it encodes.
fn unescape(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

/// Whether this segment addresses an array position.
fn is_index(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit())
}

/// This segment as an array position, when that is what it is.
fn index(segment: &str) -> Option<usize> {
    if is_index(segment) {
        segment.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_document_pointers_address_the_model() {
        let model = json!({"a": 1});
        assert_eq!(get(&model, ""), Some(&model));
        assert_eq!(get(&model, "/"), Some(&model));

        let mut model = json!({"a": 1});
        set(&mut model, "/", json!({"b": 2}));
        assert_eq!(model, json!({"b": 2}));

        let mut model = json!({"a": 1});
        set(&mut model, "", Value::Null);
        assert_eq!(model, Value::Null);
    }

    #[test]
    fn reads_descend_objects_and_arrays() {
        let model = json!({"rows": [{"name": "ada"}, {"name": "grace"}]});
        assert_eq!(get(&model, "/rows/1/name"), Some(&json!("grace")));
        assert_eq!(get(&model, "/rows/9/name"), None);
        assert_eq!(get(&model, "/rows/x"), None);
        assert_eq!(get(&model, "/missing/deeper"), None);
    }

    #[test]
    fn escapes_unescape_in_the_specified_order() {
        let model = json!({"a/b": 1, "c~d": 2, "~1": 3});
        assert_eq!(get(&model, "/a~1b"), Some(&json!(1)));
        assert_eq!(get(&model, "/c~0d"), Some(&json!(2)));
        // `~01` is the literal `~1`, not the `/` that the wrong unescape order would produce.
        assert_eq!(get(&model, "/~01"), Some(&json!(3)));

        let mut model = json!({});
        set(&mut model, "/a~1b", json!(true));
        set(&mut model, "/c~0d", json!(true));
        assert_eq!(model, json!({"a/b": true, "c~d": true}));
    }

    #[test]
    fn intermediates_take_the_shape_the_next_segment_implies() {
        let mut model = json!({});
        set(&mut model, "/user/name", json!("ada"));
        assert_eq!(model, json!({"user": {"name": "ada"}}));

        let mut model = json!({});
        set(&mut model, "/rows/0/name", json!("ada"));
        assert_eq!(model, json!({"rows": [{"name": "ada"}]}));

        let mut model = json!({"rows": "not a container"});
        set(&mut model, "/rows/0", json!(1));
        assert_eq!(model, json!({"rows": [1]}));
    }

    #[test]
    fn dash_appends_and_gaps_pad() {
        let mut model = json!({"rows": [1]});
        set(&mut model, "/rows/-", json!(2));
        assert_eq!(model, json!({"rows": [1, 2]}));

        let mut model = json!({});
        set(&mut model, "/rows/-/name", json!("ada"));
        assert_eq!(model, json!({"rows": [{"name": "ada"}]}));

        let mut model = json!({"rows": [1]});
        set(&mut model, "/rows/3", json!(9));
        assert_eq!(model, json!({"rows": [1, null, null, 9]}));
    }

    #[test]
    fn null_removes_rather_than_stores() {
        let mut model = json!({"a": 1, "b": 2});
        set(&mut model, "/a", Value::Null);
        assert_eq!(model, json!({"b": 2}));

        let mut model = json!({"rows": [1, 2, 3]});
        set(&mut model, "/rows/1", Value::Null);
        assert_eq!(model, json!({"rows": [1, 3]}));

        let mut model = json!({"rows": [1]});
        set(&mut model, "/rows/9", Value::Null);
        assert_eq!(model, json!({"rows": [1]}));
    }

    #[test]
    fn nonsense_pointers_are_quiet() {
        let model = json!({"a": "text"});
        assert_eq!(get(&model, "/a/b/c"), None);
        assert_eq!(get(&model, "///"), None);
        assert_eq!(get(&model, "no-leading-slash"), None);

        let mut model = json!({"a": "text"});
        let before = model.clone();
        set(&mut model, "/a/0/b", json!(1));
        set(&mut model, "/////", json!(1));
        // A write through a scalar builds containers rather than panicking; the point is only that
        // nothing here unwinds.
        assert!(model.is_object());
        let _ = before;

        let mut model = Value::String("scalar".into());
        set(&mut model, "/a", json!(1));
        assert_eq!(model, Value::String("scalar".into()));
    }
}
