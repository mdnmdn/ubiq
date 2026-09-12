//! The shared server: one `tiny_http` listener for the whole process, started lazily on first
//! use and kept alive for the process's lifetime (mirrors the bind/spawn idiom in
//! `crates/agent-manager/src/mcp/server.rs::InProcessServer`, minus its stop flag/`Drop` — that
//! one is scoped to a single run; this one is a process-wide singleton nothing ever tears down).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use super::archive;
use super::routes;

#[derive(Clone)]
pub(super) struct ProjectEntry {
    pub(super) name: String,
    pub(super) root: PathBuf,
}

/// One open web panel, as the interface holds it: which app it runs, the token that authenticates
/// every frame, and the URL to point a container at.
#[derive(Clone, Debug)]
pub struct WebSession {
    pub app: &'static str,
    pub token: String,
    pub url: String,
}

/// The two queues behind one session, with their own lock and condition variable so a parked
/// long-poll never holds the registry.
#[derive(Default)]
struct Queues {
    /// Interface -> web, waiting for the next `GET bridge`.
    outbound: VecDeque<serde_json::Value>,
    /// Web -> interface, waiting for the next `receive`.
    inbound: Vec<serde_json::Value>,
    closed: bool,
}

/// One live web session's server-side state. Shared by `Arc` so a route can hold it after the
/// registry lock is dropped.
pub(super) struct WebChannel {
    pub(super) app: &'static str,
    /// The vendor archive this app's `vendor/` route serves out of — one compressed bundle file,
    /// opened once by `set_vendor_root` and held for the session's life; a route seeks into it
    /// and inflates one entry per request, nothing is ever extracted to disk. `None` until a
    /// bundle lands, or if it failed to open. It is behind its own lock because the bundle is
    /// ensured after the panel is already open, and a route only ever needs to lock it for the
    /// one entry it reads.
    vendor_root: Mutex<Option<archive::Reader>>,
    queues: Mutex<Queues>,
    wake: Condvar,
}

/// How long a `GET bridge` parks before answering `[]`. Long enough that an idle panel is not a
/// request treadmill, short enough that a closed container's last poll goes away promptly.
const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(5);

impl WebChannel {
    fn new(app: &'static str) -> Self {
        Self {
            app,
            vendor_root: Mutex::new(None),
            queues: Mutex::new(Queues::default()),
            wake: Condvar::new(),
        }
    }

    /// Locks the archive and reads one entry out of it, if one is open and `path` is in it.
    pub(super) fn read_vendor(&self, path: &str) -> Option<Vec<u8>> {
        self.vendor_root.lock().unwrap().as_mut()?.read(path)
    }

    fn push_outbound(&self, frame: serde_json::Value) {
        let mut queues = self.queues.lock().unwrap();
        queues.outbound.push_back(frame);
        self.wake.notify_all();
    }

    /// Drains the outbound queue, parking on the condition variable until there is something to
    /// drain or [`LONG_POLL_TIMEOUT`] elapses. Answers an empty vector on timeout and on close.
    pub(super) fn poll_outbound(&self) -> Vec<serde_json::Value> {
        let queues = self.queues.lock().unwrap();
        let (mut queues, _timeout) = self
            .wake
            .wait_timeout_while(queues, LONG_POLL_TIMEOUT, |q| {
                q.outbound.is_empty() && !q.closed
            })
            .unwrap();
        queues.outbound.drain(..).collect()
    }

    pub(super) fn push_inbound(&self, frame: serde_json::Value) {
        self.queues.lock().unwrap().inbound.push(frame);
    }

    fn drain_inbound(&self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.queues.lock().unwrap().inbound)
    }

    /// Wakes every parked poll so a closed session's container is not held for the full timeout.
    fn close(&self) {
        let mut queues = self.queues.lock().unwrap();
        queues.closed = true;
        queues.outbound.clear();
        self.wake.notify_all();
    }
}

#[derive(Default)]
pub(super) struct Registry {
    project_to_slug: HashMap<String, String>,
    slug_to_entry: HashMap<String, ProjectEntry>,
    token_to_web: HashMap<String, Arc<WebChannel>>,
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

    pub(super) fn lookup_web(&self, token: &str) -> Option<Arc<WebChannel>> {
        self.token_to_web.get(token).cloned()
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

/// Opens one web session: starts the server if it is not up, mints a token, registers a channel
/// for it, and returns the URL a container opens — `http://127.0.0.1:<port>/_web/<app>/<token>/`.
///
/// The token is not decoration. The server answers whatever asks on loopback, and this bridge can
/// carry a save, so every frame repeats the token and a frame without it is dropped. Loopback is
/// not an authentication boundary and this does not treat it as one.
pub fn open_session(app: &'static str) -> Result<WebSession, String> {
    let token = mint_token()?;
    let cell = SERVER.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap();
    if guard.is_none() {
        *guard = Some(start()?);
    }
    let running = guard.as_ref().unwrap();
    running
        .registry
        .lock()
        .unwrap()
        .token_to_web
        .insert(token.clone(), Arc::new(WebChannel::new(app)));
    Ok(WebSession {
        url: format!("http://127.0.0.1:{}/_web/{app}/{token}/", running.port),
        app,
        token,
    })
}

/// Queues one interface -> web frame and wakes a parked long-poll. A frame for a token with no
/// live session is dropped.
pub fn send(token: &str, frame: serde_json::Value) {
    match channel(token) {
        Some(channel) => channel.push_outbound(frame),
        None => tracing::debug!("web bridge: send to a token with no live session"),
    }
}

/// Drains the web -> interface frames queued since the last call. The panel polls this.
pub fn receive(token: &str) -> Vec<serde_json::Value> {
    channel(token)
        .map(|channel| channel.drain_inbound())
        .unwrap_or_default()
}

/// Points the session's `vendor/` route at a compressed vendor archive on disk — opened once
/// here and held for the life of the session. This is a later call rather than an argument to
/// `open_session`, since the bundle is ensured after the panel is already open. `None` clears it;
/// an archive that fails to open is logged and treated the same as `None`.
pub fn set_vendor_root(token: &str, root: Option<PathBuf>) {
    let Some(channel) = channel(token) else {
        return;
    };
    let reader = root.and_then(|path| match archive::Reader::open(&path) {
        Ok(reader) => Some(reader),
        Err(err) => {
            tracing::error!("web panel: failed to open vendor archive at {path:?}: {err}");
            None
        }
    });
    *channel.vendor_root.lock().unwrap() = reader;
}

/// Drops the session. Every subsequent request on that token 404s.
pub fn close_session(token: &str) {
    let Some(cell) = SERVER.get() else { return };
    let guard = cell.lock().unwrap();
    let Some(running) = guard.as_ref() else {
        return;
    };
    let removed = running.registry.lock().unwrap().token_to_web.remove(token);
    if let Some(channel) = removed {
        channel.close();
    }
}

fn channel(token: &str) -> Option<Arc<WebChannel>> {
    let cell = SERVER.get()?;
    let guard = cell.lock().unwrap();
    let running = guard.as_ref()?;
    running.registry.lock().unwrap().lookup_web(token)
}

/// 192 bits from the platform's CSPRNG, hex-encoded. `rustls` is already a dependency (the remote
/// dialer's TLS), and its `ring` provider exposes exactly this — no new dependency, and no
/// hand-rolled generator. A ULID would not do: the process's generator is monotonic by design,
/// which is the opposite of what a secret wants.
pub(super) fn mint_token() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut bytes)
        .map_err(|_| {
            let msg = "web panel: no secure random source for the session token".to_string();
            tracing::error!("{msg}");
            msg
        })?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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
