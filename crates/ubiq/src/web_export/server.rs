//! The shared server: one `tiny_http` listener for the whole process, started lazily on first
//! use and kept alive for the process's lifetime (mirrors the bind/spawn idiom in
//! `crates/agent-manager/src/mcp/server.rs::InProcessServer`, minus its stop flag/`Drop` — that
//! one is scoped to a single run; this one is a process-wide singleton nothing ever tears down).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::routes;

#[derive(Clone)]
pub(super) struct ProjectEntry {
    pub(super) name: String,
    pub(super) root: PathBuf,
}

#[derive(Default)]
pub(super) struct Registry {
    project_to_slug: HashMap<String, String>,
    slug_to_entry: HashMap<String, ProjectEntry>,
}

impl Registry {
    fn register(&mut self, project_id: &str, project_name: &str, root: PathBuf) -> String {
        if let Some(slug) = self.project_to_slug.get(project_id) {
            let slug = slug.clone();
            self.slug_to_entry.insert(
                slug.clone(),
                ProjectEntry {
                    name: project_name.to_string(),
                    root,
                },
            );
            return slug;
        }

        let base = kebab_case(project_name);
        let mut slug = base.clone();
        if self.slug_to_entry.contains_key(&slug) {
            slug = format!("{base}-{}", short_suffix(project_id));
        }
        while self.slug_to_entry.contains_key(&slug) {
            slug.push('x');
        }

        self.project_to_slug
            .insert(project_id.to_string(), slug.clone());
        self.slug_to_entry.insert(
            slug.clone(),
            ProjectEntry {
                name: project_name.to_string(),
                root,
            },
        );
        slug
    }

    pub(super) fn lookup(&self, slug: &str) -> Option<ProjectEntry> {
        self.slug_to_entry.get(slug).cloned()
    }
}

pub(super) type SharedRegistry = Arc<Mutex<Registry>>;

struct RunningServer {
    port: u16,
    registry: SharedRegistry,
}

static SERVER: OnceLock<Mutex<Option<RunningServer>>> = OnceLock::new();

/// Starts the shared server on first call. Registers (or re-registers) `project_id` -> `root`
/// under a slug derived from `project_name`, and returns the base URL to open, e.g.
/// `"http://127.0.0.1:53210/my-project/"`.
pub fn ensure_started_and_registered(
    project_id: &str,
    project_name: &str,
    root: &Path,
) -> Result<String, String> {
    let cell = SERVER.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap();
    if guard.is_none() {
        *guard = Some(start()?);
    }
    let running = guard.as_ref().unwrap();
    let slug =
        running
            .registry
            .lock()
            .unwrap()
            .register(project_id, project_name, root.to_path_buf());
    Ok(format!("http://127.0.0.1:{}/{slug}/", running.port))
}

fn start() -> Result<RunningServer, String> {
    let http_server = tiny_http::Server::http("127.0.0.1:0").map_err(|err| {
        let msg = format!("web export: failed to bind local server: {err}");
        tracing::error!("{msg}");
        msg
    })?;
    let port = http_server
        .server_addr()
        .to_ip()
        .ok_or_else(|| {
            let msg = "web export: local server has no IP listen address".to_string();
            tracing::error!("{msg}");
            msg
        })?
        .port();

    let registry: SharedRegistry = Arc::new(Mutex::new(Registry::default()));
    let thread_registry = Arc::clone(&registry);
    std::thread::Builder::new()
        .name("ubiq-web-export".to_string())
        .spawn(move || serve(http_server, thread_registry))
        .map_err(|err| {
            let msg = format!("web export: failed to spawn server thread: {err}");
            tracing::error!("{msg}");
            msg
        })?;

    Ok(RunningServer { port, registry })
}

fn serve(http_server: tiny_http::Server, registry: SharedRegistry) {
    for request in http_server.incoming_requests() {
        routes::handle(request, &registry);
    }
}

fn kebab_case(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}

fn short_suffix(project_id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    project_id.hash(&mut hasher);
    format!("{:x}", hasher.finish() & 0xff_ffff)
}
