//! A JSON Schema interpreter for exactly the keywords Archify's six schemas use (D6):
//! `$ref $defs $id type properties required additionalProperties enum const pattern minimum
//! maximum exclusiveMinimum minItems maxItems minLength maxLength prefixItems items oneOf anyOf
//! allOf not minProperties maxProperties propertyNames` (annotations are ignored).
//!
//! It mirrors ajv with `allErrors: true`: every failure is collected, with ajv's keyword, message,
//! params and error order, so `schema/<keyword>` codes, paths and the `{params}` tail of a message
//! agree with Archify (01 §3.2, §3.3). Differences that matter are deliberate:
//!
//! - string lengths count code points, as ajv's inlined `ucs2length` does (01 §7);
//! - `integer` is an `f64` with no fractional part, so `1.0` is an integer;
//! - `$ref` resolves by `$id` (`common.schema.json#/$defs/x` against the referring resource's id).

use std::collections::HashMap;
use std::sync::Mutex;

use regex::Regex;
use serde_json::{Map, Value};

/// One failure, ajv-shaped: `instancePath`, `keyword`, `message`, `params`, `schema`.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaError {
    pub keyword: &'static str,
    /// JSON Pointer to the failing value (`""` is the root), `~` and `/` escaped.
    pub instance_path: String,
    pub message: String,
    /// Ordered, as ajv builds them: the `{params}` tail of a message prints them in this order.
    pub params: Vec<(&'static str, Value)>,
    /// The keyword's value in the schema (`error.schema`). ajv only reports it in verbose mode and
    /// Archify does not run verbose, so it never reaches a diagnostic's `evidence`.
    pub schema: Option<Value>,
}

impl SchemaError {
    pub fn param(&self, key: &str) -> Option<&Value> {
        self.params.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }
}

/// The schemas by `$id`, plus a cache of compiled `pattern`s.
pub struct Registry {
    docs: HashMap<String, Value>,
    regexes: Mutex<HashMap<String, Option<Regex>>>,
}

impl Registry {
    /// Parse and register schema documents. Panics on malformed input: the sources are the
    /// embedded, committed schemas.
    pub fn new(sources: &[&str]) -> Self {
        let mut docs = HashMap::new();
        for source in sources {
            let doc = parse_schema(source);
            let id = doc
                .get("$id")
                .and_then(Value::as_str)
                .expect("embedded schema has an $id")
                .to_owned();
            docs.insert(id, doc);
        }
        Registry {
            docs,
            regexes: Mutex::new(HashMap::new()),
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.docs.contains_key(id)
    }

    /// Validate `data` against the registered schema `id`; every error, in ajv's order.
    pub fn validate(&self, id: &str, data: &Value) -> Vec<SchemaError> {
        let Some(root) = self.docs.get(id) else {
            return Vec::new();
        };
        let mut run = Run {
            reg: self,
            errors: Vec::new(),
            path: String::new(),
        };
        run.node(root, root, data);
        run.errors
    }

    fn regex(&self, pattern: &str) -> Option<Regex> {
        let mut cache = self.regexes.lock().expect("regex cache");
        cache
            .entry(pattern.to_owned())
            .or_insert_with(|| compile_js_pattern(pattern))
            .clone()
    }

    /// `common.schema.json#/$defs/id` against `root`'s `$id`, or `#/$defs/id` within `root`.
    fn resolve<'a>(&'a self, reference: &str, root: &'a Value) -> Option<(&'a Value, &'a Value)> {
        let (file, fragment) = reference.split_once('#').unwrap_or((reference, ""));
        let target = if file.is_empty() {
            root
        } else {
            let base = root.get("$id").and_then(Value::as_str)?;
            let dir = base.rsplit_once('/').map_or("", |(dir, _)| dir);
            self.docs.get(&format!("{dir}/{file}"))?
        };
        let schema = if fragment.is_empty() {
            target
        } else {
            target.pointer(fragment)?
        };
        Some((schema, target))
    }
}

/// JS regular expressions (`u` flag) to the `regex` crate: `\uXXXX` becomes `\x{XXXX}`, and a
/// surrogate class, which no Rust string can contain, becomes a class that matches nothing.
pub(crate) fn compile_js_pattern(pattern: &str) -> Option<Regex> {
    let mut out = String::with_capacity(pattern.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let hex: String = chars[i + 2..].iter().take(4).collect();
            if chars[i + 1] == 'u' && hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                out.push_str(&format!("\\x{{{hex}}}"));
                i += 6;
            } else {
                out.push(chars[i]);
                out.push(chars[i + 1]);
                i += 2;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    let out = out.replace("[\\x{D800}-\\x{DFFF}]", "[^\\s\\S]");
    Regex::new(&out).ok()
}

struct Run<'a> {
    reg: &'a Registry,
    errors: Vec<SchemaError>,
    path: String,
}

/// The keyword groups ajv evaluates after the untyped ones, each guarded by the data's type.
#[derive(Clone, Copy, PartialEq)]
enum Group {
    Number,
    String,
    Array,
    Object,
}

impl Group {
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Group::Number => &[
                "maximum",
                "minimum",
                "exclusiveMaximum",
                "exclusiveMinimum",
                "multipleOf",
            ],
            Group::String => &["maxLength", "minLength", "pattern", "format"],
            Group::Array => &[
                "maxItems",
                "minItems",
                "prefixItems",
                "items",
                "uniqueItems",
                "contains",
            ],
            Group::Object => &[
                "maxProperties",
                "minProperties",
                "required",
                "propertyNames",
                "additionalProperties",
                "properties",
                "patternProperties",
            ],
        }
    }
}

fn is_type(data: &Value, type_name: &str) -> bool {
    match type_name {
        "object" => data.is_object(),
        "array" => data.is_array(),
        "string" => data.is_string(),
        "boolean" => data.is_boolean(),
        "null" => data.is_null(),
        "number" => data.is_number(),
        "integer" => data
            .as_f64()
            .is_some_and(|x| x.is_finite() && x.fract() == 0.0),
        _ => false,
    }
}

/// Structural equality with JS number semantics (`1 == 1.0`), as ajv's `fast-deep-equal`.
fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| deep_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => a == b,
    }
}

fn escape_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// A numeric keyword, the operator ajv prints for it, and whether `x` satisfies `limit`.
type Comparison = (&'static str, &'static str, fn(f64, f64) -> bool);

impl Run<'_> {
    fn push_error(
        &mut self,
        keyword: &'static str,
        message: String,
        params: Vec<(&'static str, Value)>,
        schema: Option<&Value>,
    ) {
        self.errors.push(SchemaError {
            keyword,
            instance_path: self.path.clone(),
            message,
            params,
            schema: schema.cloned(),
        });
    }

    fn type_error(&mut self, types: &[&str], schema: &Value) {
        let joined = types.join(",");
        self.push_error(
            "type",
            format!("must be {joined}"),
            vec![("type", Value::from(joined.clone()))],
            schema.get("type"),
        );
    }

    /// Collect the errors `f` adds without keeping them.
    fn probe<F: FnOnce(&mut Self)>(&mut self, f: F) -> Vec<SchemaError> {
        let saved = std::mem::take(&mut self.errors);
        f(self);
        std::mem::replace(&mut self.errors, saved)
    }

    fn child<F: FnOnce(&mut Self)>(&mut self, segment: &str, f: F) {
        let len = self.path.len();
        self.path.push('/');
        self.path.push_str(&escape_pointer(segment));
        f(self);
        self.path.truncate(len);
    }

    /// Evaluate `schema` (inside the resource `root`) against `data`, in ajv's keyword order:
    /// `$ref`, `type`, the untyped keywords, then the number, string, array and object groups.
    fn node(&mut self, schema: &Value, root: &Value, data: &Value) {
        let object = match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => {
                self.push_error(
                    "false schema",
                    "boolean schema is false".into(),
                    vec![],
                    None,
                );
                return;
            }
            Value::Object(object) => object,
            _ => return,
        };

        if let Some(reference) = object.get("$ref").and_then(Value::as_str)
            && let Some((target, target_root)) = self.reg.resolve(reference, root)
        {
            self.node(target, target_root, data);
        }

        let types: Vec<&str> = match object.get("type") {
            Some(Value::String(t)) => vec![t.as_str()],
            Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let type_ok = types.is_empty() || types.iter().any(|t| is_type(data, t));
        // ajv reports a failed single type at the position of the group that has rules for it,
        // otherwise before every keyword.
        let late_group = match (types.as_slice(), type_ok) {
            (["number"], false) => Some(Group::Number),
            (["string"], false) => Some(Group::String),
            (["array"], false) => Some(Group::Array),
            (["object"], false) => Some(Group::Object),
            _ => None,
        }
        .filter(|group| group.keywords().iter().any(|k| object.contains_key(*k)));
        if !type_ok && late_group.is_none() {
            self.type_error(&types, schema);
        }

        self.untyped(object, root, data);

        if data.is_number() {
            self.number(object, data);
        } else if late_group == Some(Group::Number) {
            self.type_error(&types, schema);
        }
        if let Value::String(s) = data {
            self.string(object, s);
        } else if late_group == Some(Group::String) {
            self.type_error(&types, schema);
        }
        if let Value::Array(items) = data {
            self.array(object, root, items);
        } else if late_group == Some(Group::Array) {
            self.type_error(&types, schema);
        }
        if let Value::Object(map) = data {
            self.object(object, root, map);
        } else if late_group == Some(Group::Object) {
            self.type_error(&types, schema);
        }
    }

    fn untyped(&mut self, schema: &Map<String, Value>, root: &Value, data: &Value) {
        if let Some(expected) = schema.get("const")
            && !deep_equal(data, expected)
        {
            self.push_error(
                "const",
                "must be equal to constant".into(),
                vec![("allowedValue", expected.clone())],
                Some(expected),
            );
        }
        if let Some(allowed @ Value::Array(values)) = schema.get("enum")
            && !values.iter().any(|v| deep_equal(data, v))
        {
            self.push_error(
                "enum",
                "must be equal to one of the allowed values".into(),
                vec![("allowedValues", allowed.clone())],
                Some(allowed),
            );
        }
        if let Some(inner) = schema.get("not") {
            let inner_errors = self.probe(|run| run.node(inner, root, data));
            if inner_errors.is_empty() {
                self.push_error("not", "must NOT be valid".into(), vec![], Some(inner));
            }
        }
        if let Some(branches @ Value::Array(list)) = schema.get("anyOf") {
            let mut collected = Vec::new();
            let mut any = false;
            for branch in list {
                let branch_errors = self.probe(|run| run.node(branch, root, data));
                if branch_errors.is_empty() {
                    any = true;
                    break;
                }
                collected.extend(branch_errors);
            }
            if !any {
                self.errors.extend(collected);
                self.push_error(
                    "anyOf",
                    "must match a schema in anyOf".into(),
                    vec![],
                    Some(branches),
                );
            }
        }
        if let Some(branches @ Value::Array(list)) = schema.get("oneOf") {
            let mut collected = Vec::new();
            let mut passing = Vec::new();
            for (index, branch) in list.iter().enumerate() {
                let branch_errors = self.probe(|run| run.node(branch, root, data));
                if branch_errors.is_empty() {
                    passing.push(Value::from(index));
                } else {
                    collected.extend(branch_errors);
                }
            }
            if passing.len() != 1 {
                self.errors.extend(collected);
                let passing_schemas = if passing.is_empty() {
                    Value::Null
                } else {
                    Value::Array(passing)
                };
                self.push_error(
                    "oneOf",
                    "must match exactly one schema in oneOf".into(),
                    vec![("passingSchemas", passing_schemas)],
                    Some(branches),
                );
            }
        }
        if let Some(Value::Array(list)) = schema.get("allOf") {
            for branch in list {
                self.node(branch, root, data);
            }
        }
    }

    fn number(&mut self, schema: &Map<String, Value>, data: &Value) {
        let Some(x) = data.as_f64() else { return };
        let comparisons: [Comparison; 4] = [
            ("maximum", "<=", |x, l| x <= l),
            ("minimum", ">=", |x, l| x >= l),
            ("exclusiveMaximum", "<", |x, l| x < l),
            ("exclusiveMinimum", ">", |x, l| x > l),
        ];
        for (keyword, comparison, holds) in comparisons {
            let Some(limit) = schema.get(keyword) else {
                continue;
            };
            let Some(l) = limit.as_f64() else { continue };
            if !holds(x, l) {
                self.push_error(
                    keyword,
                    format!("must be {comparison} {limit}"),
                    vec![
                        ("comparison", Value::from(comparison)),
                        ("limit", limit.clone()),
                    ],
                    Some(limit),
                );
            }
        }
    }

    /// `maxLength`/`minLength`, `maxItems`/`minItems`, `maxProperties`/`minProperties`: ajv's
    /// "must NOT have more/fewer than N <unit>" with `{limit}`.
    fn limit(
        &mut self,
        schema: &Map<String, Value>,
        keyword: &'static str,
        measured: u64,
        unit: &str,
    ) {
        let Some(limit) = schema.get(keyword) else {
            return;
        };
        let Some(bound) = limit.as_u64() else { return };
        let (broken, direction) = if keyword.starts_with("max") {
            (measured > bound, "more")
        } else {
            (measured < bound, "fewer")
        };
        if broken {
            self.push_error(
                keyword,
                format!("must NOT have {direction} than {limit} {unit}"),
                vec![("limit", limit.clone())],
                Some(limit),
            );
        }
    }

    fn string(&mut self, schema: &Map<String, Value>, data: &str) {
        let length = data.chars().count() as u64;
        self.limit(schema, "maxLength", length, "characters");
        self.limit(schema, "minLength", length, "characters");
        // An uncompilable pattern cannot fail a value: the embedded ones all compile (tested).
        if let Some(pattern @ Value::String(source)) = schema.get("pattern")
            && self.reg.regex(source).is_some_and(|re| !re.is_match(data))
        {
            self.push_error(
                "pattern",
                format!("must match pattern \"{source}\""),
                vec![("pattern", pattern.clone())],
                Some(pattern),
            );
        }
    }

    fn array(&mut self, schema: &Map<String, Value>, root: &Value, data: &[Value]) {
        let length = data.len() as u64;
        self.limit(schema, "maxItems", length, "items");
        self.limit(schema, "minItems", length, "items");
        let prefix: &[Value] = match schema.get("prefixItems") {
            Some(Value::Array(list)) => list,
            _ => &[],
        };
        for (index, (item, item_schema)) in data.iter().zip(prefix).enumerate() {
            self.child(&index.to_string(), |run| run.node(item_schema, root, item));
        }
        match schema.get("items") {
            Some(Value::Bool(false)) => {
                if data.len() > prefix.len() {
                    let limit = Value::from(prefix.len());
                    self.push_error(
                        "items",
                        format!("must NOT have more than {limit} items"),
                        vec![("limit", limit)],
                        Some(&Value::Bool(false)),
                    );
                }
            }
            Some(items @ Value::Object(_)) => {
                for (index, item) in data.iter().enumerate().skip(prefix.len()) {
                    self.child(&index.to_string(), |run| run.node(items, root, item));
                }
            }
            _ => {}
        }
    }

    fn object(&mut self, schema: &Map<String, Value>, root: &Value, data: &Map<String, Value>) {
        let size = data.len() as u64;
        self.limit(schema, "maxProperties", size, "properties");
        self.limit(schema, "minProperties", size, "properties");
        if let Some(required @ Value::Array(names)) = schema.get("required") {
            for name in names.iter().filter_map(Value::as_str) {
                if !data.contains_key(name) {
                    self.push_error(
                        "required",
                        format!("must have required property '{name}'"),
                        vec![("missingProperty", Value::from(name))],
                        Some(required),
                    );
                }
            }
        }
        if let Some(names_schema) = schema.get("propertyNames") {
            for key in data.keys() {
                let name = Value::from(key.as_str());
                let inner = self.probe(|run| run.node(names_schema, root, &name));
                if !inner.is_empty() {
                    self.errors.extend(inner);
                    self.push_error(
                        "propertyNames",
                        "property name must be valid".into(),
                        vec![("propertyName", name)],
                        Some(names_schema),
                    );
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(additional) = schema.get("additionalProperties") {
            for (key, value) in data {
                if properties.is_some_and(|p| p.contains_key(key)) {
                    continue;
                }
                match additional {
                    Value::Bool(false) => self.push_error(
                        "additionalProperties",
                        "must NOT have additional properties".into(),
                        vec![("additionalProperty", Value::from(key.as_str()))],
                        Some(additional),
                    ),
                    Value::Object(_) => {
                        self.child(key, |run| run.node(additional, root, value));
                    }
                    _ => {}
                }
            }
        }
        if let Some(properties) = properties {
            // ajv walks `properties` in the schema's text order; see `ORDER_KEY`.
            let order: Vec<&str> = match schema.get(ORDER_KEY) {
                Some(Value::Array(keys)) => keys.iter().filter_map(Value::as_str).collect(),
                _ => properties.keys().map(String::as_str).collect(),
            };
            for key in order {
                if let (Some(property_schema), Some(value)) = (properties.get(key), data.get(key)) {
                    self.child(key, |run| run.node(property_schema, root, value));
                }
            }
        }
    }
}

/// A synthetic sibling of `properties` that records its keys in text order. `serde_json` sorts
/// object keys unless `preserve_order` is on, which the embedded schemas must not depend on.
const ORDER_KEY: &str = "x-properties-order";

/// A `Value` that also remembers the key order of the object it was read from.
struct Ordered(Value, Vec<String>);

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Ordered;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Ordered, E> {
                Ok(Ordered(Value::Bool(v), Vec::new()))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Ordered, E> {
                Ok(Ordered(Value::from(v), Vec::new()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Ordered, E> {
                Ok(Ordered(Value::from(v), Vec::new()))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Ordered, E> {
                Ok(Ordered(Value::from(v), Vec::new()))
            }
            fn visit_str<E>(self, v: &str) -> Result<Ordered, E> {
                Ok(Ordered(Value::from(v), Vec::new()))
            }
            fn visit_unit<E>(self) -> Result<Ordered, E> {
                Ok(Ordered(Value::Null, Vec::new()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Ordered, A::Error> {
                let mut items = Vec::new();
                while let Some(Ordered(item, _)) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Ordered(Value::Array(items), Vec::new()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Ordered, A::Error> {
                let mut object = Map::new();
                let mut keys = Vec::new();
                let mut properties_order = None;
                while let Some(key) = map.next_key::<String>()? {
                    let Ordered(value, child_keys) = map.next_value()?;
                    if key == "properties" && value.is_object() {
                        properties_order = Some(child_keys);
                    }
                    keys.push(key.clone());
                    object.insert(key, value);
                }
                if let Some(order) = properties_order {
                    let order = order.into_iter().map(Value::from).collect();
                    object.insert(ORDER_KEY.into(), Value::Array(order));
                }
                Ok(Ordered(Value::Object(object), keys))
            }
        }
        deserializer.deserialize_any(Visit)
    }
}

fn parse_schema(source: &str) -> Value {
    let Ordered(value, _) = serde_json::from_str(source).expect("embedded schema is JSON");
    value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn registry(schema: Value) -> Registry {
        Registry::new(&[&schema.to_string()])
    }

    const ID: &str = "https://example.test/s.json";

    fn errors(mut schema: Value, data: Value) -> Vec<(String, String)> {
        schema["$id"] = json!(ID);
        registry(schema)
            .validate(ID, &data)
            .into_iter()
            .map(|e| (e.keyword.to_owned(), e.instance_path))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, p)| ((*k).into(), (*p).into()))
            .collect()
    }

    #[test]
    fn integers_are_f64_with_an_integral_check() {
        let s = json!({"type": "integer"});
        assert!(errors(s.clone(), json!(3)).is_empty());
        assert!(errors(s.clone(), json!(3.0)).is_empty());
        assert!(errors(s.clone(), json!(1e3)).is_empty());
        assert_eq!(errors(s.clone(), json!(3.5)), pairs(&[("type", "")]));
        assert_eq!(errors(s, json!("3")), pairs(&[("type", "")]));
        assert!(errors(json!({"type": "number"}), json!(true)).len() == 1);
    }

    #[test]
    fn string_length_counts_code_points() {
        // One astral character is 2 UTF-16 units, 4 UTF-8 bytes, 1 code point.
        let s = json!({"type": "string", "maxLength": 1, "minLength": 1});
        assert!(errors(s.clone(), json!("\u{1F600}")).is_empty());
        assert_eq!(errors(s.clone(), json!("ab")), pairs(&[("maxLength", "")]));
        assert_eq!(errors(s, json!("")), pairs(&[("minLength", "")]));
    }

    #[test]
    fn collects_all_errors_in_ajv_order() {
        let s = json!({
            "type": "object", "additionalProperties": false, "required": ["a", "b"],
            "properties": {"a": {"type": "string"}, "c": {"type": "number", "minimum": 5}}
        });
        let got = errors(s, json!({"a": 1, "c": 2, "z": 0}));
        assert_eq!(
            got,
            pairs(&[
                ("required", ""),
                ("additionalProperties", ""),
                ("type", "/a"),
                ("minimum", "/c")
            ])
        );
    }

    #[test]
    fn paths_escape_pointer_characters() {
        let s = json!({"type": "object", "additionalProperties": {"type": "string"}});
        assert_eq!(
            errors(s, json!({"a/b~c": 1})),
            pairs(&[("type", "/a~1b~0c")])
        );
    }

    #[test]
    fn prefix_items_and_items_false() {
        let s = json!({"type": "array", "prefixItems": [{"type": "number"}, {"type": "number"}],
                       "items": false, "minItems": 2, "maxItems": 2});
        assert!(errors(s.clone(), json!([1, 2])).is_empty());
        assert_eq!(
            errors(s.clone(), json!(["x", 2, 3])),
            pairs(&[("maxItems", ""), ("type", "/0"), ("items", "")])
        );
        assert_eq!(errors(s.clone(), json!([1])), pairs(&[("minItems", "")]));
        let e =
            Registry::new(&[&json!({"$id": ID, "prefixItems": [{}], "items": false}).to_string()])
                .validate(ID, &json!([1, 2, 3]));
        assert_eq!(e[0].message, "must NOT have more than 1 items");
        assert_eq!(e[0].param("limit"), Some(&json!(1)));
    }

    #[test]
    fn items_after_prefix_apply_to_the_rest() {
        let s = json!({"prefixItems": [{"type": "string"}], "items": {"type": "number"}});
        assert_eq!(errors(s, json!(["a", 1, "b"])), pairs(&[("type", "/2")]));
    }

    #[test]
    fn const_enum_and_deep_equality() {
        assert!(errors(json!({"const": 1}), json!(1.0)).is_empty());
        assert_eq!(
            errors(json!({"const": 1}), json!(2)),
            pairs(&[("const", "")])
        );
        assert!(errors(json!({"enum": ["a", {"x": [1]}]}), json!({"x": [1.0]})).is_empty());
        assert_eq!(
            errors(json!({"enum": ["a"]}), json!("b")),
            pairs(&[("enum", "")])
        );
    }

    #[test]
    fn composition_keywords() {
        let one_of = json!({"oneOf": [{"type": "string"}, {"type": "number"}]});
        assert!(errors(one_of.clone(), json!("x")).is_empty());
        assert_eq!(
            errors(one_of, json!(true)),
            pairs(&[("type", ""), ("type", ""), ("oneOf", "")])
        );
        let both = json!({"oneOf": [{"type": "number"}, {"minimum": 0}]});
        assert_eq!(errors(both, json!(1)), pairs(&[("oneOf", "")]));
        let any_of = json!({"anyOf": [{"type": "string"}, {"minimum": 3}]});
        assert!(errors(any_of.clone(), json!(5)).is_empty());
        assert_eq!(
            errors(any_of, json!(1)),
            pairs(&[("type", ""), ("minimum", ""), ("anyOf", "")])
        );
        let not = json!({"not": {"pattern": "^a"}});
        assert_eq!(errors(not.clone(), json!("abc")), pairs(&[("not", "")]));
        assert!(errors(not, json!("xbc")).is_empty());
        let all_of = json!({"allOf": [{"minimum": 1}, {"maximum": 3}]});
        assert_eq!(errors(all_of, json!(9)), pairs(&[("maximum", "")]));
    }

    #[test]
    fn property_names_report_inner_errors_then_their_own() {
        let s = json!({"propertyNames": {"pattern": "^[a-z]+$"}});
        assert_eq!(
            errors(s, json!({"ok": 1, "Bad": 2})),
            pairs(&[("pattern", ""), ("propertyNames", "")])
        );
    }

    #[test]
    fn number_limits_and_messages() {
        let e = Registry::new(&[
            &json!({"$id": ID, "minimum": 0.5, "exclusiveMinimum": 0, "maximum": 10}).to_string(),
        ])
        .validate(ID, &json!(-1));
        let msgs: Vec<_> = e.iter().map(|e| (e.keyword, e.message.as_str())).collect();
        assert_eq!(
            msgs,
            [
                ("minimum", "must be >= 0.5"),
                ("exclusiveMinimum", "must be > 0")
            ]
        );
        assert_eq!(e[0].param("comparison"), Some(&json!(">=")));
    }

    #[test]
    fn min_max_properties() {
        let s = json!({"minProperties": 1, "maxProperties": 1});
        assert_eq!(
            errors(s.clone(), json!({})),
            pairs(&[("minProperties", "")])
        );
        assert_eq!(
            errors(s, json!({"a": 1, "b": 2})),
            pairs(&[("maxProperties", "")])
        );
    }

    #[test]
    fn a_failed_type_skips_the_keywords_of_other_types() {
        let s = json!({"type": "string", "pattern": "^a", "minimum": 3});
        assert_eq!(errors(s, json!(7)), pairs(&[("type", "")]));
    }

    #[test]
    fn refs_resolve_locally_and_across_resources_by_id() {
        let common = json!({"$id": "https://example.test/s/common.json",
                            "$defs": {"id": {"type": "string", "pattern": "^[a-z]+$"}}});
        let main = json!({"$id": "https://example.test/s/main.json",
                          "$defs": {"local": {"type": "number"}},
                          "properties": {"a": {"$ref": "common.json#/$defs/id"},
                                         "b": {"$ref": "#/$defs/local"}}});
        let reg = Registry::new(&[&common.to_string(), &main.to_string()]);
        let got: Vec<_> = reg
            .validate(
                "https://example.test/s/main.json",
                &json!({"a": "A", "b": "x"}),
            )
            .into_iter()
            .map(|e| (e.keyword, e.instance_path))
            .collect();
        assert_eq!(
            got,
            [("pattern", "/a".to_owned()), ("type", "/b".to_owned())]
        );
    }

    #[test]
    fn js_patterns_translate() {
        let re = compile_js_pattern("[\\\\<>:\"|?*\\u0000-\\u001F\\u007F-\\u009F]").unwrap();
        assert!(re.is_match("a\u{1f}b") && re.is_match("a\u{85}") && re.is_match("a:b"));
        assert!(!re.is_match("abc"));
        let surrogates = compile_js_pattern("[\\uD800-\\uDFFF]").unwrap();
        assert!(!surrogates.is_match("any \u{1F600} text"));
        // An escaped backslash before `u` is not a unicode escape.
        assert!(compile_js_pattern("\\\\u0041").unwrap().is_match("\\u0041"));
    }

    #[test]
    fn false_schema_is_an_error() {
        assert_eq!(
            errors(json!({"properties": {"a": false}}), json!({"a": 1})),
            pairs(&[("false schema", "/a")])
        );
    }
}
