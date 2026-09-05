//! Cloning a repository: where a clone comes from, what a listing returns, and how far a clone
//! has got.
//!
//! A **clone** is how a project enters the catalogue from somewhere other than a folder the user
//! already has. It ends in exactly the same place an [`crate::messages::Message::AddProject`] does
//! — a registered project — which is why there is no clone-success message: the clone registers
//! the project and `ProjectAdded` is the signal.
//!
//! Nothing here holds material. A [`RepoSource::Connection`] names the connection whose token the
//! host lends the clone; the token itself never crosses, on the rule [`crate::connectors`] states.
//!
//! [`parse_repo_url`] is the one place repository-URL sniffing lives. Both the clone modal and the
//! omni search call it, so "is this text a repository?" has a single answer rather than two that
//! drift.

use serde::{Deserialize, Serialize};

use crate::ids::{CloneId, ConnectionId};

/// One repository as a provider listed it.
///
/// Flat and provider-neutral: the interface draws a row, and nothing here is a handle to fetch
/// more with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRepo {
    /// The provider's own identifier — `owner/name` on GitHub, a number on GitLab. Opaque to
    /// Ubiq: it is carried back to the host, never parsed here.
    pub id: String,
    /// The bare name, which is what a folder is called by default.
    pub name: String,
    /// The name a user would recognise, owner included.
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// What a clone checks out when no branch is asked for. `None` where the provider did not say.
    #[serde(default)]
    pub default_branch: Option<String>,
    pub private: bool,
    pub clone_url: String,
    /// When it last saw a push, as the provider wrote it. A string because the interface owns how
    /// a date reads and the host does not parse one it only passes on.
    #[serde(default)]
    pub pushed_at: Option<String>,
}

/// Where a clone comes from.
///
/// The two arms differ in one thing that matters: a connection lends its token, so a private
/// repository is reachable, and a bare URL is anonymous. The interface picks by how the user got
/// here — a row in a listing, or a URL they pasted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoSource {
    /// Through an authenticated identity. `repo` is the provider's own identifier, as
    /// [`RemoteRepo::id`] gave it.
    Connection {
        connection: ConnectionId,
        repo: String,
        clone_url: String,
    },
    /// A URL and nothing else. Whatever the network allows unauthenticated is what this can reach.
    Url(String),
}

/// One clone, as it is asked for.
///
/// `parent` is the folder the clone is created *inside*; the destination is `parent/name`. Split
/// rather than sent as one path because the interface offers the folder and the name as two
/// fields, and joining them is the host's business — it is the half that touches disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneRequest {
    pub clone_id: CloneId,
    pub source: RepoSource,
    /// The branch to check out. `None` is the repository's default.
    #[serde(default)]
    pub branch: Option<String>,
    /// Fetch only the tip. Cheap and usually enough; wrong the moment anyone wants history.
    #[serde(default)]
    pub shallow: bool,
    pub parent: String,
    pub name: String,
    /// Whether this is a throwaway. An ephemeral project lands under the ephemeral root and is the
    /// only kind Ubiq will delete a folder for — which is why the flag travels with the request
    /// rather than being decided after the clone has already written somewhere.
    #[serde(default)]
    pub ephemeral: bool,
}

/// How far a clone has got. One `ClonePending` per change, terminated by a registered project or
/// by exactly one `CloneFailed`.
///
/// Typed rather than a percentage because the phases are not comparable: counting has no total to
/// divide by, and a checkout's numbers are files where receiving's are objects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneStage {
    /// Working out what to clone from where — the listing lookup, the redirect, the handshake.
    Resolving,
    /// The remote is counting objects. There is no total yet, which is the whole reason this is a
    /// stage of its own.
    Counting,
    /// Objects arriving. `bytes` is what has been read, which is the number that keeps moving when
    /// a single large object stalls the count.
    Receiving {
        received: u32,
        total: u32,
        bytes: u64,
    },
    /// Writing the working tree, in files.
    CheckingOut { done: u32, total: u32 },
    /// Cloned; taking the folder into the catalogue. The next thing the interface hears is
    /// `ProjectAdded`.
    Registering,
}

/// Why a clone, or a listing, did not happen.
///
/// Typed for the reason [`crate::connectors::ConnectError`] is: each of these needs a different
/// sentence, and most name something the user can fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneError {
    /// The transfer failed: a reset, a timeout, no route.
    Network(String),
    /// The credential was refused, or there was none and the repository is not public.
    Auth,
    /// No such repository, or none this identity can see — the provider does not distinguish, and
    /// neither does this.
    NotFound,
    /// The destination folder is already there. Refused rather than merged into: a clone that
    /// lands in someone's existing work is the failure nobody can undo.
    Exists,
    /// A URL Ubiq will not clone from — an `ssh`/`git@` remote. Clones go over https, so an ssh
    /// URL is parsed only so this can say what is wrong instead of "not a repository".
    Unsupported(String),
    /// The host declined for a reason of its own: a path outside the roots it will write to, a
    /// name that is not a folder name.
    Refused(String),
}

/// A repository URL, taken apart.
///
/// `clone_url` is always normalised to `https://<host>/<owner>/<name>.git`, whatever form the text
/// arrived in — so an ssh URL yields the https remote its owner would have pasted, and the caller
/// decides whether to refuse it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedRepo {
    pub host: String,
    /// The owner, or the full group path on a provider with subgroups: `group/sub`.
    pub owner: String,
    pub name: String,
    pub clone_url: String,
}

/// GitHub paths that look like `owner/name` and are not. Not a list of every reserved word — the
/// ones a user plausibly pastes, so a settings page does not offer itself as a repository.
const NOT_REPOS: [&str; 5] = [
    "settings",
    "marketplace",
    "orgs",
    "notifications",
    "explore",
];

/// Hosts whose `owner/name` path is a repository without a `.git` suffix to say so. Any other host
/// has to end in `.git`, which is the only way to tell a repository from a page.
const KNOWN_HOSTS: [&str; 2] = ["github.com", "gitlab.com"];

/// Is this text a repository URL, and which one?
///
/// Total by construction: every rejection is `None`, and nothing here panics on input a user typed
/// into a search field one character at a time. Hand-rolled rather than a URL crate because the
/// question is narrower than parsing a URL — a query string or a fragment is a page, not a clone
/// target, so it is rejected rather than dropped.
pub fn parse_repo_url(text: &str) -> Option<ParsedRepo> {
    let text = text.trim();
    // A page's URL, not a repository's: refuse rather than silently clone what is left of it.
    if text.contains('?') || text.contains('#') {
        return None;
    }

    let (host, path, known) = if let Some(rest) = text.strip_prefix("git@") {
        // `git@host:owner/name.git`. Parsed so the caller can say "ssh is not supported"; the
        // clone URL it yields is the https one.
        let (host, path) = rest.split_once(':')?;
        (host, path, true)
    } else {
        let rest = text.strip_prefix("https://")?;
        let (host, path) = rest.split_once('/')?;
        (host, path, false)
    };

    let host = host.trim_end_matches('/').to_ascii_lowercase();
    if host.is_empty() || host.contains('@') {
        return None;
    }

    let path = path.trim_end_matches('/');
    let had_git = path.ends_with(".git");
    let path = path.strip_suffix(".git").unwrap_or(path);
    // Without a `.git` suffix, only the hosts we know the path shape of are repositories.
    if !known && !had_git && !KNOWN_HOSTS.contains(&host.as_str()) {
        return None;
    }

    let (owner, name) = path.rsplit_once('/')?;
    if owner.is_empty() || name.is_empty() || owner.split('/').any(str::is_empty) {
        return None;
    }
    // Subgroups are a GitLab thing; a deep path on GitHub is a page inside a repository.
    let first = owner.split('/').next().unwrap_or(owner);
    if host == "github.com" && (owner.contains('/') || NOT_REPOS.contains(&first)) {
        return None;
    }

    Some(ParsedRepo {
        clone_url: format!("https://{host}/{owner}/{name}.git"),
        host,
        owner: owner.to_string(),
        name: name.to_string(),
    })
}
