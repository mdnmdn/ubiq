//! `am mcp` subcommands: manage the MCP servers of a catalog layer, import pasted configs and
//! search the official MCP registry.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};

use super::skill::LayerArgs;
use crate::config::{McpServer, McpTransport};
use crate::registry::{
    CatalogStore, McpEntry, McpExpose, McpRegistryClient, ParamKind, Registry, parse_mcp_config,
};

/// `am mcp` argument parser.
#[derive(Debug, Parser)]
#[command(name = "am-mcp", disable_help_flag = false)]
struct McpArgs {
    #[command(flatten)]
    layer: LayerArgs,
    #[command(subcommand)]
    command: McpCommand,
}

/// Subcommands for `am mcp`.
#[derive(Debug, Subcommand)]
enum McpCommand {
    /// List the MCP servers of the layer.
    #[command(name = "ls")]
    List,
    /// Add a server: `add <id> -- <command> [args..]` (local) or `add <id> --url <url>` (remote).
    Add {
        /// The server id.
        id: String,
        /// Remote server URL.
        #[arg(long, conflicts_with = "command")]
        url: Option<String>,
        /// The remote server speaks SSE instead of streamable HTTP.
        #[arg(long, requires = "url")]
        sse: bool,
        /// Environment variable for a local server, `KEY=VALUE` (repeatable).
        #[arg(long = "env")]
        env: Vec<String>,
        /// HTTP header for a remote server, `KEY=VALUE` or `KEY: VALUE` (repeatable).
        #[arg(long = "header")]
        headers: Vec<String>,
        /// One line about what the server is for.
        #[arg(long)]
        description: Option<String>,
        /// The command and its arguments, after `--`.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Import servers from a pasted config (any harness's format; `-` reads stdin).
    Import {
        /// File to read, or `-`.
        file: String,
        /// Id for a config holding one unnamed server.
        #[arg(long)]
        id: Option<String>,
        /// Prefix for every imported id.
        #[arg(long)]
        id_prefix: Option<String>,
    },
    /// Remove a server.
    Rm {
        /// The server id.
        id: String,
    },
    /// Search the official MCP registry.
    Search {
        /// Text to search for.
        query: String,
        /// Maximum number of servers.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

/// Run an mcp subcommand, given argv AFTER the `mcp` word.
pub(super) fn run(args: &[String]) -> Result<()> {
    let args = if args.is_empty() {
        vec!["ls".to_string()]
    } else {
        args.to_vec()
    };
    let args = McpArgs::try_parse_from(std::iter::once("am-mcp".to_string()).chain(args))?;
    let store = args.layer.store()?;
    match args.command {
        McpCommand::List => cmd_list(&store),
        McpCommand::Add {
            id,
            url,
            sse,
            env,
            headers,
            description,
            command,
        } => {
            let server = build_server(&id, url, sse, &env, &headers, command)?;
            put(&store, server, description)?;
            println!("added MCP '{id}'");
            Ok(())
        }
        McpCommand::Import {
            file,
            id,
            id_prefix,
        } => cmd_import(&store, &file, id, id_prefix),
        McpCommand::Rm { id } => {
            store.remove_mcp(&id)?;
            println!("removed MCP '{id}'");
            Ok(())
        }
        McpCommand::Search { query, limit } => cmd_search(&query, limit),
    }
}

/// Split `KEY=VALUE` (or `KEY: VALUE` when `colon`) at its first separator.
fn split_kv(s: &str, colon: bool) -> Result<(String, String)> {
    let at = s
        .find(|c| c == '=' || (colon && c == ':'))
        .ok_or_else(|| anyhow!("expected KEY=VALUE, got '{s}'"))?;
    Ok((s[..at].trim().to_string(), s[at + 1..].trim().to_string()))
}

fn kv_map(items: &[String], colon: bool) -> Result<BTreeMap<String, String>> {
    items.iter().map(|s| split_kv(s, colon)).collect()
}

fn build_server(
    id: &str,
    url: Option<String>,
    sse: bool,
    env: &[String],
    headers: &[String],
    mut command: Vec<String>,
) -> Result<McpServer> {
    let mut server = McpServer {
        id: id.to_string(),
        transport: McpTransport::Stdio,
        command: None,
        args: vec![],
        env: kv_map(env, false)?,
        url: None,
        headers: kv_map(headers, true)?,
    };
    match url {
        Some(url) => {
            server.transport = if sse {
                McpTransport::Sse
            } else {
                McpTransport::Http
            };
            server.url = Some(url);
        }
        None if command.is_empty() => {
            bail!("give a command after `--`, or --url for a remote server")
        }
        None => {
            server.command = Some(command.remove(0));
            server.args = command;
        }
    }
    Ok(server)
}

fn put(store: &impl CatalogStore, server: McpServer, description: Option<String>) -> Result<()> {
    store.put_mcp(&McpEntry {
        id: server.id.clone(),
        def: server,
        expose: McpExpose::Tools,
        summary: None,
        description,
    })
}

fn endpoint(s: &McpServer) -> String {
    match (&s.command, &s.url) {
        (Some(cmd), _) => std::iter::once(cmd.as_str())
            .chain(s.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        (_, Some(url)) => url.clone(),
        _ => "(no command or url)".to_string(),
    }
}

fn cmd_list(store: &impl Registry) -> Result<()> {
    let mcps = store.mcps()?;
    if mcps.is_empty() {
        println!("(no MCP servers)");
    }
    for m in mcps {
        let t = format!("{:?}", m.def.transport).to_lowercase();
        println!("  {}  [{t}] {}", m.id, endpoint(&m.def));
    }
    Ok(())
}

fn cmd_import(
    store: &impl CatalogStore,
    file: &str,
    id: Option<String>,
    prefix: Option<String>,
) -> Result<()> {
    let text = if file == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        std::fs::read_to_string(PathBuf::from(file))?
    };
    let servers = parse_mcp_config(&text)?;
    let prefix = prefix.unwrap_or_default();
    for mut s in servers {
        if s.id.is_empty() {
            s.id = id
                .clone()
                .ok_or_else(|| anyhow!("the config holds one unnamed server: give it an --id"))?;
        }
        s.id = format!("{prefix}{}", s.id);
        let id = s.id.clone();
        put(store, s, None)?;
        println!("imported MCP '{id}'");
    }
    Ok(())
}

fn cmd_search(query: &str, limit: usize) -> Result<()> {
    let (servers, next) = McpRegistryClient::default().search(query, limit, None)?;
    if servers.is_empty() {
        println!("(no matches)");
    }
    for s in servers {
        println!(
            "{}  {}",
            s.name,
            s.version
                .as_deref()
                .map(|v| format!("v{v}"))
                .unwrap_or_default()
        );
        if let Some(d) = &s.description {
            println!("    {}", super::skill::short(d));
        }
        for (i, o) in s.options.iter().enumerate() {
            println!("    [{}] {}: {}", i + 1, o.label, endpoint(&o.server));
            for p in &o.params {
                let kind = match p.kind {
                    ParamKind::Env => "env",
                    ParamKind::Header => "header",
                    ParamKind::Arg => "arg",
                };
                let flags = format!(
                    "{}{}",
                    if p.required { " required" } else { "" },
                    if p.secret { " secret" } else { "" }
                );
                println!("          {kind} {}{flags}", p.name);
            }
        }
    }
    if next.is_some() {
        println!("(more results: raise --limit)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn add_local_and_remote() {
        let cmd = McpArgs::try_parse_from([
            "am-mcp",
            "--catalog",
            "/x",
            "add",
            "fs",
            "--env",
            "A=b=c",
            "--",
            "npx",
            "-y",
            "pkg",
        ])
        .unwrap();
        let McpCommand::Add {
            id,
            env,
            command,
            url,
            ..
        } = cmd.command
        else {
            panic!()
        };
        let s = build_server(&id, url, false, &env, &[], command).unwrap();
        assert_eq!(s.command.as_deref(), Some("npx"));
        assert_eq!(s.args, ["-y", "pkg"]);
        assert_eq!(s.env["A"], "b=c");

        let s = build_server(
            "r",
            Some("https://h".into()),
            true,
            &[],
            &args(&["X-Key: v", "Auth=Bearer t"]),
            vec![],
        )
        .unwrap();
        assert_eq!(s.transport, McpTransport::Sse);
        assert_eq!(s.headers["X-Key"], "v");
        assert_eq!(s.headers["Auth"], "Bearer t");
        assert!(build_server("e", None, false, &[], &[], vec![]).is_err());
    }

    #[test]
    fn import_and_rm_against_a_temp_catalog() {
        let dir = tempfile::TempDir::new().unwrap();
        let cat = dir.path().join("cat");
        let file = dir.path().join("in.json");
        std::fs::write(&file, r#"{"mcpServers":{"a":{"command":"x"}}}"#).unwrap();
        let cat_s = cat.to_str().unwrap();
        run(&args(&[
            "--catalog",
            cat_s,
            "import",
            file.to_str().unwrap(),
            "--id-prefix",
            "p-",
        ]))
        .unwrap();
        run(&args(&[
            "--catalog",
            cat_s,
            "add",
            "b",
            "--url",
            "https://b",
        ]))
        .unwrap();
        let store = crate::registry::FsRegistry::new(&cat);
        let ids: Vec<_> = store.mcps().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["b", "p-a"]);
        run(&args(&["--catalog", cat_s, "rm", "b"])).unwrap();
        run(&args(&["--catalog", cat_s, "ls"])).unwrap();
        assert!(run(&args(&["--catalog", cat_s, "rm", "b"])).is_err());
    }
}
