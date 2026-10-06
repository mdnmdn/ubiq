//! Renderer-owned copy: Archify's built-in `en` and `zh-CN` catalogues and the `meta.translations`
//! merge (`renderers/shared/i18n.mjs`, 438 keys, dumped by `gen-i18n.mjs`), plus the V4
//! locale check (`applyLocaleTranslations`, `00` §3.1).
//!
//! Only a few keys reach a scene: `legend.title`, `legend.<type>.*` and `node.context.<type>`. The
//! whole table is embedded because V4's coverage report is computed against all of it.
//!
//! The quirks of `registerLocale` are kept, because they decide what a document paints:
//! - `meta.translations` that is non-empty makes a runtime catalogue of **English** layered with the
//!   valid entries, for any locale tag, `en` and `zh-CN` included (a partial `zh-CN` override reads
//!   English for the keys it misses, not the built-in Chinese);
//! - a key whose `{placeholders}` differ from English, or whose value is empty, is skipped;
//! - without translations, `zh-CN` is the built-in catalogue and any other tag is English.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use serde_json::{Map, Value, json};

use crate::diag::{Diagnostic, Subject, js_round};
use crate::model::common::Translations;

const RAW: &str = include_str!("data/i18n.json");

/// `SUPPORTED_LOCALES`.
pub const SUPPORTED: [&str; 2] = ["en", "zh-CN"];

struct Row {
    key: &'static str,
    en: String,
    zh: String,
    placeholders: BTreeSet<String>,
}

struct Catalogue {
    rows: Vec<Row>,
    index: HashMap<&'static str, usize>,
}

fn catalogue() -> &'static Catalogue {
    static CELL: OnceLock<Catalogue> = OnceLock::new();
    CELL.get_or_init(|| {
        let table: Vec<(String, String, String)> = serde_json::from_str(RAW).expect("data/i18n.json");
        let mut rows = Vec::with_capacity(table.len());
        let mut index = HashMap::new();
        for (key, en, zh) in table {
            // Leaked once: the table lives for the process and keys are `&'static`.
            let key: &'static str = Box::leak(key.into_boxed_str());
            index.insert(key, rows.len());
            rows.push(Row { key, placeholders: placeholders(&en), en, zh });
        }
        Catalogue { rows, index }
    })
}

/// `extractPlaceholders`: the set of `{name}` tokens, `name` being `[a-zA-Z0-9_]+`.
fn placeholders(message: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = message.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                out.insert(message[start..end].to_owned());
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The catalogue one document resolves to.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Locale {
    /// The built-in English catalogue.
    #[default]
    English,
    /// The built-in `zh-CN` catalogue.
    ZhCn,
    /// English layered with the document's valid `meta.translations`.
    Runtime(HashMap<String, String>),
}

impl Locale {
    /// What `meta.locale` and `meta.translations` resolve to (`registerLocale` then `resolveLocale`).
    pub fn of(locale: Option<&str>, translations: Option<&Translations>) -> Locale {
        let Some(tag) = locale.filter(|l| !l.is_empty()) else {
            return Locale::English;
        };
        match translations.filter(|t| !t.is_empty()) {
            Some(t) => {
                let cat = catalogue();
                let usable = t
                    .iter()
                    .filter(|(key, value)| {
                        cat.index.get(key.as_str()).is_some_and(|&i| !value.is_empty() && placeholders(value) == cat.rows[i].placeholders)
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                Locale::Runtime(usable)
            }
            None if tag == "zh-CN" => Locale::ZhCn,
            None => Locale::English,
        }
    }

    /// `translateMessage(locale, key)` for a message without placeholders. An unknown key is a bug
    /// in the caller; it reads as the key itself.
    pub fn t(&self, key: &str) -> String {
        let cat = catalogue();
        let Some(&i) = cat.index.get(key) else {
            debug_assert!(false, "missing i18n key {key}");
            return key.to_owned();
        };
        match self {
            Locale::English => cat.rows[i].en.clone(),
            Locale::ZhCn => cat.rows[i].zh.clone(),
            Locale::Runtime(overrides) => overrides.get(key).cloned().unwrap_or_else(|| cat.rows[i].en.clone()),
        }
    }

    /// `legend.title`.
    pub fn legend_title(&self) -> String {
        self.t("legend.title")
    }
}

/// The V4 warnings (`applyLocaleTranslations`): `i18n/translation-coverage` when the translations
/// miss a key, name an unknown one or break a placeholder, `i18n/locale-fallback` for a locale tag
/// with neither a built-in catalogue nor translations. Never blocking. Archify prints the message
/// on stderr as `archify: <message>`; it is not in the success receipt.
pub fn check(diagram_type: &str, locale: Option<&str>, translations: Option<&Translations>) -> Vec<Diagnostic> {
    let Some(tag) = locale.filter(|l| !l.is_empty()) else {
        return Vec::new();
    };
    let quoted = Value::String(tag.to_owned()).to_string();
    match translations.filter(|t| !t.is_empty()) {
        Some(t) => {
            let cat = catalogue();
            let missing: Vec<&str> = cat.rows.iter().map(|r| r.key).filter(|k| !t.contains_key(*k)).collect();
            let unknown: Vec<&str> = t.keys().map(String::as_str).filter(|k| !cat.index.contains_key(k)).collect();
            let mut mismatches = Vec::new();
            let mut covered = 0usize;
            for (key, value) in t {
                let Some(&i) = cat.index.get(key.as_str()) else { continue };
                let expected = &cat.rows[i].placeholders;
                let actual = placeholders(value);
                if value.is_empty() || actual != *expected {
                    mismatches.push(json!({
                        "key": key,
                        "expected": expected.iter().collect::<Vec<_>>(),
                        "actual": if value.is_empty() { Value::Null } else { json!(actual.iter().collect::<Vec<_>>()) },
                    }));
                } else {
                    covered += 1;
                }
            }
            if missing.is_empty() && unknown.is_empty() && mismatches.is_empty() {
                return Vec::new();
            }
            let total = cat.rows.len();
            let percent = js_round(covered as f64 / total as f64 * 100.0);
            let message = format!(
                "meta.translations for locale {quoted} covers {covered}/{total} renderer-owned messages ({percent}%); uncovered keys fall back to English."
            );
            let mut evidence = Map::new();
            evidence.insert("missingKeys".into(), json!(missing.iter().take(10).collect::<Vec<_>>()));
            evidence.insert("missingKeysTotal".into(), json!(missing.len()));
            evidence.insert("unknownKeys".into(), json!(unknown.iter().take(10).collect::<Vec<_>>()));
            evidence.insert("unknownKeysTotal".into(), json!(unknown.len()));
            evidence.insert("placeholderMismatches".into(), Value::Array(mismatches.iter().take(10).cloned().collect()));
            evidence.insert("placeholderMismatchesTotal".into(), json!(mismatches.len()));
            vec![
                Diagnostic::warning("i18n/translation-coverage", &message)
                    .with_subject(Subject::of(diagram_type).with_path("/meta/translations"))
                    .with_evidence(evidence)
                    .with_fixes(["Add the missing keys to meta.translations.", "Match each translation's {placeholders} to the English source string."]),
            ]
        }
        None if !SUPPORTED.contains(&tag) => {
            let message = format!(
                "meta.locale {quoted} has no built-in catalog and no meta.translations; the Viewer chrome and <html lang> fall back to English."
            );
            vec![
                Diagnostic::warning("i18n/locale-fallback", &message)
                    .with_subject(Subject::of(diagram_type).with_path("/meta/locale"))
                    .with_fixes(["Supply meta.translations for this locale.".to_owned(), format!("Use a built-in locale: {}.", SUPPORTED.join(", "))]),
            ]
        }
        None => Vec::new(),
    }
}

/// [`check`] over a typed document's `meta`.
pub fn check_doc(doc: &crate::model::Doc) -> Vec<Diagnostic> {
    use crate::model::Doc;
    let ty = doc.diagram_type();
    match doc {
        Doc::Architecture(d) => check(ty, d.meta.locale.as_deref(), d.meta.translations.as_ref()),
        Doc::Workflow(d) => check(ty, d.meta.locale.as_deref(), d.meta.translations.as_ref()),
        Doc::Sequence(d) => check(ty, d.meta.locale.as_deref(), d.meta.translations.as_ref()),
        Doc::Dataflow(d) => check(ty, d.meta.locale.as_deref(), d.meta.translations.as_ref()),
        Doc::Lifecycle(d) => check(ty, d.meta.locale.as_deref(), d.meta.translations.as_ref()),
    }
}

/// `catalogKeys().length`.
pub fn key_count() -> usize {
    catalogue().rows.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(pairs: &[(&str, &str)]) -> Translations {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn the_table_is_the_full_catalogue() {
        assert_eq!(key_count(), 438);
        assert_eq!(Locale::English.legend_title(), "Legend");
        assert_eq!(Locale::ZhCn.legend_title(), "图例");
        assert_eq!(Locale::ZhCn.t("legend.architecture.messagebus"), "消息总线");
        assert_eq!(Locale::English.t("node.context.dataflow"), "Data-flow node");
    }

    #[test]
    fn resolution_follows_register_locale() {
        assert_eq!(Locale::of(None, None), Locale::English);
        assert_eq!(Locale::of(Some("zh-CN"), None), Locale::ZhCn);
        assert_eq!(Locale::of(Some("xx"), None), Locale::English);
        // Translations layer over English, even for zh-CN.
        let t = tr(&[("legend.title", "Legenda")]);
        let l = Locale::of(Some("zh-CN"), Some(&t));
        assert_eq!(l.legend_title(), "Legenda");
        assert_eq!(l.t("legend.architecture.cloud"), "Cloud");
        // A key with the wrong placeholders or an empty value is skipped.
        let t = tr(&[("node.focus", "{nope}"), ("legend.title", "")]);
        let l = Locale::of(Some("it"), Some(&t));
        assert_eq!(l.t("node.focus"), "Focus {label}");
        assert_eq!(l.legend_title(), "Legend");
    }

    #[test]
    fn v4_warnings_say_what_archify_prints() {
        let t = tr(&[("legend.title", "Legenda")]);
        let w = check("architecture", Some("it"), Some(&t));
        assert_eq!(w.len(), 1);
        assert_eq!(
            w[0].message,
            "meta.translations for locale \"it\" covers 1/438 renderer-owned messages (0%); uncovered keys fall back to English."
        );
        let w = check("architecture", Some("xx"), None);
        assert_eq!(w[0].code, "i18n/locale-fallback");
        assert!(check("architecture", Some("zh-CN"), None).is_empty());
        assert!(check("architecture", None, None).is_empty());
    }
}
