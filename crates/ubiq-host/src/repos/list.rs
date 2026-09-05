//! The provider calls a clone needs before it can start: which repositories, and which branches.
//!
//! Everything here is the connector family's machinery reused unchanged —
//! [`crate::connectors::providers`] for where an instance's API lives,
//! [`crate::connectors::http`] for the request, and the pin the user vouched for at that origin.
//! A listing is a network call like a probe is, so nothing here runs on the coordinator's thread;
//! [`super::Repos`] is what puts it on one of its own.
//!
//! **Every field of a row but two is optional.** Providers disagree about all of them, and a
//! listing that refused the whole page because one repository had no description would be worse
//! than one that skipped it. A row without a usable clone URL and a name is skipped — there is
//! nothing to clone and nothing to call it.
//!
//! Branches for a pasted URL have no API to ask: there is no connection, so git itself is asked,
//! anonymously, over the same https the clone would use.

use serde_json::Value;
use ubiq_proto::connectors::{ConnectError, ProviderId};
use ubiq_proto::repos::{CloneError, RemoteRepo};

use crate::connectors::flow::encode;
use crate::connectors::http::{self, Failure};
use crate::connectors::providers;

/// How many pages of a listing are read before it is called truncated.
///
/// A ceiling rather than "everything": an account with thousands of repositories would otherwise
/// spend a minute of somebody's time on rows the interface filters away. What it costs is honesty
/// about having stopped, which is what the `truncated` flag buys back.
const REPO_PAGES: u32 = 5;

/// Who is asking, resolved: the connection's provider, where it lives, and what it may present.
///
/// One struct rather than five parameters threaded through every call, because they always travel
/// together and always come from the same record.
pub struct Identity {
    pub provider: ProviderId,
    pub instance: Option<String>,
    /// The provider's own name for the account, which is what a server-side search scopes to.
    pub account: String,
    pub token: Option<String>,
    /// The certificate vouched for at this instance's origin, if one was.
    pub pin: Option<String>,
}

/// The repositories this identity can see, and whether the listing stopped short.
///
/// `query` is `None` for what the provider offers unprompted — the user's own repositories, most
/// recently pushed first — and `Some` only when the interface's own filter over that came up
/// empty. A search is one page: it is already the provider's answer to "which ones", so paging it
/// would be paging a shortlist.
pub fn repos(who: &Identity, query: Option<&str>) -> Result<(Vec<RemoteRepo>, bool), CloneError> {
    let instance = who.instance.as_deref();
    if let Some(query) = query {
        let path = match who.provider {
            ProviderId::Github => format!(
                "/search/repositories?q={}+in:name+user:{}",
                encode(query),
                encode(&who.account)
            ),
            ProviderId::Gitlab => format!(
                "/projects?membership=true&order_by=last_activity_at&per_page=100&search={}",
                encode(query)
            ),
            ProviderId::Gitea => format!("/repos/search?q={}&limit=50", encode(query)),
            other => return Err(unsupported(other)),
        };
        let url = providers::api_url(who.provider, instance, &path).map_err(connect)?;
        let body = get(who, &url)?;
        return Ok((rows(who.provider, &body), false));
    }

    let mut found = Vec::new();
    for page in 1..=REPO_PAGES {
        let path = match who.provider {
            ProviderId::Github => format!("/user/repos?sort=pushed&per_page=100&page={page}"),
            ProviderId::Gitlab => format!(
                "/projects?membership=true&order_by=last_activity_at&per_page=100&page={page}"
            ),
            ProviderId::Gitea => format!("/user/repos?limit=50&page={page}"),
            other => return Err(unsupported(other)),
        };
        let url = providers::api_url(who.provider, instance, &path).map_err(connect)?;
        let body = get(who, &url)?;
        let page_size = array(who.provider, &body).len();
        found.extend(rows(who.provider, &body));
        // A short page is the end of the listing, whatever the ceiling says. Only stopping *at*
        // the ceiling is a truncation.
        if page_size < per_page(who.provider) {
            return Ok((found, false));
        }
    }
    Ok((found, true))
}

/// One repository's branches, and the one it defaults to, through the provider's API.
///
/// `repo` is [`RemoteRepo::id`] as the listing gave it, carried back opaque. The default branch is
/// deliberately not fetched: the interface already holds it on the row the user picked, and a
/// second round trip to learn what it can already read is a round trip for nothing.
pub fn branches(who: &Identity, repo: &str) -> Result<(Vec<String>, Option<String>), CloneError> {
    let path = match who.provider {
        ProviderId::Github => format!("/repos/{repo}/branches?per_page=100"),
        ProviderId::Gitlab => format!(
            "/projects/{}/repository/branches?per_page=100",
            encode(repo)
        ),
        ProviderId::Gitea => format!("/repos/{repo}/branches"),
        other => return Err(unsupported(other)),
    };
    let url = providers::api_url(who.provider, who.instance.as_deref(), &path).map_err(connect)?;
    let body = get(who, &url)?;
    let mut names = Vec::new();
    let mut default = None;
    for row in body.as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(name) = string(row, "name") else {
            continue;
        };
        // GitLab says which one is the default in the row itself; the other two do not, and the
        // interface has the repository's own answer.
        if row.get("default").and_then(Value::as_bool) == Some(true) {
            default = Some(name.clone());
        }
        names.push(name);
    }
    Ok((names, default))
}

/// The same question of a URL nobody is authenticated to.
///
/// There is no API to ask — a pasted URL names a host, not a product — so git is asked directly.
/// The refs are listed without fetching anything, which is why this is cheap enough to do while a
/// dialog is open.
pub fn remote_branches(url: &str) -> Result<(Vec<String>, Option<String>), CloneError> {
    if !url.starts_with("https://") {
        return Err(CloneError::Unsupported(url.to_string()));
    }
    let mut remote = git2::Remote::create_detached(url).map_err(git)?;
    remote.connect(git2::Direction::Fetch).map_err(git)?;
    let names = remote
        .list()
        .map_err(git)?
        .iter()
        .filter_map(|head| head.name().strip_prefix("refs/heads/").map(String::from))
        .collect();
    // `default_branch` answers a full ref, not a name.
    let default = remote
        .default_branch()
        .ok()
        .and_then(|branch| branch.as_str().ok().map(str::to_string))
        .map(|name| name.trim_start_matches("refs/heads/").to_string());
    let _ = remote.disconnect();
    Ok((names, default))
}

// ── reading a provider's rows ────────────────────────────────────────

/// Where the rows are in a body. GitHub's search wraps them, Gitea's search wraps them
/// differently, and every plain listing is the array itself.
fn array(provider: ProviderId, body: &Value) -> &[Value] {
    let rows = match provider {
        ProviderId::Github => body.get("items").or(Some(body)),
        ProviderId::Gitea => body.get("data").or(Some(body)),
        _ => Some(body),
    };
    rows.and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// One page of rows, as the interface draws them. Anything unreadable is skipped, never fatal.
fn rows(provider: ProviderId, body: &Value) -> Vec<RemoteRepo> {
    array(provider, body)
        .iter()
        .filter_map(|row| repo(provider, row))
        .collect()
}

/// One row, in whichever shape this provider writes it.
///
/// The same shape as [`providers::account_name`]: one `match`, one place that knows what a
/// provider calls a field. `None` is a row with no clone URL or no name — nothing to clone, and
/// nothing to call it.
fn repo(provider: ProviderId, row: &Value) -> Option<RemoteRepo> {
    let (id, name, full_name, private, clone_url, pushed_at) = match provider {
        ProviderId::Gitlab => (
            row.get("id").map(|id| id.to_string())?,
            string(row, "path").or_else(|| string(row, "name"))?,
            string(row, "path_with_namespace").unwrap_or_default(),
            string(row, "visibility").as_deref() != Some("public"),
            string(row, "http_url_to_repo")?,
            string(row, "last_activity_at"),
        ),
        // Gitea writes GitHub's shape, down to the field names.
        ProviderId::Github | ProviderId::Gitea => (
            string(row, "full_name").or_else(|| string(row, "name"))?,
            string(row, "name")?,
            string(row, "full_name").unwrap_or_default(),
            row.get("private").and_then(Value::as_bool).unwrap_or(true),
            string(row, "clone_url")?,
            string(row, "pushed_at").or_else(|| string(row, "updated_at")),
        ),
        _ => return None,
    };
    Some(RemoteRepo {
        full_name: if full_name.is_empty() {
            name.clone()
        } else {
            full_name
        },
        id,
        name,
        description: string(row, "description"),
        default_branch: string(row, "default_branch"),
        private,
        clone_url,
        pushed_at,
    })
}

fn string(row: &Value, key: &str) -> Option<String> {
    row.get(key)?.as_str().map(str::to_string)
}

/// How many rows a full page holds for this provider, so a short one can be recognised.
fn per_page(provider: ProviderId) -> usize {
    match provider {
        ProviderId::Gitea => 50,
        _ => 100,
    }
}

// ── failures ─────────────────────────────────────────────────────────

fn get(who: &Identity, url: &str) -> Result<Value, CloneError> {
    http::get_json(who.pin.as_deref(), url, who.token.as_deref()).map_err(failure)
}

/// What a failed request means to a clone.
///
/// The status code is read back out of the sentence [`crate::connectors::http`] wrote, because
/// that is where it is: `Failure` is the connector family's vocabulary and keeps the code only in
/// the text. Faithful rather than clever — anything that is not one of the three codes that name
/// something the user can act on is a network failure, which is what it looks like from here.
fn failure(failure: Failure) -> CloneError {
    match failure {
        Failure::Connect(ConnectError::Http(detail)) => match status(&detail) {
            Some(401 | 403) => CloneError::Auth,
            Some(404) => CloneError::NotFound,
            _ => CloneError::Network(detail),
        },
        Failure::Connect(error) => connect(error),
        Failure::Certificate(cert) => CloneError::Network(format!(
            "the server offered a certificate nobody has vouched for ({})",
            cert.sha256
        )),
        // Nothing here holds a cancel channel, so this is unreachable; it is still a failed call.
        Failure::Cancelled => CloneError::Network("the request was abandoned".to_string()),
    }
}

/// The code out of `HTTP 404: ...`, which is the only place `get_json` keeps one.
fn status(detail: &str) -> Option<u16> {
    detail
        .strip_prefix("HTTP ")?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn connect(error: ConnectError) -> CloneError {
    match error {
        ConnectError::BadInstance => {
            CloneError::Refused("that connection's instance is not a URL".to_string())
        }
        ConnectError::Tls(detail) | ConnectError::Http(detail) => CloneError::Network(detail),
        other => CloneError::Network(format!("{other:?}")),
    }
}

fn git(error: git2::Error) -> CloneError {
    super::clone::error(error)
}

/// A provider with no repository listing this build can write. Not a failure of the connection:
/// there is nothing here to clone from in the first place.
fn unsupported(provider: ProviderId) -> CloneError {
    CloneError::Unsupported(format!("{provider:?} lists no repositories"))
}
