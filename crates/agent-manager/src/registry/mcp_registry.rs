//! Client of the official MCP registry (`registry.modelcontextprotocol.io`): search servers and
//! turn each into ready-to-edit [`McpDraft`]s (one per package or remote it offers).
//!
//! Needs the `remote` feature. The JSON-to-types mapping is [`parse_registry_page`], a pure
//! function, so it is tested without a network.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::anyhow;
use serde::Deserialize;

use crate::Result;
use crate::config::{McpServer, McpTransport};

/// The public registry.
pub const DEFAULT_BASE: &str = "https://registry.modelcontextprotocol.io";

/// One server listed in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryServer {
    /// Reverse-DNS name, e.g. `io.github.org/name`.
    pub name: String,
    /// Display title, when it has one.
    pub title: Option<String>,
    /// What it does.
    pub description: Option<String>,
    /// Latest version.
    pub version: Option<String>,
    /// Source repository URL.
    pub repository: Option<String>,
    /// Ways to run it: one per package and per remote endpoint.
    pub options: Vec<McpDraft>,
}

/// One way to run a registry server, as a catalog server definition to review and save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDraft {
    /// What it is, e.g. `npx remote-filesystem-mcp-server` or `remote: streamable-http`.
    pub label: String,
    /// The server definition (id derived from the registry name).
    pub server: McpServer,
    /// Values the person has to fill in or check.
    pub params: Vec<ParamHint>,
}

/// Where a [`ParamHint`] lands in the server definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// An environment variable.
    Env,
    /// An HTTP header.
    Header,
    /// A command-line argument.
    Arg,
}

/// A value the registry says a server takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamHint {
    /// Variable, header or argument name.
    pub name: String,
    /// Where it goes.
    pub kind: ParamKind,
    /// What it is for.
    pub description: Option<String>,
    /// Whether the server does not work without it.
    pub required: bool,
    /// Whether it is a credential.
    pub secret: bool,
    /// The registry's default value.
    pub default: Option<String>,
}

/// Client of an MCP registry; `base` is the URL without a trailing slash.
#[derive(Debug, Clone)]
pub struct McpRegistryClient {
    base: String,
}

impl Default for McpRegistryClient {
    fn default() -> Self {
        Self::new(DEFAULT_BASE)
    }
}

impl McpRegistryClient {
    /// A client of the registry at `base`.
    pub fn new(base: impl Into<String>) -> Self {
        McpRegistryClient {
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    /// Search the latest version of each server; `cursor` continues a previous page. Returns the
    /// servers and the cursor of the next page, when there is one.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<(Vec<RegistryServer>, Option<String>)> {
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(true)
            .timeout(Duration::from_secs(20))
            .build();
        let mut req = agent
            .get(&format!("{}/v0.1/servers", self.base))
            .query("search", query)
            .query("limit", &limit.to_string())
            .query("version", "latest");
        if let Some(cursor) = cursor {
            req = req.query("cursor", cursor);
        }
        let body = req
            .call()
            .map_err(|e| anyhow!("MCP registry: {e}"))?
            .into_string()?;
        parse_registry_page(&body)
    }
}

// ---- wire shapes (lenient: everything optional) ----

#[derive(Deserialize, Default)]
struct Page {
    #[serde(default)]
    servers: Vec<Item>,
    #[serde(default)]
    metadata: Meta,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Meta {
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    server: Raw,
}

#[derive(Deserialize)]
struct Raw {
    name: String,
    title: Option<String>,
    description: Option<String>,
    version: Option<String>,
    repository: Option<Repo>,
    #[serde(default)]
    packages: Vec<Package>,
    #[serde(default)]
    remotes: Vec<Remote>,
}

#[derive(Deserialize)]
struct Repo {
    url: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Package {
    registry_type: String,
    identifier: String,
    version: Option<String>,
    runtime_hint: Option<String>,
    #[serde(default)]
    runtime_arguments: Vec<Field>,
    #[serde(default)]
    package_arguments: Vec<Field>,
    #[serde(default)]
    environment_variables: Vec<Field>,
    transport: Option<Transport>,
}

#[derive(Deserialize)]
struct Transport {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
}

#[derive(Deserialize)]
struct Remote {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(default)]
    headers: Vec<Field>,
}

/// An argument, environment variable or header (the registry uses one shape for all three).
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Field {
    #[serde(rename = "type")]
    kind: Option<String>,
    name: Option<String>,
    value: Option<String>,
    default: Option<String>,
    description: Option<String>,
    #[serde(default)]
    is_required: bool,
    #[serde(default)]
    is_secret: bool,
    value_hint: Option<String>,
}

impl Field {
    fn hint(&self, name: &str, kind: ParamKind) -> ParamHint {
        ParamHint {
            name: name.to_string(),
            kind,
            description: self.description.clone(),
            required: self.is_required,
            secret: self.is_secret,
            default: self.default.clone(),
        }
    }
}

/// Map one page of `GET /v0.1/servers` to servers and the next cursor.
pub fn parse_registry_page(json: &str) -> Result<(Vec<RegistryServer>, Option<String>)> {
    let page: Page =
        serde_json::from_str(json).map_err(|e| anyhow!("unexpected registry response: {e}"))?;
    let servers = page
        .servers
        .into_iter()
        .map(|i| map_server(i.server))
        .collect();
    Ok((servers, page.metadata.next_cursor.filter(|c| !c.is_empty())))
}

fn map_server(raw: Raw) -> RegistryServer {
    let id = draft_id(&raw.name);
    let mut options: Vec<McpDraft> = raw
        .packages
        .iter()
        .filter_map(|p| map_package(&id, p))
        .collect();
    options.extend(raw.remotes.iter().map(|r| map_remote(&id, r)));
    RegistryServer {
        name: raw.name,
        title: raw.title,
        description: raw.description,
        version: raw.version,
        repository: raw.repository.and_then(|r| r.url),
        options,
    }
}

/// `io.github.org/some-server` -> `some-server`, with anything outside `[A-Za-z0-9._-]` as `-`.
fn draft_id(name: &str) -> String {
    let last = name.rsplit('/').next().unwrap_or(name);
    let id: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    id.trim_start_matches('.').to_string()
}

fn transport_of(kind: &str) -> McpTransport {
    if kind.eq_ignore_ascii_case("sse") {
        McpTransport::Sse
    } else {
        McpTransport::Http
    }
}

/// Headers as a literal value (or default, else empty) plus a hint each.
fn headers_of(fields: &[Field]) -> (BTreeMap<String, String>, Vec<ParamHint>) {
    let mut headers = BTreeMap::new();
    let mut params = Vec::new();
    for f in fields {
        let Some(name) = &f.name else { continue };
        headers.insert(
            name.clone(),
            f.value
                .clone()
                .or_else(|| f.default.clone())
                .unwrap_or_default(),
        );
        params.push(f.hint(name, ParamKind::Header));
    }
    (headers, params)
}

fn map_remote(id: &str, r: &Remote) -> McpDraft {
    let (headers, params) = headers_of(&r.headers);
    McpDraft {
        label: format!("remote: {}", r.kind),
        server: McpServer {
            id: id.to_string(),
            transport: transport_of(&r.kind),
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some(r.url.clone()),
            headers,
        },
        params,
    }
}

/// Command-line arguments of a package: literal values inline, values left to the person as hints.
fn args_of(fields: &[Field], out: &mut Vec<String>, params: &mut Vec<ParamHint>) {
    for f in fields {
        let value = f.value.clone().or_else(|| f.default.clone());
        let needs_input = f.value.as_deref().is_none_or(|v| v.contains('{'));
        match f.kind.as_deref() {
            Some("named") => {
                let Some(name) = &f.name else { continue };
                out.push(name.clone());
                if let Some(v) = value.filter(|v| !v.is_empty()) {
                    out.push(v);
                }
                if needs_input {
                    params.push(f.hint(name, ParamKind::Arg));
                }
            }
            _ => {
                let label = f
                    .value_hint
                    .as_deref()
                    .or(f.name.as_deref())
                    .unwrap_or("argument");
                if let Some(v) = value.filter(|v| !v.is_empty()) {
                    out.push(v);
                }
                if needs_input {
                    params.push(f.hint(label, ParamKind::Arg));
                }
            }
        }
    }
}

fn map_package(id: &str, p: &Package) -> Option<McpDraft> {
    let ver = p
        .version
        .as_deref()
        .filter(|v| !v.is_empty() && *v != "latest");
    let (default_cmd, mut args, spec) = match p.registry_type.as_str() {
        "npm" => ("npx", vec![], with_version(&p.identifier, "@", ver)),
        "pypi" => ("uvx", vec![], with_version(&p.identifier, "==", ver)),
        "oci" => (
            "docker",
            vec!["run".into(), "-i".into(), "--rm".into()],
            with_version(&p.identifier, ":", ver),
        ),
        "nuget" => ("dnx", vec![], with_version(&p.identifier, "@", ver)),
        _ => return None,
    };
    let command = p
        .runtime_hint
        .clone()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| default_cmd.to_string());
    let mut params = Vec::new();
    args_of(&p.runtime_arguments, &mut args, &mut params);
    if p.registry_type == "npm" && !args.iter().any(|a| a == "-y" || a == "--yes") {
        args.insert(0, "-y".into());
    }
    args.push(spec);
    if p.registry_type == "nuget" {
        args.push("--yes".into());
    }
    args_of(&p.package_arguments, &mut args, &mut params);

    let transport = p.transport.as_ref();
    let remote_kind = transport.map(|t| t.kind.as_str()).filter(|k| *k != "stdio");
    if let Some(kind) = remote_kind {
        // The package runs elsewhere and is reached over HTTP/SSE at the transport's url.
        let url = transport.and_then(|t| t.url.clone())?;
        return Some(McpDraft {
            label: format!("remote: {kind} ({})", p.identifier),
            server: McpServer {
                id: id.to_string(),
                transport: transport_of(kind),
                command: None,
                args: vec![],
                env: BTreeMap::new(),
                url: Some(url),
                headers: BTreeMap::new(),
            },
            params: vec![],
        });
    }

    let mut env = BTreeMap::new();
    for f in &p.environment_variables {
        let Some(name) = &f.name else { continue };
        env.insert(
            name.clone(),
            f.value
                .clone()
                .or_else(|| f.default.clone())
                .unwrap_or_default(),
        );
        params.push(f.hint(name, ParamKind::Env));
    }
    Some(McpDraft {
        label: format!("{command} {}", p.identifier),
        server: McpServer {
            id: id.to_string(),
            transport: McpTransport::Stdio,
            command: Some(command),
            args,
            env,
            url: None,
            headers: BTreeMap::new(),
        },
        params,
    })
}

fn with_version(identifier: &str, sep: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{identifier}{sep}{v}"),
        None => identifier.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"{
  "servers": [
    {"server": {"name":"com.pulsemcp/remote-filesystem","description":"Remote FS.",
      "repository":{"url":"https://github.com/pulsemcp/mcp-servers","source":"github"},"version":"0.1.5",
      "packages":[{"registryType":"npm","identifier":"remote-filesystem-mcp-server","version":"0.1.5",
        "runtimeHint":"npx","transport":{"type":"stdio"},
        "runtimeArguments":[{"value":"-y","type":"positional"}],
        "packageArguments":[{"type":"named","name":"--root","description":"Root dir","isRequired":true},
                            {"type":"named","name":"--mode","value":"ro"}],
        "environmentVariables":[{"name":"GCS_BUCKET","description":"Bucket","isRequired":true},
                                {"name":"KEY","isSecret":true},
                                {"name":"PUBLIC","default":"false"}]}]},
     "_meta":{}},
    {"server": {"name":"io.github.x/py-thing","title":"Py Thing","version":"2.0.0",
      "packages":[{"registryType":"pypi","identifier":"py-thing","version":"2.0.0","transport":{"type":"stdio"}},
                  {"registryType":"oci","identifier":"ghcr.io/x/thing","version":"1.0","transport":{"type":"stdio"},
                   "runtimeArguments":[{"type":"named","name":"-e","value":"A=b"}]},
                  {"registryType":"nuget","identifier":"X.Thing","version":"3.1.0","transport":{"type":"stdio"}},
                  {"registryType":"cargo","identifier":"unsupported","transport":{"type":"stdio"}},
                  {"registryType":"npm","identifier":"hosted","transport":{"type":"streamable-http","url":"https://h/mcp"}}],
      "remotes":[{"type":"sse","url":"https://r/sse","headers":[{"name":"Payment-Signature","isSecret":true},
                                                              {"name":"X-Api","value":"Bearer {token}","isRequired":true}]},
                 {"type":"streamable-http","url":"https://r/mcp"}]}}
  ],
  "metadata": {"nextCursor":"io.github.x/py-thing:2.0.0","count":2}
}"#;

    #[test]
    fn maps_a_registry_page() {
        let (servers, next) = parse_registry_page(PAGE).unwrap();
        assert_eq!(next.as_deref(), Some("io.github.x/py-thing:2.0.0"));
        assert_eq!(servers.len(), 2);

        let fs = &servers[0];
        assert_eq!(
            fs.repository.as_deref(),
            Some("https://github.com/pulsemcp/mcp-servers")
        );
        let npm = &fs.options[0];
        assert_eq!(npm.label, "npx remote-filesystem-mcp-server");
        assert_eq!(npm.server.id, "remote-filesystem");
        assert_eq!(npm.server.command.as_deref(), Some("npx"));
        assert_eq!(
            npm.server.args,
            [
                "-y",
                "remote-filesystem-mcp-server@0.1.5",
                "--root",
                "--mode",
                "ro"
            ]
        );
        assert_eq!(npm.server.env.len(), 3);
        assert_eq!(npm.server.env["PUBLIC"], "false");
        let hint = |n: &str| npm.params.iter().find(|p| p.name == n).unwrap();
        assert!(hint("GCS_BUCKET").required && hint("GCS_BUCKET").kind == ParamKind::Env);
        assert!(hint("KEY").secret);
        assert_eq!(hint("PUBLIC").default.as_deref(), Some("false"));
        assert!(hint("--root").required && hint("--root").kind == ParamKind::Arg);
        assert!(npm.params.iter().all(|p| p.name != "--mode"));

        let py = &servers[1];
        assert_eq!(py.title.as_deref(), Some("Py Thing"));
        let labels: Vec<_> = py.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "uvx py-thing",
                "docker ghcr.io/x/thing",
                "dnx X.Thing",
                "remote: streamable-http (hosted)",
                "remote: sse",
                "remote: streamable-http"
            ]
        );
        assert_eq!(py.options[0].server.args, ["py-thing==2.0.0"]);
        assert_eq!(
            py.options[1].server.args,
            ["run", "-i", "--rm", "-e", "A=b", "ghcr.io/x/thing:1.0"]
        );
        assert_eq!(py.options[2].server.args, ["X.Thing@3.1.0", "--yes"]);
        let hosted = &py.options[3].server;
        assert_eq!(
            (hosted.transport, hosted.url.as_deref()),
            (McpTransport::Http, Some("https://h/mcp"))
        );
        let sse = &py.options[4];
        assert_eq!(sse.server.transport, McpTransport::Sse);
        assert_eq!(sse.server.headers["X-Api"], "Bearer {token}");
        assert_eq!(sse.server.headers["Payment-Signature"], "");
        assert!(
            sse.params
                .iter()
                .any(|p| p.name == "Payment-Signature" && p.secret && p.kind == ParamKind::Header)
        );
    }

    #[test]
    fn empty_and_broken_pages() {
        let (s, next) = parse_registry_page(r#"{"servers":[],"metadata":{"count":0}}"#).unwrap();
        assert!(s.is_empty() && next.is_none());
        assert!(parse_registry_page("<html>").is_err());
    }

    #[test]
    fn client_trims_the_base() {
        assert_eq!(McpRegistryClient::new("https://x/").base, "https://x");
        assert_eq!(McpRegistryClient::default().base, DEFAULT_BASE);
    }
}
