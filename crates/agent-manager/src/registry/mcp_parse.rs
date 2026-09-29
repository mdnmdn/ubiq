//! Parse a pasted MCP server configuration, whichever harness's dialect it is in.
//!
//! Accepted: `{"mcpServers": {...}}` (Claude, Cursor, Copilot), `{"servers": {...}}` (VS Code),
//! `{"mcp": {...}}` (opencode: `type: local|remote`, `command: [cmd, args…]`, `environment`),
//! `{"context_servers": {...}}` (Zed: `command` plain or `{path, args, env}`), a bare
//! `{id: server}` map, a single server object (id ""), and Codex's TOML `[mcp_servers.<id>]`.
//! JSON may carry `//` and `/* */` comments and trailing commas.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail};
use serde_json::{Map, Value};

use crate::Result;
use crate::config::{McpServer, McpTransport};

/// Keys that wrap a map of servers, in the order they are tried.
const WRAPPERS: &[&str] = &[
    "mcpServers",
    "servers",
    "mcp",
    "context_servers",
    "mcp_servers",
];

/// Parse `text` into servers; ids come from the map keys (empty for a single unnamed server).
pub fn parse_mcp_config(text: &str) -> Result<Vec<McpServer>> {
    let text = text.trim();
    if text.is_empty() {
        bail!("nothing to parse");
    }
    let root: Value = if text.starts_with('{') {
        serde_json::from_str(&clean_json(text)).map_err(|e| anyhow!("invalid JSON: {e}"))?
    } else {
        let table: toml::Table =
            toml::from_str(text).map_err(|e| anyhow!("not JSON, and invalid TOML: {e}"))?;
        serde_json::to_value(table)?
    };
    let Value::Object(obj) = root else {
        bail!("expected an object of servers");
    };

    for key in WRAPPERS {
        if let Some(Value::Object(map)) = obj.get(*key) {
            return from_map(map);
        }
    }
    if looks_like_server(&Value::Object(obj.clone())) {
        return Ok(vec![from_value("", &Value::Object(obj))?]);
    }
    if !obj.is_empty() && obj.values().all(looks_like_server) {
        return from_map(&obj);
    }
    bail!(
        "no MCP servers found (expected mcpServers, servers, mcp, context_servers, mcp_servers, or a server object)"
    )
}

fn from_map(map: &Map<String, Value>) -> Result<Vec<McpServer>> {
    let servers = map
        .iter()
        .map(|(id, v)| from_value(id, v))
        .collect::<Result<Vec<_>>>()?;
    if servers.is_empty() {
        bail!("the server list is empty");
    }
    Ok(servers)
}

fn looks_like_server(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        ["command", "url", "serverUrl", "httpUrl"]
            .iter()
            .any(|k| o.contains_key(*k))
    })
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(scalar).collect())
        .unwrap_or_default()
}

/// A JSON scalar as text (numbers and booleans included, as env values often are).
fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn string_map(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| scalar(v).map(|s| (k.clone(), s)))
                .collect()
        })
        .unwrap_or_default()
}

fn from_value(id: &str, v: &Value) -> Result<McpServer> {
    let who = if id.is_empty() {
        "server".to_string()
    } else {
        format!("server '{id}'")
    };
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow!("{who}: expected an object"))?;

    let mut command = None;
    let mut args = strings(obj.get("args").unwrap_or(&Value::Null));
    let mut env = string_map(obj.get("env").or_else(|| obj.get("environment")));
    match obj.get("command") {
        Some(Value::String(s)) => command = Some(s.clone()),
        // opencode: the whole argv in one array.
        Some(Value::Array(a)) => {
            let mut argv = a.iter().filter_map(scalar);
            command = argv.next();
            let mut rest: Vec<String> = argv.collect();
            rest.append(&mut args);
            args = rest;
        }
        // Zed: { path, args, env }.
        Some(Value::Object(c)) => {
            command = c.get("path").and_then(Value::as_str).map(str::to_string);
            args = strings(c.get("args").unwrap_or(&Value::Null));
            for (k, val) in string_map(c.get("env")) {
                env.entry(k).or_insert(val);
            }
        }
        _ => {}
    }
    let url = ["url", "serverUrl", "httpUrl"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .map(str::to_string);
    let headers = string_map(obj.get("headers").or_else(|| obj.get("http_headers")));

    let kind = obj
        .get("type")
        .or_else(|| obj.get("transport"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let transport = match kind.as_deref() {
        Some("stdio" | "local") => McpTransport::Stdio,
        Some(
            "http" | "streamable-http" | "streamableHttp" | "streamable_http" | "streamable"
            | "http_stream" | "remote",
        ) => McpTransport::Http,
        Some("sse") => McpTransport::Sse,
        Some(other) => bail!("{who}: unknown type '{other}'"),
        None if command.is_some() => McpTransport::Stdio,
        // Gemini CLI: `httpUrl` is streamable HTTP, a bare `url` is SSE.
        None if obj.get("httpUrl").is_some_and(Value::is_string) => McpTransport::Http,
        None if url.as_deref().is_some_and(ends_in_sse) => McpTransport::Sse,
        None => McpTransport::Http,
    };
    match transport {
        McpTransport::Stdio if command.is_none() => bail!("{who}: a stdio server needs a command"),
        McpTransport::Http | McpTransport::Sse if url.is_none() => {
            bail!("{who}: a remote server needs a url")
        }
        _ => {}
    }
    Ok(McpServer {
        id: id.to_string(),
        transport,
        command,
        args,
        env,
        url,
        headers,
    })
}

/// Whether the path of `url` (query and fragment dropped) ends in `/sse`.
fn ends_in_sse(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.trim_end_matches('/')
        .to_ascii_lowercase()
        .ends_with("/sse")
}

/// Strip `//` and `/* */` comments outside strings, then commas that precede `}` or `]`.
fn clean_json(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_str) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else {
            out.push(c);
        }
        i += 1;
    }

    // Second pass: drop a comma whose next non-space character closes a container.
    let chars: Vec<char> = out.chars().collect();
    let mut clean = String::with_capacity(out.len());
    let (mut in_str, mut i) = (false, 0);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            if c == '\\' && i + 1 < chars.len() {
                clean.push(c);
                i += 1;
                clean.push(chars[i]);
            } else {
                clean.push(c);
                in_str = c != '"';
            }
        } else if c == ',' {
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}' | ']')) {
                clean.push(c);
            }
        } else {
            in_str = c == '"';
            clean.push(c);
        }
        i += 1;
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> McpServer {
        let mut v = parse_mcp_config(text).unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        v.remove(0)
    }

    #[test]
    fn untyped_remote_servers_follow_the_gemini_shape() {
        let s = one(r#"{"mcpServers":{"g":{"httpUrl":"https://h/api"}}}"#);
        assert_eq!(s.transport, McpTransport::Http);
        assert_eq!(s.url.as_deref(), Some("https://h/api"));
        let s = one(r#"{"mcpServers":{"g":{"url":"https://h/sse?k=1"}}}"#);
        assert_eq!(s.transport, McpTransport::Sse);
        let s = one(r#"{"mcpServers":{"g":{"url":"https://h/mcp"}}}"#);
        assert_eq!(s.transport, McpTransport::Http);
        for t in ["streamable_http", "streamable", "http_stream"] {
            let s = one(&format!(
                r#"{{"mcpServers":{{"g":{{"type":"{t}","url":"https://h/sse"}}}}}}"#
            ));
            assert_eq!(s.transport, McpTransport::Http, "{t}");
        }
    }

    #[test]
    fn claude_cursor_mcp_servers() {
        let s = one(
            r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","@x/fs","/tmp"],"env":{"K":"v","N":1}}}}"#,
        );
        assert_eq!(s.id, "fs");
        assert_eq!(s.transport, McpTransport::Stdio);
        assert_eq!(s.command.as_deref(), Some("npx"));
        assert_eq!(s.args, ["-y", "@x/fs", "/tmp"]);
        assert_eq!(s.env["N"], "1");

        let s = one(
            r#"{"mcpServers":{"r":{"type":"http","url":"https://h/mcp","headers":{"Authorization":"Bearer x"}}}}"#,
        );
        assert_eq!(s.transport, McpTransport::Http);
        assert_eq!(s.headers["Authorization"], "Bearer x");
    }

    #[test]
    fn vscode_servers_and_type_aliases() {
        let all = parse_mcp_config(
            r#"{"servers":{"a":{"type":"sse","url":"https://a/sse"},
                "b":{"type":"streamable-http","url":"https://b"},
                "c":{"type":"stdio","command":"c"}}}"#,
        )
        .unwrap();
        let t: Vec<_> = all.iter().map(|s| (s.id.as_str(), s.transport)).collect();
        assert_eq!(
            t,
            [
                ("a", McpTransport::Sse),
                ("b", McpTransport::Http),
                ("c", McpTransport::Stdio)
            ]
        );
    }

    #[test]
    fn opencode_local_and_remote() {
        let all = parse_mcp_config(
            r#"{"mcp":{"l":{"type":"local","command":["npx","-y","pkg"],"environment":{"A":"b"}},
                "r":{"type":"remote","url":"https://r/mcp","headers":{"X":"y"}}}}"#,
        )
        .unwrap();
        assert_eq!(all[0].command.as_deref(), Some("npx"));
        assert_eq!(all[0].args, ["-y", "pkg"]);
        assert_eq!(all[0].env["A"], "b");
        assert_eq!(all[1].transport, McpTransport::Http);
    }

    #[test]
    fn zed_context_servers() {
        let all = parse_mcp_config(
            r#"{"context_servers":{"z":{"command":{"path":"node","args":["s.js"],"env":{"E":"1"}},"settings":{}},
                "p":{"command":"plain","args":["a"]}}}"#,
        )
        .unwrap();
        let z = all.iter().find(|s| s.id == "z").unwrap();
        assert_eq!(
            (z.command.as_deref(), z.args.as_slice()),
            (Some("node"), &["s.js".to_string()][..])
        );
        assert_eq!(z.env["E"], "1");
        assert_eq!(
            all.iter().find(|s| s.id == "p").unwrap().command.as_deref(),
            Some("plain")
        );
    }

    #[test]
    fn bare_map_and_single_server() {
        let all = parse_mcp_config(r#"{"a":{"command":"x"},"b":{"url":"https://b"}}"#).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].transport, McpTransport::Http); // no type, no command
        let s = one(r#"{"command":"solo","args":["1"]}"#);
        assert_eq!(s.id, "");
        assert_eq!(s.command.as_deref(), Some("solo"));
    }

    #[test]
    fn codex_toml() {
        let all = parse_mcp_config(
            r#"
[mcp_servers.docs]
command = "npx"
args = ["-y", "docs-mcp"]
env = { TOKEN = "t" }

[mcp_servers.web]
url = "https://w/mcp"
http_headers = { "X-Key" = "k" }
"#,
        )
        .unwrap();
        assert_eq!(all.len(), 2);
        let docs = all.iter().find(|s| s.id == "docs").unwrap();
        assert_eq!(docs.args, ["-y", "docs-mcp"]);
        assert_eq!(docs.env["TOKEN"], "t");
        let web = all.iter().find(|s| s.id == "web").unwrap();
        assert_eq!(web.transport, McpTransport::Http);
        assert_eq!(web.headers["X-Key"], "k");
    }

    #[test]
    fn comments_and_trailing_commas() {
        let s = one(r#"{
  // the server
  "mcpServers": {
    "a": { "command": "x", /* inline */ "args": ["http://not-a-comment", "a,]",], },
  },
}"#);
        assert_eq!(s.args, ["http://not-a-comment", "a,]"]);
    }

    #[test]
    fn errors_are_specific() {
        assert!(parse_mcp_config("").is_err());
        assert!(
            parse_mcp_config("{ nope")
                .unwrap_err()
                .to_string()
                .contains("JSON")
        );
        assert!(parse_mcp_config(r#"{"mcpServers":{}}"#).is_err());
        assert!(parse_mcp_config(r#"{"foo":1}"#).is_err());
        let e = parse_mcp_config(r#"{"mcpServers":{"a":{"type":"stdio"}}}"#).unwrap_err();
        assert!(e.to_string().contains("'a'") && e.to_string().contains("command"));
        assert!(
            parse_mcp_config(r#"{"mcpServers":{"a":{"type":"weird","command":"x"}}}"#).is_err()
        );
        assert!(parse_mcp_config(r#"{"mcpServers":{"a":{"type":"http"}}}"#).is_err());
    }
}
