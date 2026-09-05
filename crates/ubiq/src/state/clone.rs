//! Cloning a project, as the interface holds it while the modal is up.
//!
//! One struct for the whole modal rather than a step enum: unlike a connect flow, every field here
//! is answerable at any time — a URL can be pasted with a connection already chosen, and the
//! destination is editable while a listing is still arriving. What decides *which* source a clone
//! uses is [`CloneMode`], set by whichever half the user last touched.
//!
//! **An answer naming an id this state no longer holds is discarded**, the discipline every
//! id-carrying family in the window follows. That is what [`CloneState::accept_repos`] and its
//! siblings are: they answer whether the answer was still wanted, and the caller draws nothing
//! when it was not.
//!
//! Nothing here touches a path. `parent` is a string the host or the platform's own chooser gave
//! us, and it is handed back unread — see the rule in `AGENTS.md`.

use ubiq_proto::ids::{CloneId, ConnectionId, RepoQueryId};
use ubiq_proto::repos::{
    CloneError, CloneStage, ParsedRepo, RemoteRepo, RepoSource, parse_repo_url,
};

use crate::state::navigator::subsequence;

/// Which half of the modal the clone is coming from.
///
/// Held rather than inferred, because both halves can be filled at once: a repository stays
/// selected while a URL is being pasted, and picking either is what says which one is meant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CloneMode {
    /// A row from a connection's listing.
    #[default]
    Connection,
    /// A URL, and whatever the network allows unauthenticated.
    Url,
}

/// The clone modal, while it is up.
#[derive(Default)]
pub struct CloneState {
    pub mode: CloneMode,
    /// Whose listing is being shown. `None` while there are no connections, or before one is
    /// picked.
    pub connection: Option<ConnectionId>,
    pub repos: Vec<RemoteRepo>,
    /// The listing hit the provider's page ceiling, so an empty local filter is not proof that
    /// nothing matches — it is what lets [`CloneState::wants_server_search`] say so.
    pub truncated: bool,
    pub filter: String,
    /// The row that was picked, held whole rather than as an index: a re-fetched listing
    /// renumbers, and a selection that silently moved to another repository is the one bug this
    /// modal cannot afford.
    pub repo: Option<RemoteRepo>,
    pub branches: Vec<String>,
    pub branch: Option<String>,
    pub url: String,
    /// The folder the clone is created inside. Empty means the host's own root, which the
    /// interface never names.
    pub parent: String,
    pub name: String,
    pub ephemeral: bool,
    pub shallow: bool,
    /// The listing this state is waiting on, and the branch listing beside it. Two fields because
    /// both can be in flight at once and each discards its own stale answers.
    pub repos_query: Option<RepoQueryId>,
    pub branches_query: Option<RepoQueryId>,
    pub clone_id: Option<CloneId>,
    pub stage: Option<CloneStage>,
    pub error: Option<CloneError>,
}

impl CloneState {
    /// The rows the list draws: the fetched listing, filtered in memory.
    ///
    /// **In memory, not by asking again.** A provider's listing is small enough to filter here,
    /// and a round trip per keystroke is a list that flickers. The server is only asked when this
    /// yields nothing over a truncated listing — [`Self::wants_server_search`].
    pub fn visible(&self) -> Vec<&RemoteRepo> {
        let needle = self.filter.trim().to_lowercase();
        self.repos
            .iter()
            .filter(|repo| {
                subsequence(
                    &needle,
                    &format!(
                        "{} {}",
                        repo.full_name,
                        repo.description.as_deref().unwrap_or_default()
                    ),
                )
            })
            .collect()
    }

    /// Whether the filter has run out of local answers and the provider still has pages to give.
    pub fn wants_server_search(&self) -> bool {
        !self.filter.trim().is_empty() && self.truncated && self.visible().is_empty()
    }

    /// Where a clone would come from as the modal stands, or `None` while nothing names one.
    pub fn source(&self) -> Option<RepoSource> {
        match self.mode {
            CloneMode::Connection => {
                let repo = self.repo.as_ref()?;
                Some(RepoSource::Connection {
                    connection: self.connection?,
                    repo: repo.id.clone(),
                    clone_url: repo.clone_url.clone(),
                })
            }
            CloneMode::Url => match check_url(&self.url)? {
                Ok(parsed) => Some(RepoSource::Url(parsed.clone_url)),
                Err(_) => None,
            },
        }
    }

    /// What a fresh clone would be called: the repository's own name, which the user may rename.
    pub fn default_name(&self) -> String {
        match self.mode {
            CloneMode::Connection => self.repo.as_ref().map(|repo| repo.name.clone()),
            CloneMode::Url => match check_url(&self.url) {
                Some(Ok(parsed)) => Some(parsed.name),
                _ => None,
            },
        }
        .unwrap_or_default()
    }

    /// A listing arrived. `false` means it named an id this state no longer holds, and nothing was
    /// applied.
    pub fn accept_repos(
        &mut self,
        query_id: RepoQueryId,
        repos: Vec<RemoteRepo>,
        truncated: bool,
    ) -> bool {
        if self.repos_query != Some(query_id) {
            return false;
        }
        self.repos_query = None;
        self.repos = repos;
        self.truncated = truncated;
        self.error = None;
        true
    }

    /// A branch listing arrived. The default is preselected only when the user has not already
    /// picked one — the picker showed the repository's own default while this was loading, and
    /// overwriting a deliberate choice with it would be a click undone by the network.
    pub fn accept_branches(
        &mut self,
        query_id: RepoQueryId,
        branches: Vec<String>,
        default: Option<String>,
    ) -> bool {
        if self.branches_query != Some(query_id) {
            return false;
        }
        self.branches_query = None;
        self.branches = branches;
        if self.branch.is_none() {
            self.branch = default;
        }
        true
    }

    /// A listing failed. Either query may be the one that named it, so both are answered here.
    pub fn accept_error(&mut self, query_id: RepoQueryId, error: CloneError) -> bool {
        if self.repos_query == Some(query_id) {
            self.repos_query = None;
        } else if self.branches_query == Some(query_id) {
            self.branches_query = None;
        } else {
            return false;
        }
        self.error = Some(error);
        true
    }
}

/// What a pasted URL amounts to. `None` is an empty field, which is not yet wrong; `Err` is the
/// sentence the modal prints under it.
///
/// The sniffing itself is [`parse_repo_url`]'s and is never redone here — an ssh remote parses,
/// and is refused with a sentence rather than passed off as "not a repository", which is the whole
/// reason that function bothers to parse one.
pub fn check_url(text: &str) -> Option<Result<ParsedRepo, &'static str>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with("git@") || text.starts_with("ssh://") {
        return Some(Err("Ubiq clones over https. Paste the https URL instead."));
    }
    Some(parse_repo_url(text).ok_or("That is not a repository URL."))
}

/// How far a clone has got, as a line under the modal.
pub fn stage_note(stage: &CloneStage) -> String {
    match stage {
        CloneStage::Resolving => "Resolving\u{2026}".to_string(),
        CloneStage::Counting => "Counting objects\u{2026}".to_string(),
        CloneStage::Receiving {
            received,
            total,
            bytes,
        } => format!(
            "Receiving {received}/{total} objects \u{b7} {}",
            crate::state::file_picker::size_label(Some(*bytes))
        ),
        CloneStage::CheckingOut { done, total } => {
            format!("Checking out {done}/{total} files")
        }
        CloneStage::Registering => "Registering the project\u{2026}".to_string(),
    }
}

/// The sentence a failed clone or listing reads as.
pub fn clone_error_note(error: &CloneError) -> String {
    match error {
        CloneError::Network(detail) => format!("The transfer failed: {detail}"),
        CloneError::Auth => {
            "That credential was refused, or the repository is not public.".to_string()
        }
        CloneError::NotFound => "No repository there, or none this identity can see.".to_string(),
        CloneError::Exists => {
            "There is already a folder at that destination. Nothing was written.".to_string()
        }
        CloneError::Unsupported(detail) => format!("Ubiq cannot clone from that: {detail}"),
        CloneError::Refused(detail) => format!("The host declined: {detail}"),
    }
}
