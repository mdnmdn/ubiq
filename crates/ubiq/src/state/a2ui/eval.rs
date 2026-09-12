//! Bindings: the three shapes a surface's property can take, and what each one evaluates to.
//!
//! **A2UI property is not a value.** It is a literal, a pointer into the data model, or a call into
//! the protocol's small function library — and the three are told apart by shape alone, with no tag
//! to match on. That discrimination is the whole reason this module hand-writes a `Deserialize`
//! rather than deriving `#[serde(untagged)]`: untagged tries the arms in order and a call object
//! matches a literal `serde_json::Value` every time, so every function in every payload was being
//! swallowed into a literal and drawn as its own JSON.
//!
//! **A relative path is relative to the item being drawn.** A template repeats one component over
//! an array, and the properties inside it name fields of the element rather than of the model, so
//! evaluation carries a [`Scope`] saying which element that is.
//!
//! **Nothing here fails and nothing here acts.** An unknown function, an unresolvable path, an
//! uncompilable pattern: each yields a blank rather than an error, because the renderer's rule
//! everywhere is that a payload it does not fully understand still draws. And `openUrl` evaluates
//! to nothing — what an action *does* is the caller's decision, never this module's.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::value;

/// A property's value: written out, bound to a pointer, or computed by a function.
#[derive(Debug, Clone)]
pub enum Dynamic {
    Literal(Value),
    Path(String),
    Call(Box<Call>),
}

/// One call into the protocol's function library.
///
/// `catalog_id` names the catalog a function belongs to when a payload says so; the basic catalog
/// is implied and nothing here dispatches on it yet.
#[derive(Debug, Clone)]
pub struct Call {
    pub call: String,
    pub args: BTreeMap<String, Dynamic>,
    pub catalog_id: Option<String>,
}

impl<'de> Deserialize<'de> for Dynamic {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Dynamic::from_json(Value::deserialize(deserializer)?))
    }
}

impl<'de> Deserialize<'de> for Call {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let object = value.as_object().and_then(Call::from_object);
        object.ok_or_else(|| serde::de::Error::custom("not a function call"))
    }
}

impl Dynamic {
    /// Tell the three shapes apart, in the one order that does not confuse them.
    ///
    /// A binding is an object whose *only* key is a string `path`; the sole-key test is what keeps
    /// a literal that happens to carry a `path` field from being read as one. A call is an object
    /// with a string `call`, and anything left is written out as it stands.
    fn from_json(value: Value) -> Dynamic {
        let Some(object) = value.as_object() else {
            return Dynamic::Literal(value);
        };
        if object.len() == 1
            && let Some(Value::String(path)) = object.get("path")
        {
            return Dynamic::Path(path.clone());
        }
        match Call::from_object(object) {
            Some(call) => Dynamic::Call(Box::new(call)),
            None => Dynamic::Literal(value),
        }
    }
}

impl Call {
    /// Read a call out of an object, recursing into every argument.
    fn from_object(object: &Map<String, Value>) -> Option<Call> {
        let Some(Value::String(name)) = object.get("call") else {
            return None;
        };
        let args = object
            .get("args")
            .and_then(Value::as_object)
            .map(|args| {
                args.iter()
                    .map(|(key, value)| (key.clone(), Dynamic::from_json(value.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let catalog_id = match object.get("catalogId") {
            Some(Value::String(id)) => Some(id.clone()),
            _ => None,
        };
        Some(Call {
            call: name.clone(),
            args,
            catalog_id,
        })
    }
}

/// Where a path resolves from: an absolute pointer is itself, a relative one hangs off the
/// template item being drawn.
#[derive(Clone, Copy, Default, Debug)]
pub struct Scope<'a> {
    pub item: Option<&'a str>,
    pub index: Option<usize>,
}

impl<'a> Scope<'a> {
    /// The absolute pointer this path names from here.
    pub fn resolve(&self, path: &str) -> String {
        if path.starts_with('/') {
            return path.to_string();
        }
        match self.item {
            Some(item) => format!("{}/{}", item.trim_end_matches('/'), path),
            None => format!("/{path}"),
        }
    }

    /// The item pointer, or the empty string outside a template.
    ///
    /// Surfaces key per-item interface state — which tab, which draft — by this, and the empty
    /// string is the key of the one instance a non-repeating component has.
    pub fn key(&self) -> &str {
        self.item.unwrap_or("")
    }
}

impl Dynamic {
    /// What this property is worth against this model.
    pub fn eval(&self, model: &Value, scope: Scope<'_>) -> Value {
        match self {
            Dynamic::Literal(value) => value.clone(),
            Dynamic::Path(path) => value::get(model, &scope.resolve(path))
                .cloned()
                .unwrap_or(Value::Null),
            Dynamic::Call(call) => self::call(&call.call, &call.args, model, scope),
        }
    }

    /// What to draw: null and an unresolved path become `""`, an object or array becomes its
    /// compact JSON.
    pub fn as_display_string(&self, model: &Value, scope: Scope<'_>) -> String {
        display(Some(&self.eval(model, scope)))
    }

    /// Whether this reads as true: a bool is itself, a number is non-zero, a string is non-empty
    /// and not `"false"`, an array or object is true, and null or an unresolved path is false.
    ///
    /// The string rule is the one that matters in practice — agents author `"true"` and `"false"`
    /// into string fields constantly, and a renderer that called both truthy would draw the wrong
    /// half of every conditional.
    pub fn as_bool(&self, model: &Value, scope: Scope<'_>) -> bool {
        truthy(&self.eval(model, scope))
    }

    /// This as a number, reading a numeric string as one.
    pub fn as_f64(&self, model: &Value, scope: Scope<'_>) -> Option<f64> {
        number(&self.eval(model, scope))
    }

    /// This as a list of strings.
    ///
    /// A bare string yields one element: the catalog asks for a list wherever a selection can be
    /// multiple, and agents write the single value instead often enough that refusing it would
    /// blank a working picker.
    pub fn as_string_list(&self, model: &Value, scope: Scope<'_>) -> Vec<String> {
        match self.eval(model, scope) {
            Value::Null => Vec::new(),
            Value::String(text) => vec![text],
            Value::Array(items) => items.iter().map(|item| display(Some(item))).collect(),
            other => vec![display(Some(&other))],
        }
    }

    /// The absolute pointer a keystroke writes to, when there is one.
    ///
    /// Only a path is writable: a literal has nowhere to put the edit and a call is derived.
    pub fn write_target(&self, scope: Scope<'_>) -> Option<String> {
        match self {
            Dynamic::Path(path) => Some(scope.resolve(path)),
            _ => None,
        }
    }

    /// The pointer a bound property names, for the affordance drawn under an input.
    pub fn path(&self) -> Option<&str> {
        match self {
            Dynamic::Path(path) => Some(path.as_str()),
            _ => None,
        }
    }
}

/// How any value reads as a string.
fn display(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(other) => other.to_string(),
    }
}

/// How any value reads as a condition. See [`Dynamic::as_bool`] for the rules and why.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().map(|n| n != 0.0).unwrap_or(false),
        Value::String(text) => !text.is_empty() && text != "false",
        _ => true,
    }
}

/// How any value reads as a number, a numeric string included.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Read a condition's result the way a renderer must: a `ValidationResult` object is itself, and a
/// bare boolean is read as `{valid: that}` — agents author that constantly.
pub fn validation(value: &Value) -> (bool, Option<String>) {
    if let Some(object) = value.as_object()
        && let Some(valid) = object.get("valid")
    {
        let message = match object.get("message") {
            Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
            _ => None,
        };
        return (truthy(valid), message);
    }
    (truthy(value), None)
}

/// A `ValidationResult`, the shape every check in the library returns.
fn result(valid: bool, message: Option<&str>) -> Value {
    let mut object = Map::new();
    object.insert("valid".to_string(), Value::Bool(valid));
    object.insert(
        "message".to_string(),
        match message {
            Some(text) => Value::String(text.to_string()),
            None => Value::Null,
        },
    );
    Value::Object(object)
}

/// One argument, when the payload gives it.
fn arg<'a>(args: &'a BTreeMap<String, Dynamic>, name: &str) -> Option<&'a Dynamic> {
    args.get(name)
}

/// One argument evaluated, or null when it is absent.
fn value_of(
    args: &BTreeMap<String, Dynamic>,
    name: &str,
    model: &Value,
    scope: Scope<'_>,
) -> Value {
    arg(args, name)
        .map(|dynamic| dynamic.eval(model, scope))
        .unwrap_or(Value::Null)
}

/// One argument as a string, or `""` when it is absent.
fn string_of(
    args: &BTreeMap<String, Dynamic>,
    name: &str,
    model: &Value,
    scope: Scope<'_>,
) -> String {
    display(Some(&value_of(args, name, model, scope)))
}

/// One argument as a number, when it is present and reads as one.
fn number_of(
    args: &BTreeMap<String, Dynamic>,
    name: &str,
    model: &Value,
    scope: Scope<'_>,
) -> Option<f64> {
    arg(args, name).and_then(|dynamic| number(&dynamic.eval(model, scope)))
}

/// Call one function of the library.
///
/// An unknown name evaluates to null rather than erroring: a payload may name a function from a
/// catalog this build does not carry, and the protocol asks a renderer to degrade — a blank
/// property rather than a refused surface.
pub fn call(
    name: &str,
    args: &BTreeMap<String, Dynamic>,
    model: &Value,
    scope: Scope<'_>,
) -> Value {
    match name {
        "required" => required(args, model, scope),
        "regex" => regex_check(args, model, scope),
        "length" => length(args, model, scope),
        "numeric" => numeric(args, model, scope),
        "email" => email(args, model, scope),
        "formatString" => Value::String(format_string(args, model, scope)),
        "formatNumber" => Value::String(format_number(args, model, scope)),
        "formatCurrency" => Value::String(format_currency(args, model, scope)),
        "formatDate" => Value::String(format_date(args, model, scope)),
        "pluralize" => Value::String(pluralize(args, model, scope)),
        "and" => Value::Bool(logic(args, model, scope, true)),
        "or" => Value::Bool(logic(args, model, scope, false)),
        "not" => Value::Bool(!truthy(&value_of(args, "value", model, scope))),
        "openUrl" => Value::Null,
        "@index" => index(args, model, scope),
        _ => Value::Null,
    }
}

/// Invalid when the value is null, absent, the empty string, or an empty array.
fn required(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> Value {
    let value = value_of(args, "value", model, scope);
    let present = match &value {
        Value::Null => false,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => true,
    };
    result(present, (!present).then_some("This field is required"))
}

/// Invalid when the value does not match the pattern.
///
/// An uncompilable pattern is invalid with a message saying so: the fault is the payload's, and a
/// person looking at the surface should see which pattern was refused.
fn regex_check(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> Value {
    let subject = string_of(args, "value", model, scope);
    let pattern = string_of(args, "pattern", model, scope);
    match regex::Regex::new(&pattern) {
        Ok(compiled) => {
            let matched = compiled.is_match(&subject);
            result(matched, (!matched).then_some("Invalid format"))
        }
        Err(_) => result(false, Some("The validation pattern is not a valid regex")),
    }
}

/// Invalid when a string's character count, or an array's length, falls outside the bounds given.
///
/// Both bounds are optional and neither present passes — a payload that names no bound is asking
/// nothing.
fn length(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> Value {
    let value = value_of(args, "value", model, scope);
    let measured = match &value {
        Value::Array(items) => items.len(),
        Value::Null => 0,
        Value::String(text) => text.chars().count(),
        other => display(Some(other)).chars().count(),
    } as f64;
    bounds(args, model, scope, measured, "Length")
}

/// Invalid when the value, read as a number, falls outside the bounds given.
fn numeric(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> Value {
    let Some(measured) = number(&value_of(args, "value", model, scope)) else {
        return result(false, Some("Enter a number"));
    };
    bounds(args, model, scope, measured, "Value")
}

/// The bound check `length` and `numeric` share.
fn bounds(
    args: &BTreeMap<String, Dynamic>,
    model: &Value,
    scope: Scope<'_>,
    measured: f64,
    noun: &str,
) -> Value {
    let min = number_of(args, "min", model, scope);
    let max = number_of(args, "max", model, scope);
    if let Some(min) = min
        && measured < min
    {
        return result(false, Some(&format!("{noun} must be at least {min}")));
    }
    if let Some(max) = max
        && measured > max
    {
        return result(false, Some(&format!("{noun} must be at most {max}")));
    }
    result(true, None)
}

/// Invalid when the value is not shaped like an address.
///
/// This is a structural check — one `@`, something either side, a dot in the domain, no spaces —
/// rather than RFC 5322, which no useful form wants and no small function delivers.
fn email(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> Value {
    let subject = string_of(args, "value", model, scope);
    let mut parts = subject.split('@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    let shaped = parts.next().is_none()
        && !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !subject.chars().any(char::is_whitespace);
    result(shaped, (!shaped).then_some("Enter a valid email address"))
}

/// Fill `${…}` placeholders in the value, each one a pointer read through the scope.
///
/// `$${` is a literal `${`, an unresolved placeholder is the empty string, and an unterminated
/// `${` is left exactly as it was written rather than swallowing the rest of the string.
fn format_string(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> String {
    let template: Vec<char> = string_of(args, "value", model, scope).chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < template.len() {
        if template[at] == '$'
            && template.get(at + 1) == Some(&'$')
            && template.get(at + 2) == Some(&'{')
        {
            out.push_str("${");
            at += 3;
            continue;
        }
        if template[at] == '$' && template.get(at + 1) == Some(&'{') {
            match template[at + 2..].iter().position(|c| *c == '}') {
                Some(offset) => {
                    let end = at + 2 + offset;
                    let pointer: String = template[at + 2..end].iter().collect();
                    let resolved = scope.resolve(pointer.trim());
                    out.push_str(&display(value::get(model, &resolved)));
                    at = end + 1;
                }
                None => {
                    out.extend(template[at..].iter());
                    break;
                }
            }
            continue;
        }
        out.push(template[at]);
        at += 1;
    }
    out
}

/// The most decimal places an unspecified `decimals` will print.
const MAX_DECIMALS: usize = 6;

/// The value as a number, with the decimals and thousands grouping asked for.
fn format_number(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> String {
    let Some(value) = number(&value_of(args, "value", model, scope)) else {
        return String::new();
    };
    let decimals = number_of(args, "decimals", model, scope)
        .map(|places| places.max(0.0).min(MAX_DECIMALS as f64) as usize)
        .unwrap_or_else(|| natural_decimals(value));
    let grouping = arg(args, "grouping").is_none_or(|flag| truthy(&flag.eval(model, scope)));
    render_number(value, decimals, grouping)
}

/// The value as an amount of money, prefixed with a symbol when the code has a well-known one.
fn format_currency(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> String {
    let Some(value) = number(&value_of(args, "value", model, scope)) else {
        return String::new();
    };
    let code = string_of(args, "currency", model, scope);
    let (symbol, default_decimals) = match code.as_str() {
        "USD" => ("$".to_string(), 2),
        "EUR" => ("€".to_string(), 2),
        "GBP" => ("£".to_string(), 2),
        "JPY" => ("¥".to_string(), 0),
        "" => (String::new(), 2),
        other => (format!("{other} "), 2),
    };
    let decimals = number_of(args, "decimals", model, scope)
        .map(|places| places.max(0.0).min(MAX_DECIMALS as f64) as usize)
        .unwrap_or(default_decimals);
    let grouping = arg(args, "grouping").is_none_or(|flag| truthy(&flag.eval(model, scope)));
    let body = render_number(value.abs(), decimals, grouping);
    let sign = if value < 0.0 { "-" } else { "" };
    format!("{sign}{symbol}{body}")
}

/// How many decimals this number needs to print without losing anything, within reason.
fn natural_decimals(value: f64) -> usize {
    let text = format!("{value}");
    text.split_once('.')
        .map(|(_, fraction)| fraction.len().min(MAX_DECIMALS))
        .unwrap_or(0)
}

/// A number rounded to a fixed number of places, with `,` every three digits when asked.
fn render_number(value: f64, decimals: usize, grouping: bool) -> String {
    let text = format!("{value:.decimals$}");
    if !grouping {
        return text;
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.as_str()),
    };
    let (whole, fraction) = match rest.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (rest, None),
    };
    let mut grouped = String::new();
    for (position, digit) in whole.chars().enumerate() {
        if position > 0 && (whole.len() - position) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

/// The value as a date, in the pattern given.
///
/// The value is read as RFC 3339 first and then as `%Y-%m-%d`; an unparseable one yields `""`.
/// The pattern understands a handful of tokens — `yyyy yy MM dd HH mm ss` — rather than CLDR, and
/// passes every other character through as written.
fn format_date(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> String {
    let raw = string_of(args, "value", model, scope);
    let Some(moment) = parse_moment(raw.trim()) else {
        return String::new();
    };
    let pattern = string_of(args, "format", model, scope);
    let pattern = if pattern.is_empty() {
        "yyyy-MM-dd".to_string()
    } else {
        pattern
    };
    moment.format(&chrono_pattern(&pattern)).to_string()
}

/// Read a moment out of the two forms agents actually send.
fn parse_moment(raw: &str) -> Option<NaiveDateTime> {
    if let Ok(moment) = DateTime::parse_from_rfc3339(raw) {
        return Some(moment.naive_local());
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
}

/// Rewrite the protocol's tokens as chrono's, escaping everything else.
fn chrono_pattern(pattern: &str) -> String {
    const TOKENS: &[(&str, &str)] = &[
        ("yyyy", "%Y"),
        ("yy", "%y"),
        ("MM", "%m"),
        ("dd", "%d"),
        ("HH", "%H"),
        ("mm", "%M"),
        ("ss", "%S"),
    ];
    let mut out = String::new();
    let mut rest = pattern;
    'outer: while !rest.is_empty() {
        for (token, specifier) in TOKENS {
            if let Some(tail) = rest.strip_prefix(token) {
                out.push_str(specifier);
                rest = tail;
                continue 'outer;
            }
        }
        let mut chars = rest.chars();
        if let Some(character) = chars.next() {
            if character == '%' {
                out.push_str("%%");
            } else {
                out.push(character);
            }
        }
        rest = chars.as_str();
    }
    out
}

/// The wording for a count.
///
/// English categories only — `one` at exactly 1, `zero` at 0 when the payload gives one, `other`
/// otherwise — rather than CLDR's plural rules. `two`, `few` and `many` are accepted so a payload
/// written for another language still parses, but nothing here ever selects them.
fn pluralize(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> String {
    let count = number(&value_of(args, "value", model, scope)).unwrap_or(0.0);
    let pick = |name: &str| arg(args, name).map(|form| form.as_display_string(model, scope));
    let other = pick("other").unwrap_or_default();
    if count == 1.0 {
        return pick("one").unwrap_or(other);
    }
    if count == 0.0 {
        return pick("zero").unwrap_or(other);
    }
    other
}

/// `and` and `or` over a list, each element read with the same truthiness as a condition.
///
/// An empty list is the identity of the operation: `and` of nothing is true, `or` of nothing false.
fn logic(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>, all: bool) -> bool {
    let values = value_of(args, "values", model, scope);
    let items = match &values {
        Value::Array(items) => items.as_slice(),
        _ => &[],
    };
    if all {
        items.iter().all(truthy)
    } else {
        items.iter().any(truthy)
    }
}

/// The position of the template item being drawn, plus an optional offset.
///
/// Outside a template there is no position, so this is null rather than 0 — a surface that shows
/// `@index` where nothing repeats is saying something wrong, and a blank says so.
fn index(args: &BTreeMap<String, Dynamic>, model: &Value, scope: Scope<'_>) -> Value {
    let Some(position) = scope.index else {
        return Value::Null;
    };
    let offset = number_of(args, "offset", model, scope).unwrap_or(0.0);
    Value::from(position as f64 + offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(source: Value) -> Dynamic {
        serde_json::from_value(source).expect("a dynamic parses from any JSON")
    }

    fn eval(source: Value, model: &Value) -> Value {
        parse(source).eval(model, Scope::default())
    }

    #[test]
    fn the_three_shapes_are_told_apart() {
        assert!(matches!(parse(json!({"path": "/a"})), Dynamic::Path(_)));
        assert!(matches!(parse(json!("plain")), Dynamic::Literal(_)));
        assert!(matches!(parse(json!(7)), Dynamic::Literal(_)));

        // A call is not swallowed into a literal, which is what untagged deserialisation did.
        let Dynamic::Call(call) = parse(json!({"call": "required", "args": {"value": 1}})) else {
            panic!("a call object must parse as a call");
        };
        assert_eq!(call.call, "required");
        assert!(call.args.contains_key("value"));

        // `path` alongside anything else is a literal: the sole-key test is the discriminator.
        assert!(matches!(
            parse(json!({"path": "/a", "other": 1})),
            Dynamic::Literal(_)
        ));
        assert!(matches!(parse(json!({"path": 7})), Dynamic::Literal(_)));
    }

    #[test]
    fn calls_carry_their_catalog_and_nest() {
        let Dynamic::Call(call) = parse(json!({
            "call": "not",
            "catalogId": "basic",
            "args": {"value": {"call": "required", "args": {"value": {"path": "/a"}}}}
        })) else {
            panic!("a call object must parse as a call");
        };
        assert_eq!(call.catalog_id.as_deref(), Some("basic"));
        assert!(matches!(call.args.get("value"), Some(Dynamic::Call(_))));
    }

    #[test]
    fn scope_resolves_the_three_cases() {
        let inside = Scope {
            item: Some("/rows/2"),
            index: Some(2),
        };
        assert_eq!(inside.resolve("/x"), "/x");
        assert_eq!(inside.resolve("x"), "/rows/2/x");
        assert_eq!(Scope::default().resolve("x"), "/x");
        assert_eq!(inside.key(), "/rows/2");
        assert_eq!(Scope::default().key(), "");
    }

    #[test]
    fn paths_read_through_the_scope() {
        let model = json!({"rows": [{"name": "ada"}, {"name": "grace"}]});
        let binding = parse(json!({"path": "name"}));
        let scope = Scope {
            item: Some("/rows/1"),
            index: Some(1),
        };
        assert_eq!(binding.as_display_string(&model, scope), "grace");
        assert_eq!(binding.as_display_string(&model, Scope::default()), "");
        assert_eq!(binding.write_target(scope).as_deref(), Some("/rows/1/name"));
        assert_eq!(binding.path(), Some("name"));
        assert_eq!(parse(json!("x")).write_target(scope), None);
    }

    #[test]
    fn conversions_follow_the_stated_rules() {
        let model = json!({});
        let of = |source: Value| parse(source).as_bool(&model, Scope::default());
        assert!(of(json!(true)));
        assert!(!of(json!(false)));
        assert!(of(json!(3)));
        assert!(!of(json!(0)));
        assert!(of(json!("yes")));
        assert!(!of(json!("")));
        assert!(!of(json!("false")));
        assert!(!of(json!(null)));

        assert_eq!(
            parse(json!("2.5")).as_f64(&model, Scope::default()),
            Some(2.5)
        );
        assert_eq!(
            parse(json!("red")).as_string_list(&model, Scope::default()),
            vec!["red".to_string()]
        );
        assert_eq!(
            parse(json!(["red", 2])).as_string_list(&model, Scope::default()),
            vec!["red".to_string(), "2".to_string()]
        );
        assert_eq!(
            parse(json!({"a": 1})).as_display_string(&model, Scope::default()),
            "{\"a\":1}"
        );
    }

    #[test]
    fn index_needs_a_template_scope() {
        let model = json!({});
        let inside = Scope {
            item: Some("/rows/2"),
            index: Some(2),
        };
        assert_eq!(eval(json!({"call": "@index"}), &model), Value::Null);
        assert_eq!(
            parse(json!({"call": "@index"})).eval(&model, inside),
            json!(2.0)
        );
        assert_eq!(
            parse(json!({"call": "@index", "args": {"offset": 1}})).eval(&model, inside),
            json!(3.0)
        );
    }

    #[test]
    fn validations_answer_with_a_result() {
        let model = json!({"name": "", "age": 40});
        let valid = |source: Value| validation(&eval(source, &model)).0;

        assert!(!valid(
            json!({"call": "required", "args": {"value": {"path": "/name"}}})
        ));
        assert!(valid(json!({"call": "required", "args": {"value": "ada"}})));
        assert!(!valid(json!({"call": "required", "args": {"value": []}})));

        assert!(valid(
            json!({"call": "regex", "args": {"value": "abc", "pattern": "^a"}})
        ));
        let broken = eval(
            json!({"call": "regex", "args": {"value": "abc", "pattern": "("}}),
            &model,
        );
        let (ok, message) = validation(&broken);
        assert!(!ok);
        assert!(message.is_some());

        assert!(valid(
            json!({"call": "length", "args": {"value": "héllo", "max": 5}})
        ));
        assert!(!valid(
            json!({"call": "length", "args": {"value": "héllo", "max": 4}})
        ));
        assert!(valid(
            json!({"call": "length", "args": {"value": "anything"}})
        ));

        assert!(valid(
            json!({"call": "numeric", "args": {"value": {"path": "/age"}, "min": 18}})
        ));
        assert!(!valid(
            json!({"call": "numeric", "args": {"value": "10", "min": 18}})
        ));
        assert!(!valid(
            json!({"call": "numeric", "args": {"value": "ada", "min": 1}})
        ));

        assert!(valid(json!({"call": "email", "args": {"value": "a@b.co"}})));
        assert!(!valid(json!({"call": "email", "args": {"value": "a@b"}})));
        assert!(!valid(
            json!({"call": "email", "args": {"value": "a b@c.co"}})
        ));
        assert!(!valid(
            json!({"call": "email", "args": {"value": "a@b@c.co"}})
        ));
    }

    #[test]
    fn validation_reads_a_bare_boolean() {
        assert_eq!(validation(&json!(true)), (true, None));
        assert_eq!(
            validation(&json!({"valid": false, "message": "no"})),
            (false, Some("no".to_string()))
        );
        assert_eq!(validation(&json!({"valid": true})), (true, None));
    }

    #[test]
    fn format_string_interpolates_pointers() {
        let model = json!({"user": {"name": "Ada"}});
        let of = |template: &str| {
            eval(
                json!({"call": "formatString", "args": {"value": template}}),
                &model,
            )
        };
        assert_eq!(of("Hi ${/user/name}!"), json!("Hi Ada!"));
        assert_eq!(of("Hi ${/user/nope}!"), json!("Hi !"));
        assert_eq!(of("literal $${/user/name}"), json!("literal ${/user/name}"));
        assert_eq!(of("unterminated ${/user"), json!("unterminated ${/user"));

        let scoped = parse(json!({"call": "formatString", "args": {"value": "${name}"}}));
        let model = json!({"rows": [{"name": "Grace"}]});
        let scope = Scope {
            item: Some("/rows/0"),
            index: Some(0),
        };
        assert_eq!(scoped.eval(&model, scope), json!("Grace"));
    }

    #[test]
    fn numbers_dates_and_plurals_format() {
        let model = json!({});
        assert_eq!(
            eval(
                json!({"call": "formatNumber", "args": {"value": 1234567.891, "decimals": 2}}),
                &model
            ),
            json!("1,234,567.89")
        );
        assert_eq!(
            eval(
                json!({"call": "formatNumber", "args": {"value": 1234, "grouping": false}}),
                &model
            ),
            json!("1234")
        );
        assert_eq!(
            eval(
                json!({"call": "formatCurrency", "args": {"value": 1234.5, "currency": "USD"}}),
                &model
            ),
            json!("$1,234.50")
        );
        assert_eq!(
            eval(
                json!({"call": "formatCurrency", "args": {"value": 1234.6, "currency": "JPY"}}),
                &model
            ),
            json!("¥1,235")
        );
        assert_eq!(
            eval(
                json!({"call": "formatCurrency", "args": {"value": 10, "currency": "CHF"}}),
                &model
            ),
            json!("CHF 10.00")
        );
        assert_eq!(
            eval(
                json!({"call": "formatDate", "args": {"value": "2024-03-09", "format": "dd/MM/yyyy"}}),
                &model
            ),
            json!("09/03/2024")
        );
        assert_eq!(
            eval(
                json!({"call": "formatDate", "args": {"value": "2024-03-09T14:05:00Z", "format": "yyyy HH:mm"}}),
                &model
            ),
            json!("2024 14:05")
        );
        assert_eq!(
            eval(
                json!({"call": "formatDate", "args": {"value": "nope"}}),
                &model
            ),
            json!("")
        );
        let plural = |count: i64| {
            eval(
                json!({"call": "pluralize", "args": {"value": count, "one": "1 item", "other": "many", "zero": "none"}}),
                &model,
            )
        };
        assert_eq!(plural(1), json!("1 item"));
        assert_eq!(plural(0), json!("none"));
        assert_eq!(plural(5), json!("many"));
        assert_eq!(
            eval(
                json!({"call": "pluralize", "args": {"value": 0, "other": "many"}}),
                &model
            ),
            json!("many")
        );
    }

    #[test]
    fn logic_and_navigation_and_the_unknown() {
        let model = json!({"a": true, "b": false});
        assert_eq!(
            eval(
                json!({"call": "and", "args": {"values": [true, 1, "x"]}}),
                &model
            ),
            json!(true)
        );
        assert_eq!(
            eval(
                json!({"call": "and", "args": {"values": [true, ""]}}),
                &model
            ),
            json!(false)
        );
        assert_eq!(
            eval(
                json!({"call": "or", "args": {"values": [false, "false", 2]}}),
                &model
            ),
            json!(true)
        );
        assert_eq!(
            eval(json!({"call": "and", "args": {"values": []}}), &model),
            json!(true)
        );
        assert_eq!(
            eval(json!({"call": "or", "args": {"values": []}}), &model),
            json!(false)
        );
        assert_eq!(
            eval(
                json!({"call": "not", "args": {"value": {"path": "/b"}}}),
                &model
            ),
            json!(true)
        );

        assert_eq!(
            eval(
                json!({"call": "openUrl", "args": {"url": "https://example.com"}}),
                &model
            ),
            Value::Null
        );
        assert_eq!(eval(json!({"call": "notAFunction"}), &model), Value::Null);
    }
}
