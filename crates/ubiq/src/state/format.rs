//! Pretty-printing a buffer in place — the editor tab menu's **Format** row.
//!
//! Pure text in, text out; the app layer applies the result as one edit. Only JSON, YAML and XML
//! are formattable ([`can_format`]); JSON Lines is not (a line is its own document).
//!
//! - **JSON** is validated by `serde_json`, then re-indented token by token over the raw text, so
//!   numbers, key order and duplicate keys are untouched. Indent 2 spaces.
//! - **YAML** is a parse and re-emit through `serde_yaml`, so **comments are dropped**. Every
//!   document of a multi-document stream is kept.
//! - **XML** is a `quick-xml` event pass with 2-space indent; whitespace-only text is dropped; all other events
//!   pass through untouched.

use quick_xml::events::Event;
use quick_xml::{Reader, Writer};

use super::FileLanguage;

/// Whether the tab menu offers Format for this language.
pub fn can_format(language: FileLanguage) -> bool {
    matches!(
        language,
        FileLanguage::Json | FileLanguage::Yaml | FileLanguage::Xml
    )
}

/// Pretty-print `text` as `language`, or say why it cannot be parsed. `None` when the language is
/// not formattable.
pub fn format_text(language: FileLanguage, text: &str) -> Option<Result<String, String>> {
    Some(match language {
        FileLanguage::Json => format_json(text),
        FileLanguage::Yaml => format_yaml(text),
        FileLanguage::Xml => format_xml(text),
        _ => return None,
    })
}

pub fn format_json(text: &str) -> Result<String, String> {
    // Validate only; the output is a re-indent of the raw tokens, so numbers, key order and
    // duplicate keys come through untouched.
    serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|e| e.to_string())?;
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', depth * 2));
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    match chars[i] {
                        '\\' => {
                            i += 1;
                            if i < chars.len() {
                                out.push(chars[i]);
                            }
                        }
                        '"' => break,
                        _ => {}
                    }
                    i += 1;
                }
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                let next = chars[i + 1..].iter().position(|c| !c.is_whitespace());
                if next.is_some_and(|n| chars[i + 1 + n] == close) {
                    out.push(c);
                    out.push(close);
                    i += 1 + next.unwrap_or(0);
                } else {
                    out.push(c);
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
        i += 1;
    }
    out.push('\n');
    Ok(out)
}

pub fn format_yaml(text: &str) -> Result<String, String> {
    use serde::Deserialize;
    let mut out = String::new();
    for (i, document) in serde_yaml::Deserializer::from_str(text).enumerate() {
        let value = serde_yaml::Value::deserialize(document).map_err(|e| e.to_string())?;
        if i > 0 {
            out.push_str("---\n");
        }
        out.push_str(&serde_yaml::to_string(&value).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn format_xml(text: &str) -> Result<String, String> {
    let mut reader = Reader::from_str(text);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match &event {
            Event::Eof => break,
            // Only whitespace-only text is layout; every other event passes through untouched.
            Event::Text(t) if AsRef::<str>::as_ref(t).trim().is_empty() => continue,
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        writer.write_event(event).map_err(|e| e.to_string())?;
    }
    if depth != 0 {
        return Err("unclosed element".into());
    }
    let mut out = String::from_utf8(writer.into_inner()).map_err(|e| e.to_string())?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_indents_two_and_keeps_key_order() {
        let out = format_json(r#"{"b":1,"a":[1,2],"c":{"x":null}}"#).unwrap();
        assert_eq!(
            out,
            "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ],\n  \"c\": {\n    \"x\": null\n  }\n}\n"
        );
    }

    #[test]
    fn json_numbers_and_duplicate_keys_survive() {
        let out = format_json(r#"{"a":12345678901234567890123,"b":1.50,"c":1e3,"a":2}"#).unwrap();
        assert_eq!(
            out,
            "{\n  \"a\": 12345678901234567890123,\n  \"b\": 1.50,\n  \"c\": 1e3,\n  \"a\": 2\n}\n"
        );
    }

    #[test]
    fn json_strings_with_braces_and_escapes_are_verbatim() {
        let src = r#"{"k":"{[,:]} \" \\","e":{},"l":[ ]}"#;
        let out = format_json(src).unwrap();
        assert_eq!(
            out,
            "{\n  \"k\": \"{[,:]} \\\" \\\\\",\n  \"e\": {},\n  \"l\": []\n}\n"
        );
    }

    #[test]
    fn xml_keeps_entities_comments_and_cdata_text() {
        assert_eq!(
            format_xml("<a>x &amp; y</a>").unwrap(),
            "<a>x &amp; y</a>\n"
        );
        assert_eq!(
            format_xml("<a>x <!--c--> y</a>").unwrap(),
            "<a>x <!--c--> y</a>\n"
        );
        assert_eq!(
            format_xml("<a>x <![CDATA[<b>]]> y</a>").unwrap(),
            "<a>x <![CDATA[<b>]]> y</a>\n"
        );
    }

    #[test]
    fn json_error_is_reported() {
        assert!(format_json("{\"a\":").is_err());
    }

    #[test]
    fn yaml_reindents_and_keeps_every_document() {
        let out = format_yaml("a:    1\nb:\n      - x\n      - y\n---\nc: 2\n").unwrap();
        assert_eq!(out, "a: 1\nb:\n- x\n- y\n---\nc: 2\n");
    }

    #[test]
    fn yaml_error_is_reported() {
        assert!(format_yaml("a: [1, 2\n").is_err());
    }

    #[test]
    fn xml_indents_two() {
        let out = format_xml("<a><b>t</b><c/></a>").unwrap();
        assert_eq!(out, "<a>\n  <b>t</b>\n  <c/>\n</a>\n");
    }

    #[test]
    fn xml_error_is_reported() {
        assert!(format_xml("<a><b></a>").is_err());
        assert!(format_xml("<a>").is_err());
    }

    #[test]
    fn only_data_languages_format() {
        assert!(can_format(FileLanguage::Json));
        assert!(can_format(FileLanguage::Yaml));
        assert!(can_format(FileLanguage::Xml));
        assert!(!can_format(FileLanguage::JsonLines));
        assert!(!can_format(FileLanguage::Rust));
        assert!(format_text(FileLanguage::Rust, "").is_none());
    }
}
