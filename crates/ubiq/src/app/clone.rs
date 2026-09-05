//! Cloning a project: what the modal's controls do, and what the host says back.
//!
//! Everything outbound here is fire-and-forget on the bus, the house pattern: nothing waits for an
//! answer, and the answer — when it comes — lands in [`AppState::receive_repo`] and is discarded
//! unless it names the id this window is still holding.
//!
//! **There is no clone-success path in this file.** A finished clone is a registered project, so
//! `ProjectAdded` is what closes the modal — see `receive_project` in `wire.rs`.

use std::time::Duration;

use super::*;
use crate::state::clone::{CloneMode, CloneState, check_url};
use ubiq_proto::ids::{CloneId, ConnectionId, RepoQueryId};
use ubiq_proto::repos::{CloneRequest, CloneStage, RemoteRepo};

/// How long a filter with no local answers waits before the provider is asked. Long enough that a
/// typed word is one request rather than five.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);

impl AppState {
    /// Open the clone modal. `prefill` is a URL the user already has — the omni search's clone row
    /// hands one over — which puts the modal straight into its URL half.
    pub fn open_clone(
        &mut self,
        prefill: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = self
            .workbench
            .settings
            .ui
            .last_connection
            .as_deref()
            .and_then(|id| {
                self.workbench
                    .settings
                    .host
                    .connections
                    .iter()
                    .find(|c| c.id.to_string() == id)
                    .map(|c| c.id)
            })
            .or_else(|| {
                self.workbench
                    .settings
                    .host
                    .connections
                    .first()
                    .map(|c| c.id)
            });

        let mut state = CloneState {
            connection,
            mode: match prefill.is_some() {
                true => CloneMode::Url,
                false => CloneMode::Connection,
            },
            url: prefill.clone().unwrap_or_default(),
            ..CloneState::default()
        };
        state.parent = self.clone_root(state.ephemeral);
        state.name = state.default_name();

        self.workbench.open_menu = None;
        self.workbench.clone_project = Some(state);
        // The fields are the window's, so what a previous clone left in them is cleared here
        // rather than carried into this one.
        self.set_clone_fields(window, cx);
        if prefill.is_some() {
            self.ask_clone_branches(cx);
        } else {
            self.ask_clone_repos(None, cx);
        }
        cx.notify();
    }

    /// Close it. A clone still running is cancelled: an abandoned modal must not leave a transfer
    /// writing into a folder nobody is watching.
    pub fn close_clone(&mut self, cx: &mut Context<Self>) {
        if let Some(clone) = self.workbench.clone_project.take()
            && let Some(clone_id) = clone.clone_id
        {
            self.bus.send(Message::CancelClone { clone_id });
        }
        cx.notify();
    }

    /// The folder a clone lands in as the settings stand. Empty when the host has named no root —
    /// the interface never invents a path, and an empty `parent` is what says "your default".
    fn clone_root(&self, ephemeral: bool) -> String {
        let host = &self.workbench.settings.host;
        match ephemeral {
            true => host.ephemeral_root.clone(),
            false => host.projects_root.clone(),
        }
        .unwrap_or_default()
    }

    /// Put the modal's own state into the fields that draw it. `set_value` needs a window, which
    /// is why this is not simply part of assigning the state.
    fn set_clone_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (url, name) = match &self.workbench.clone_project {
            Some(clone) => (clone.url.clone(), clone.name.clone()),
            None => (String::new(), String::new()),
        };
        self.clone_url_input
            .clone()
            .update(cx, |input, cx| input.set_value(&url, window, cx));
        self.clone_name_input
            .clone()
            .update(cx, |input, cx| input.set_value(&name, window, cx));
        self.clone_filter_input
            .clone()
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    // ── The controls ────────────────────────────────────────────────

    /// Pick whose listing to show, and remember it: the next clone opens on the same identity.
    pub fn pick_clone_connection(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        let Some(clone) = self.workbench.clone_project.as_mut() else {
            return;
        };
        clone.connection = Some(connection);
        clone.mode = CloneMode::Connection;
        clone.repos.clear();
        clone.repo = None;
        clone.branches.clear();
        clone.branch = None;
        self.workbench.open_menu = None;
        self.workbench.settings.ui.last_connection = Some(connection.to_string());
        self.remember_settings();
        self.ask_clone_repos(None, cx);
        cx.notify();
    }

    /// The repository filter changed. In-memory first; the provider is only asked when that runs
    /// out over a listing the provider said it truncated, and then only once the typing stops.
    pub fn retype_clone_filter(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(clone) = self.workbench.clone_project.as_mut() else {
            return;
        };
        clone.filter = text.clone();
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                // The field has moved on, so this burst is not the one that ends the typing.
                let wanted = this
                    .workbench
                    .clone_project
                    .as_ref()
                    .is_some_and(|clone| clone.filter == text && clone.wants_server_search());
                if wanted {
                    this.ask_clone_repos(Some(text), cx);
                }
            });
        })
        .detach();
    }

    /// The URL field changed. Nothing is sent — the branch listing waits until the URL is settled
    /// enough to parse, and a half-typed URL parses as nothing.
    pub fn retype_clone_url(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(clone) = self.workbench.clone_project.as_mut() else {
            return;
        };
        let settled = matches!(check_url(&text), Some(Ok(_)));
        let was = clone.url.clone();
        clone.url = text;
        clone.mode = CloneMode::Url;
        clone.repo = None;
        if !settled {
            clone.branches.clear();
            clone.branch = None;
        }
        if settled && was != clone.url {
            let name = clone.default_name();
            self.rename_clone(name, window, cx);
            self.ask_clone_branches(cx);
        }
        cx.notify();
    }

    /// Pick a repository from the listing. The default branch is shown at once and the real list
    /// is asked for behind it.
    pub fn pick_clone_repo(
        &mut self,
        repo: RemoteRepo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clone) = self.workbench.clone_project.as_mut() else {
            return;
        };
        clone.mode = CloneMode::Connection;
        clone.branch = repo.default_branch.clone();
        clone.branches = repo.default_branch.clone().into_iter().collect();
        clone.repo = Some(repo);
        let name = clone.default_name();
        self.rename_clone(name, window, cx);
        self.ask_clone_branches(cx);
        cx.notify();
    }

    /// Open a dropdown whose panel carries the shared filter field, clearing and focusing it —
    /// the gesture `open_agent_bench_menu` performs for the agents screen, said once for any
    /// picker that opts into search.
    pub fn open_picker_menu(&mut self, menu: MenuId, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(menu);
        let search = self.picker_search.clone();
        search.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub fn pick_clone_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        if let Some(clone) = self.workbench.clone_project.as_mut() {
            clone.branch = Some(branch);
        }
        self.workbench.open_menu = None;
        cx.notify();
    }

    /// The name field changed, or a picked repository named it.
    pub fn rename_clone(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(clone) = self.workbench.clone_project.as_mut() {
            clone.name = name.clone();
        }
        let input = self.clone_name_input.clone();
        if input.read(cx).value().as_ref() != name.as_str() {
            input.update(cx, |input, cx| input.set_value(&name, window, cx));
        }
        cx.notify();
    }

    /// Throwaway or not. Ticking it moves the destination to the ephemeral root and turns shallow
    /// on — both defaults, and both still the user's to change afterwards.
    pub fn toggle_clone_ephemeral(&mut self, cx: &mut Context<Self>) {
        let Some(ephemeral) = self
            .workbench
            .clone_project
            .as_ref()
            .map(|clone| !clone.ephemeral)
        else {
            return;
        };
        let parent = self.clone_root(ephemeral);
        if let Some(clone) = self.workbench.clone_project.as_mut() {
            clone.ephemeral = ephemeral;
            clone.shallow = ephemeral || clone.shallow;
            clone.parent = parent;
        }
        cx.notify();
    }

    pub fn toggle_clone_shallow(&mut self, cx: &mut Context<Self>) {
        if let Some(clone) = self.workbench.clone_project.as_mut() {
            clone.shallow = !clone.shallow;
        }
        cx.notify();
    }

    /// Ask the operating system where to put it. The same three outcomes `choose_folder` handles:
    /// the channel can close with the dialog, the platform can refuse to open one, and the user
    /// can cancel.
    pub fn choose_clone_folder(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Clone into".into()),
        });

        cx.spawn(async move |this, cx| {
            let answer = match chosen.await {
                Ok(answer) => answer,
                Err(_) => return,
            };

            this.update(cx, |this, cx| match answer {
                Ok(Some(paths)) => {
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    if let Some(clone) = this.workbench.clone_project.as_mut() {
                        clone.parent = path.to_string_lossy().into_owned();
                    }
                    cx.notify();
                }
                Ok(None) => {}
                Err(error) => {
                    this.workbench.project_error =
                        Some(format!("could not open a chooser: {error}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ── Asking the host ─────────────────────────────────────────────

    /// Ask for a listing. `query` is the provider's own search text; `None` is what an empty field
    /// shows.
    fn ask_clone_repos(&mut self, query: Option<String>, cx: &mut Context<Self>) {
        let Some(clone) = self.workbench.clone_project.as_mut() else {
            return;
        };
        let Some(connection) = clone.connection else {
            return;
        };
        let query_id = RepoQueryId::generate();
        clone.repos_query = Some(query_id);
        self.bus.send(Message::ListRepos {
            query_id,
            connection,
            query,
        });
        cx.notify();
    }

    /// Ask for one repository's branches, for whichever source the modal is on.
    fn ask_clone_branches(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self
            .workbench
            .clone_project
            .as_ref()
            .and_then(CloneState::source)
        else {
            return;
        };
        let query_id = RepoQueryId::generate();
        if let Some(clone) = self.workbench.clone_project.as_mut() {
            clone.branches_query = Some(query_id);
        }
        self.bus
            .send(Message::ListRepoBranches { query_id, source });
        cx.notify();
    }

    /// Start it. The modal stays up and turns into a progress report; it closes on `ProjectAdded`.
    pub fn start_clone(&mut self, cx: &mut Context<Self>) {
        let Some(clone) = self.workbench.clone_project.as_ref() else {
            return;
        };
        let (Some(source), false) = (clone.source(), clone.name.trim().is_empty()) else {
            return;
        };
        let clone_id = CloneId::generate();
        let request = CloneRequest {
            clone_id,
            source,
            branch: clone.branch.clone(),
            shallow: clone.shallow,
            parent: clone.parent.clone(),
            name: clone.name.trim().to_string(),
            ephemeral: clone.ephemeral,
        };
        if let Some(clone) = self.workbench.clone_project.as_mut() {
            clone.clone_id = Some(clone_id);
            clone.stage = Some(CloneStage::Resolving);
            clone.error = None;
        }
        self.bus.send(Message::CloneRepo { request });
        cx.notify();
    }

    /// Stop it, leaving the modal up so the user can change something and try again. The partial
    /// destination is the host's to clean up.
    pub fn cancel_clone(&mut self, cx: &mut Context<Self>) {
        let Some(clone) = self.workbench.clone_project.as_mut() else {
            return;
        };
        let Some(clone_id) = clone.clone_id.take() else {
            return;
        };
        clone.stage = None;
        self.bus.send(Message::CancelClone { clone_id });
        cx.notify();
    }

    // ── What the host says ──────────────────────────────────────────

    /// The repository family.
    ///
    /// Every answer names an id, and one this window no longer holds is dropped rather than drawn:
    /// a listing that arrives after the connection was changed is an answer to a question nobody
    /// is asking any more.
    ///
    /// Answers with the message when it belongs to another family.
    pub(super) fn receive_repo(
        &mut self,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::Repos {
                query_id,
                repos,
                truncated,
            } => {
                if let Some(clone) = self.workbench.clone_project.as_mut()
                    && clone.accept_repos(query_id, repos, truncated)
                {
                    cx.notify();
                }
            }

            Message::RepoBranches {
                query_id,
                branches,
                default,
            } => {
                if let Some(clone) = self.workbench.clone_project.as_mut()
                    && clone.accept_branches(query_id, branches, default)
                {
                    cx.notify();
                }
            }

            Message::RepoError { query_id, error } => {
                if let Some(clone) = self.workbench.clone_project.as_mut()
                    && clone.accept_error(query_id, error)
                {
                    cx.notify();
                }
            }

            Message::ClonePending { clone_id, stage } => {
                if let Some(clone) = self.workbench.clone_project.as_mut()
                    && clone.clone_id == Some(clone_id)
                {
                    clone.stage = Some(stage);
                    cx.notify();
                }
            }

            Message::CloneFailed { clone_id, error } => {
                if let Some(clone) = self.workbench.clone_project.as_mut()
                    && clone.clone_id == Some(clone_id)
                {
                    clone.clone_id = None;
                    clone.stage = None;
                    clone.error = Some(error);
                    cx.notify();
                }
            }

            other => return Some(other),
        }
        None
    }

    /// Whether a clone this window started is still running — what tells `ProjectAdded` that the
    /// project arriving is the one the modal was waiting for.
    pub(super) fn clone_in_flight(&self) -> bool {
        self.workbench
            .clone_project
            .as_ref()
            .is_some_and(|clone| clone.clone_id.is_some())
    }

    /// Whether a project is a throwaway this window may delete rather than merely forget.
    ///
    /// Read from the snapshot, never worked out here. The host applies the test it deletes by, so
    /// what this window warns about and what `Forget` actually removes cannot come apart — and the
    /// ephemeral root's unset value is a default only the host knows.
    pub fn project_is_ephemeral(&self, project: ProjectId, cx: &App) -> bool {
        WindowRegistry::read(cx)
            .project(project)
            .is_some_and(|entry| entry.ephemeral)
    }
}
