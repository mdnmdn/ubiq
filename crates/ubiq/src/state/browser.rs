//! The internal browser's data: the tabs a project has open, the history its address field offers,
//! and the rules that turn what was typed into a place to go.
//!
//! Renders nothing and names no webview. A tab here is a key (`browser:<n>`, which is also its
//! dock panel's key and the id the MCP surface will address it by), what the page last said about
//! itself, and the address field's widget — the one widget this state holds, because the field's
//! text is the model of what is being typed.

use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{Entity, Subscription};
use gpui_component::input::InputState;

/// The search engines the address field can send a query to. The one place an engine is named:
/// each carries its own URL template and start page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Google,
    Bing,
    Brave,
    Ecosia,
    Startpage,
}

impl SearchEngine {
    pub const ALL: [SearchEngine; 6] = [
        SearchEngine::DuckDuckGo,
        SearchEngine::Google,
        SearchEngine::Bing,
        SearchEngine::Brave,
        SearchEngine::Ecosia,
        SearchEngine::Startpage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "DuckDuckGo",
            SearchEngine::Google => "Google",
            SearchEngine::Bing => "Bing",
            SearchEngine::Brave => "Brave",
            SearchEngine::Ecosia => "Ecosia",
            SearchEngine::Startpage => "Startpage",
        }
    }

    /// The URL a query is appended to, already percent-encoded.
    fn template(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "https://duckduckgo.com/?q=",
            SearchEngine::Google => "https://www.google.com/search?q=",
            SearchEngine::Bing => "https://www.bing.com/search?q=",
            SearchEngine::Brave => "https://search.brave.com/search?q=",
            SearchEngine::Ecosia => "https://www.ecosia.org/search?q=",
            SearchEngine::Startpage => "https://www.startpage.com/do/search?q=",
        }
    }

    /// What a tab opens on when nothing was asked for.
    pub fn home(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "https://duckduckgo.com/",
            SearchEngine::Google => "https://www.google.com/",
            SearchEngine::Bing => "https://www.bing.com/",
            SearchEngine::Brave => "https://search.brave.com/",
            SearchEngine::Ecosia => "https://www.ecosia.org/",
            SearchEngine::Startpage => "https://www.startpage.com/",
        }
    }

    pub fn search_url(self, query: &str) -> String {
        format!("{}{}", self.template(), encode(query))
    }
}

/// Which cookie jar and storage a tab's webview uses: the one Ubiq-wide profile, or a project's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    Common,
    Private(ubiq_proto::ids::ProjectId),
}

impl Profile {
    /// The 16-byte id macOS keys a persistent data store by: one fixed id for the common profile,
    /// the project's own id for a private one.
    pub fn data_id(self) -> [u8; 16] {
        match self {
            Profile::Common => *b"ubiq.browser.cmn",
            Profile::Private(project) => project.as_ulid().to_bytes(),
        }
    }

    /// The directory name Windows keeps the profile under.
    pub fn dir_name(self) -> String {
        match self {
            Profile::Common => "common".to_string(),
            Profile::Private(project) => format!("project-{}", project.as_ulid()),
        }
    }
}

/// The zoom steps, as page-zoom factors.
const ZOOMS: &[f64] = &[
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

/// The next step from `now`, up or down, staying at the end of the range.
pub fn zoom_step(now: f64, up: bool) -> f64 {
    let at = ZOOMS
        .iter()
        .position(|z| (*z - now).abs() < 0.001)
        .unwrap_or_else(|| ZOOMS.iter().position(|z| *z >= now).unwrap_or(ZOOMS.len() - 1));
    let to = match up {
        true => (at + 1).min(ZOOMS.len() - 1),
        false => at.saturating_sub(1),
    };
    ZOOMS[to]
}

/// How many pages the history remembers per project.
pub const HISTORY_MAX: usize = 500;

/// How many rows the address dropdown offers.
pub const SUGGEST_MAX: usize = 8;

/// A top-level domain this build trusts to mean "that is an address" for a bare `name.tld`.
/// A short list on purpose: `README.md` and `main.rs` are file names, not sites.
const TLDS: &[&str] = &[
    "com", "org", "net", "io", "dev", "co", "it", "de", "fr", "uk", "edu", "gov", "info", "me",
    "xyz", "tv", "eu", "us", "ca", "es", "nl", "ch", "cloud",
];

static NEXT: AtomicU64 = AtomicU64::new(1);

/// One page the address field can offer again.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
}

/// One open browser tab.
pub struct BrowserTab {
    /// `browser:<n>` — stable for the life of the tab.
    pub key: String,
    /// Where the page is now, as the webview last reported it.
    pub url: String,
    /// The document's own title; empty until it arrives.
    pub title: String,
    pub loading: bool,
    /// When `loading` last became true, so a load that never reports its end can be given up on.
    pub loading_since: Option<std::time::Instant>,
    /// Whether a page has ever finished loading: before that the webview may be holding scripts
    /// back, so the UI answers what it can from its own state.
    pub committed: bool,
    /// What the address field should show next time it is not being typed in.
    pub pending_field: Option<String>,
    /// The address field. Built in `render`, where there is a window.
    pub input: Option<Entity<InputState>>,
    pub subscription: Option<Subscription>,
    /// The profile the webview was (or will be) built on. Fixed for the life of the tab: a change
    /// in the settings reaches a tab when it is reopened.
    pub profile: Profile,
    /// Whether that profile is kept on disk (false: an ephemeral, incognito store).
    pub persist: bool,
    /// Page zoom, 1.0 being 100%.
    pub zoom: f64,
}

impl BrowserTab {
    pub fn new(url: &str, profile: Profile, persist: bool) -> Self {
        Self {
            key: format!("browser:{}", NEXT.fetch_add(1, Ordering::Relaxed)),
            url: url.to_string(),
            title: String::new(),
            loading: false,
            loading_since: None,
            committed: false,
            pending_field: Some(url.to_string()),
            input: None,
            subscription: None,
            profile,
            persist,
            zoom: 1.0,
        }
    }

    /// What the tab is called: the page's title, else its host, else its URL.
    pub fn label(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.trim().to_string();
        }
        if self.url.is_empty() || self.url == "about:blank" {
            return "New tab".to_string();
        }
        host_of(&self.url).unwrap_or(&self.url).to_string()
    }
}

/// A project's browser: its tabs and what it has visited.
#[derive(Default)]
pub struct BrowserState {
    pub tabs: Vec<BrowserTab>,
    history: Vec<HistoryEntry>,
    /// Whether this window has asked the host for the project's history yet.
    pub history_requested: bool,
}

impl BrowserState {
    pub fn tab(&self, key: &str) -> Option<&BrowserTab> {
        self.tabs.iter().find(|tab| tab.key == key)
    }

    pub fn tab_mut(&mut self, key: &str) -> Option<&mut BrowserTab> {
        self.tabs.iter_mut().find(|tab| tab.key == key)
    }

    /// Where the address dropdown reads its pages from. Today the session's own list; the host's
    /// history store replaces what is behind this one function.
    pub fn history_source(&self) -> &[HistoryEntry] {
        &self.history
    }

    /// Replace the history with the host's, which is the record: most recent first.
    pub fn set_history(&mut self, entries: Vec<ubiq_proto::browser::BrowserHistoryEntry>) {
        self.history = entries
            .into_iter()
            .map(|entry| HistoryEntry {
                url: entry.url,
                title: entry.title,
            })
            .collect();
    }

    /// Forget every page the address dropdown offers.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Remember a finished page, newest first, one row per URL.
    pub fn record(&mut self, url: &str, title: &str) {
        if url.is_empty() || url == "about:blank" {
            return;
        }
        self.history.retain(|entry| entry.url != url);
        self.history.insert(
            0,
            HistoryEntry {
                url: url.to_string(),
                title: title.to_string(),
            },
        );
        self.history.truncate(HISTORY_MAX);
    }

    /// A title that arrived after its page finished.
    pub fn retitle(&mut self, url: &str, title: &str) {
        if let Some(entry) = self.history.iter_mut().find(|entry| entry.url == url) {
            entry.title = title.to_string();
        }
    }

    /// The history rows a typed string keeps: every word of it somewhere in the URL or title.
    pub fn suggestions(&self, typed: &str) -> Vec<HistoryEntry> {
        suggest(self.history_source(), typed)
    }
}

/// [`BrowserState::suggestions`] over any list.
pub fn suggest(history: &[HistoryEntry], typed: &str) -> Vec<HistoryEntry> {
    let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    history
        .iter()
        .filter(|entry| {
            let hay = format!("{} {}", entry.url, entry.title).to_lowercase();
            words.iter().all(|word| hay.contains(word.as_str()))
        })
        .take(SUGGEST_MAX)
        .cloned()
        .collect()
}

/// The host of a URL, without scheme, credentials, port or path.
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

/// Whether a string reads as an address rather than words to search for: a scheme, `localhost`, an
/// IPv4 address, or `name.tld` with a trusted top-level domain.
pub fn looks_like_url(input: &str) -> bool {
    let input = input.trim();
    if input.is_empty() || input.contains(char::is_whitespace) {
        return false;
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        return input.len() > "https://".len();
    }
    let authority = input.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.split(':').next().unwrap_or("");
    if host == "localhost" {
        return true;
    }
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
    {
        return true;
    }
    parts.len() >= 2
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '-'))
        && parts
            .last()
            .is_some_and(|tld| TLDS.contains(&tld.to_lowercase().as_str()))
}

/// What a typed string means as a place: an address is made absolute, anything else is a search.
pub fn normalize(input: &str, engine: SearchEngine) -> String {
    let input = input.trim();
    if input.is_empty() {
        return engine.home().to_string();
    }
    if input.contains("://") || input.starts_with("about:") {
        return input.to_string();
    }
    if looks_like_url(input) {
        let host = host_of(input).unwrap_or("");
        let local = host == "localhost" || host.chars().all(|c| c.is_ascii_digit() || c == '.');
        return format!("{}://{input}", if local { "http" } else { "https" });
    }
    engine.search_url(input)
}

/// What ⌘K's "Open in internal browser" row should open for a query — the raw text, which
/// `AppState::open_browser` turns into a place with the chosen engine — or `None` when the query
/// is not about the browser at all. `Some("")` is a blank tab.
pub fn nav_target(query: &str) -> Option<String> {
    let query = query.trim();
    let lower = query.to_lowercase();
    if let Some(rest) = lower.strip_prefix("browser")
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        let rest = query["browser".len()..].trim();
        return Some(rest.to_string());
    }
    if looks_like_url(query) {
        return Some(query.to_string());
    }
    None
}

/// Whether the query asks for the browser outright — an `http(s)://` scheme or the `browser`
/// word — rather than merely looking like an address. Only an explicit ask outranks the search row.
pub fn nav_explicit(query: &str) -> bool {
    let lower = query.trim().to_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower
            .strip_prefix("browser")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// Percent-encode a query for a URL's query string.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_searches() {
        let ddg = SearchEngine::DuckDuckGo;
        assert_eq!(normalize("example.com", ddg), "https://example.com");
        assert_eq!(normalize("localhost:3000/x", ddg), "http://localhost:3000/x");
        assert_eq!(normalize("https://a.b/c", ddg), "https://a.b/c");
        assert_eq!(
            normalize("rust lifetimes", ddg),
            "https://duckduckgo.com/?q=rust+lifetimes"
        );
        assert!(normalize("x y", SearchEngine::Google).starts_with("https://www.google.com/"));
        assert_eq!(zoom_step(1.0, true), 1.1);
        assert_eq!(zoom_step(0.5, false), 0.5);
        assert!(!looks_like_url("README.md"));
        assert!(!looks_like_url("src/main.rs"));
    }

    #[test]
    fn navigator_targets() {
        assert_eq!(nav_target("browser"), Some(String::new()));
        assert_eq!(nav_target("browser foo.com"), Some("foo.com".to_string()));
        assert_eq!(nav_target("router"), None);
        assert_eq!(nav_target("example.com").as_deref(), Some("example.com"));
        assert_eq!(nav_target("http"), None);
        assert_eq!(nav_target("httpx"), None);
        assert_eq!(nav_target("browserify"), None);
        assert_eq!(nav_target("deploy.sh"), None);
        assert_eq!(nav_target("http_client.rs"), None);
    }
}
