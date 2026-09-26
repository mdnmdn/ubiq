//! What the task-sync surfaces do: ask, edit a draft binding, save it, import, force a pass.
//!
//! **Every line of this module goes through [`ubiq_proto::messages::Message`].** The interface has
//! no handle on the host's provider registry, no reference to a `TaskProvider` and no way to run a
//! query itself — a Test is a message out and a count back, and that is what makes the same
//! renderer serve a provider compiled into a second edition's binary.
//!
//! **Nothing here holds credential material.** A `Secret` field's typed value is carried on
//! `SetTaskSource` and filed by the host in the connector family's secret store; nothing reads one
//! back, and the interface keeps no copy past the save.
//!
//! The stale-answer discipline is the repository family's: the interface mints the query id, every
//! reply carries it back, and a reply naming an id no longer in
//! [`crate::state::tasksrc::TaskSrcState::asking`] is discarded rather than drawn. A filter is
//! edited by typing, so that case is the ordinary one rather than the exception.

use std::collections::HashMap;

use gpui::{AppContext, Context, Window};
use gpui_component::input::{InputEvent, InputState};

use ubiq_proto::ids::{ProjectId, TaskId, TaskSrcQueryId};
use ubiq_proto::messages::Message;
use ubiq_proto::tasksrc::{
    Authority, Binding, ConfigFieldKind, Direction, DriftSide, Facets, POLL_FLOOR, ProviderInfo,
    RemoteContainer, RemoteContainerId, RemoteItem, RemoteItemId, RemoteLane, SyncScope, SyncState,
    TaskLink,
};
use ubiq_proto::work::{Kind, Priority, Status};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::tasksrc::{Ask, ImportDialog, TaskSrcMenu, TestResult};

impl AppState {
    // ── Asking ──────────────────────────────────────────────────────

    /// Ask what this build can sync against. Run when the settings section arrives on screen, so a
    /// window that never opens it never asks.
    pub fn ask_task_providers(&mut self, cx: &mut Context<Self>) {
        if !self.tasksrc.providers.is_empty() {
            return;
        }
        let query_id = TaskSrcQueryId::generate();
        self.tasksrc.asking.insert(query_id, Ask::Providers);
        self.bus.send(Message::ListTaskProviders { query_id });
        cx.notify();
    }

    /// Ask what this project is bound to. Run when a project swings into the window.
    pub fn ask_task_source(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        if self.tasksrc.project == Some(project) {
            return;
        }
        self.tasksrc.leave_project();
        self.tasksrc.project = Some(project);
        self.bus.send(Message::GetTaskSource {
            project_id: project,
        });
        cx.notify();
    }

    /// Ask again for the draft's containers — what a change of connection has to run, because the
    /// boards one identity can reach are not the boards another can.
    ///
    /// Public because a contributed section owns its own picker chain and must be able to re-ask
    /// without reaching for the provider list: before `D189` the only way in was to pick the
    /// provider a second time, which is three public calls where one belongs.
    pub fn reask_task_containers(&mut self, cx: &mut Context<Self>) {
        self.ask_containers(cx);
    }

    fn ask_containers(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.as_ref() else {
            return;
        };
        let query_id = TaskSrcQueryId::generate();
        self.tasksrc.asking.insert(query_id, Ask::Containers);
        self.bus.send(Message::ListRemoteContainers {
            query_id,
            provider: draft.provider.clone(),
            connection: draft.connection,
        });
        cx.notify();
    }

    /// Ask for the bound container's lanes and every facet the declared fields draw from — one
    /// ask, because the provider answers both off one container description.
    fn ask_lanes(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.clone() else {
            return;
        };
        if draft.container.as_str().is_empty() {
            return;
        }
        let query_id = TaskSrcQueryId::generate();
        self.tasksrc.asking.insert(query_id, Ask::Lanes);
        self.bus.send(Message::ListRemoteLanes {
            query_id,
            binding: Box::new(draft),
        });
        cx.notify();
    }

    /// **Run the filter.** `R7`: it fetches under the binding's own filter and reports what came
    /// back. Nothing here parses anything, and nothing here validates a field's shape — a Test
    /// that checked a query's syntax would prove less than this does and would be a grammar
    /// nobody owns.
    pub fn test_task_source(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.clone() else {
            return;
        };
        let query_id = TaskSrcQueryId::generate();
        self.tasksrc.asking.insert(query_id, Ask::Test);
        self.tasksrc.testing = Some(query_id);
        // The last answer goes as soon as a new ask leaves: a stale count beside a filter that has
        // moved on is worse than no count.
        self.tasksrc.test = None;
        self.bus.send(Message::TestTaskSource {
            query_id,
            binding: Box::new(draft),
        });
        cx.notify();
    }

    // ── Editing the draft ───────────────────────────────────────────

    /// Pick the provider, by index into the offered list. `None` unbinds the draft.
    pub fn pick_task_provider(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(index) = index else {
            self.tasksrc.draft = None;
            self.tasksrc.containers.clear();
            self.tasksrc.lanes.clear();
            self.tasksrc.facets = Facets::default();
            cx.notify();
            return;
        };
        let Some(info) = self.tasksrc.providers.get(index) else {
            return;
        };
        let provider = info.id.to_string();
        // A connection has to come from somewhere, and since `D189` it comes from the connections
        // this build actually holds: the provider declares its connector family and the picker
        // below offers that family's connections. The draft starts on whichever of them the
        // binding already named, or on the only candidate when there is exactly one — a single
        // choice is not a decision worth asking for — and otherwise on a placeholder the picker
        // replaces. **The interface never holds a token**, only the reference.
        let connection = {
            let offered = crate::state::tasksrc::connections_for(
                info,
                &self.workbench.settings.host.connections,
            );
            let held = self.tasksrc.draft.as_ref().map(|draft| draft.connection);
            held.filter(|id| offered.iter().any(|one| one.id == *id))
                .or_else(|| match offered.as_slice() {
                    [only] => Some(only.id),
                    _ => None,
                })
                .unwrap_or_else(ubiq_proto::ids::ConnectionId::generate)
        };
        let mut draft = Binding::new(provider, connection, RemoteContainerId::new(String::new()));
        if let Some(old) = self.tasksrc.draft.as_ref() {
            // Everything that is not provider-shaped survives a change of provider: the interval,
            // the direction and the switch are the user's answers about *this project*, not about
            // the tracker.
            draft.poll = old.poll;
            draft.direction = old.direction;
            draft.authority = old.authority;
            draft.auto_import = old.auto_import;
        }
        self.tasksrc.draft = Some(draft);
        self.tasksrc.containers.clear();
        self.tasksrc.lanes.clear();
        self.tasksrc.facets = Facets::default();
        self.tasksrc.test = None;
        self.ask_containers(cx);
        cx.notify();
    }

    /// Bind the draft to one of the connections this build holds, by index into what
    /// [`crate::state::tasksrc::connections_for`] offered for the draft's provider (`D189`).
    ///
    /// **This is what makes a binding bindable to a real identity.** Everything below a connection
    /// is scoped to it — the boards one account can reach are not another's — so the containers,
    /// the lanes and the facets are dropped and re-asked rather than carried across. The
    /// credential itself never comes near: a [`ConnectionId`](ubiq_proto::ids::ConnectionId) is a
    /// reference the host resolves against the connector family's secret store.
    pub fn pick_task_connection(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(info) = self.tasksrc.provider() else {
            return;
        };
        let chosen =
            crate::state::tasksrc::connections_for(info, &self.workbench.settings.host.connections)
                .get(index)
                .map(|one| one.id);
        let Some(chosen) = chosen else {
            return;
        };
        let Some(draft) = self.tasksrc.draft.as_mut() else {
            return;
        };
        if draft.connection == chosen {
            return;
        }
        draft.connection = chosen;
        // Every id below the connection belongs to the identity that answered for it.
        draft.container = RemoteContainerId::new(String::new());
        draft.lanes.clear();
        draft.kinds.clear();
        draft.priorities.clear();
        draft.filter = ubiq_proto::tasksrc::Filter::default();
        self.tasksrc.containers.clear();
        self.tasksrc.lanes.clear();
        self.tasksrc.facets = Facets::default();
        self.tasksrc.test = None;
        self.ask_containers(cx);
        cx.notify();
    }

    pub fn pick_task_container(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(container) = self.tasksrc.containers.get(index).map(|c| c.id.clone()) else {
            return;
        };
        let Some(draft) = self.tasksrc.draft.as_mut() else {
            return;
        };
        if draft.container == container {
            return;
        }
        draft.container = container;
        // Every id in the binding is scoped to the container, so a change of board empties the
        // maps rather than carrying ids that mean nothing here.
        draft.lanes.clear();
        draft.kinds.clear();
        draft.priorities.clear();
        draft.filter = ubiq_proto::tasksrc::Filter::default();
        self.tasksrc.lanes.clear();
        self.tasksrc.facets = Facets::default();
        self.tasksrc.test = None;
        self.ask_lanes(cx);
        cx.notify();
    }

    /// Set one declared field's whole value list. The one way a `Text`, `Secret` or `Choice`
    /// field is written, so nothing needs to know which of the three it was.
    pub fn set_task_filter(&mut self, key: String, values: Vec<String>, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.as_mut() else {
            return;
        };
        if values.is_empty() {
            draft.filter.fields.remove(&key);
        } else {
            draft.filter.set(key, values);
        }
        self.tasksrc.test = None;
        cx.notify();
    }

    pub fn set_task_filter_flag(&mut self, key: String, on: bool, cx: &mut Context<Self>) {
        self.set_task_filter(key, vec![on.to_string()], cx);
    }

    /// Add or remove one value of a `MultiChoice` field. An empty set is **no constraint**, never
    /// "match nothing", which is why removing the last value clears the key rather than storing an
    /// empty list.
    pub fn toggle_task_filter_value(&mut self, key: String, id: String, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.as_mut() else {
            return;
        };
        let held = draft.filter.fields.entry(key.clone()).or_default();
        match held.iter().position(|value| value == &id) {
            Some(ix) => {
                held.remove(ix);
            }
            None => held.push(id),
        }
        if held.is_empty() {
            draft.filter.fields.remove(&key);
        }
        self.tasksrc.test = None;
        cx.notify();
    }

    /// Map one remote lane onto a column, or unmap it. Unmapped **parks** — the tasks in it stay
    /// where they are rather than moving somewhere nobody chose (`R9`).
    pub fn map_task_lane(&mut self, lane: String, status: Option<Status>, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.as_mut() else {
            return;
        };
        match status {
            Some(status) => {
                draft.lanes.insert(lane, status);
            }
            None => {
                draft.lanes.remove(&lane);
            }
        }
        cx.notify();
    }

    /// Map one of the provider's own words onto a [`Kind`] or a [`Priority`], or drop the mapping.
    ///
    /// The value arrives as the label the picker drew, which is the only string both halves agree
    /// on for a closed set — parsing it back is a lookup over `all()`, not a match written twice.
    pub fn map_task_word(
        &mut self,
        word: String,
        value: Option<String>,
        priority: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.tasksrc.draft.as_mut() else {
            return;
        };
        match (priority, value) {
            (true, Some(label)) => {
                if let Some(found) = Priority::all()
                    .into_iter()
                    .find(|p| p.label().unwrap_or("normal") == label)
                {
                    draft.priorities.insert(word, found);
                }
            }
            (true, None) => {
                draft.priorities.remove(&word);
            }
            (false, Some(label)) => {
                if let Some(found) = Kind::all().into_iter().find(|k| k.label() == label) {
                    draft.kinds.insert(word, found);
                }
            }
            (false, None) => {
                draft.kinds.remove(&word);
            }
        }
        cx.notify();
    }

    pub fn set_task_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if let Some(draft) = self.tasksrc.draft.as_mut() {
            draft.direction = direction;
            cx.notify();
        }
    }

    pub fn set_task_authority(&mut self, authority: Authority, cx: &mut Context<Self>) {
        if let Some(draft) = self.tasksrc.draft.as_mut() {
            draft.authority = authority;
            cx.notify();
        }
    }

    pub fn set_task_auto_import(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(draft) = self.tasksrc.draft.as_mut() {
            draft.auto_import = on;
            cx.notify();
        }
    }

    /// Nudge the interval. The **asked-for** value is what is stored, never the floored one: the
    /// floor is this build's rule, and a later build with a lower one must not find the number
    /// rewritten.
    pub fn nudge_task_poll(&mut self, delta: i64, cx: &mut Context<Self>) {
        if let Some(draft) = self.tasksrc.draft.as_mut() {
            let next = (draft.poll as i64 + delta).max(POLL_FLOOR as i64);
            draft.poll = next as u64;
            cx.notify();
        }
    }

    // ── Saving ──────────────────────────────────────────────────────

    /// Send the draft. The typed `Text` and `Secret` values are gathered here rather than on every
    /// keystroke, so a half-typed query never reaches the file.
    pub fn save_task_source(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        self.gather_tasksrc_inputs(cx);
        let Some(draft) = self.tasksrc.draft.clone() else {
            return;
        };
        self.bus.send(Message::SetTaskSource {
            project_id,
            binding: Some(Box::new(draft)),
        });
        cx.notify();
    }

    /// Unbind. **Nothing is deleted**: the binding and the link rows go, every task stays exactly
    /// as it is, with its content and its history (`R9`).
    pub fn unbind_task_source(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        self.bus.send(Message::SetTaskSource {
            project_id,
            binding: None,
        });
        cx.notify();
    }

    /// Run a pass now, over the whole board.
    pub fn sync_task_source_now(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        self.bus.send(Message::SyncTaskSource {
            project_id,
            scope: SyncScope::All,
        });
        cx.notify();
    }

    /// Run a pass over one task — the card's own *Sync now*.
    pub fn sync_task_now(&mut self, task: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        self.bus.send(Message::SyncTaskSource {
            project_id,
            scope: SyncScope::Task(task),
        });
        cx.notify();
    }

    // ── Drift ───────────────────────────────────────────────────────

    /// Settle one drifted task, one way or the other (`D188`).
    ///
    /// **This overrides the binding's authority switch on purpose.** The switch is what happens
    /// when nobody is looking; this is somebody looking at the two values and choosing, which is
    /// the whole reason the overview draws them side by side.
    pub fn resolve_task_drift(&mut self, task_id: TaskId, side: DriftSide, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        self.bus.send(Message::ResolveTaskDrift {
            project_id,
            task_id,
            side,
        });
        cx.notify();
    }

    /// The same, for every drifted task at once — the overview's **Push all** and **Pull all**.
    ///
    /// One message per task rather than a bulk variant: each is a separate round trip at the
    /// provider anyway, one of them failing must not take the rest with it, and the host's arm is
    /// then the same arm a single row presses.
    pub fn resolve_all_drift(&mut self, side: DriftSide, cx: &mut Context<Self>) {
        for task_id in self.tasksrc.drifted.clone() {
            self.resolve_task_drift(task_id, side, cx);
        }
    }

    /// Drop one task's link row.
    pub fn unlink_task(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        self.bus.send(Message::UnlinkTask {
            project_id,
            task_id,
        });
        cx.notify();
    }

    // ── The import dialog ───────────────────────────────────────────

    pub fn open_task_import(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.tasksrc.draft.clone() else {
            return;
        };
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        let query_id = TaskSrcQueryId::generate();
        self.tasksrc.asking.insert(query_id, Ask::Items);
        self.tasksrc.import = Some(ImportDialog {
            query_id: Some(query_id),
            loading: true,
            ..ImportDialog::default()
        });
        self.bus.send(Message::ListRemoteItems {
            query_id,
            project_id,
            binding: Box::new(draft),
        });
        cx.notify();
    }

    pub fn close_task_import(&mut self, cx: &mut Context<Self>) {
        self.tasksrc.import = None;
        cx.notify();
    }

    pub fn toggle_import_item(&mut self, id: RemoteItemId, cx: &mut Context<Self>) {
        let Some(dialog) = self.tasksrc.import.as_mut() else {
            return;
        };
        if dialog.is_linked(&id) {
            return;
        }
        match dialog.picked.iter().position(|held| held == &id) {
            Some(ix) => {
                dialog.picked.remove(ix);
            }
            None => dialog.picked.push(id),
        }
        cx.notify();
    }

    /// Create a task per ticked item. Nothing that was not ticked is created (`R12`).
    pub fn import_picked_items(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.tasksrc.project else {
            return;
        };
        let Some(dialog) = self.tasksrc.import.as_ref() else {
            return;
        };
        if dialog.picked.is_empty() {
            return;
        }
        self.bus.send(Message::ImportRemoteItems {
            project_id,
            items: dialog.picked.clone(),
        });
        self.tasksrc.import = None;
        cx.notify();
    }

    // ── Menus ───────────────────────────────────────────────────────

    /// Open one of the section's dropdowns. The window's single-open-menu rule applies as it does
    /// everywhere else; the discriminant says which.
    pub fn open_tasksrc_menu(&mut self, menu: TaskSrcMenu, cx: &mut Context<Self>) {
        if self.tasksrc.menu.as_ref() == Some(&menu) {
            self.tasksrc.menu = None;
            self.close_menu(cx);
            return;
        }
        self.tasksrc.menu = Some(menu);
        self.open_menu(MenuId::TaskSrc, cx);
    }

    // ── The text buffers ────────────────────────────────────────────

    /// Keep one buffer per declared `Text` or `Secret` field, seeded from the draft the moment a
    /// schema arrives with none.
    ///
    /// Called once a frame from `Render`, on `ensure_kb_inputs`' discipline and for its reason —
    /// **no `cx.notify()`**, because this runs inside the render it would otherwise spin. A
    /// buffer cannot be built anywhere else: `InputState::new` needs a `&mut Window`, and a
    /// section's `render` has only a `&Window`.
    pub(super) fn ensure_tasksrc_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.tasksrc.provider() else {
            self.tasksrc.inputs.clear();
            self.tasksrc_subs.clear();
            return;
        };
        let wanted: Vec<(String, bool)> = info
            .config_schema
            .iter()
            .filter_map(|spec| match spec.kind {
                ConfigFieldKind::Text => Some((spec.key.to_string(), false)),
                ConfigFieldKind::Secret => Some((spec.key.to_string(), true)),
                _ => None,
            })
            .collect();

        self.tasksrc
            .inputs
            .retain(|key, _| wanted.iter().any(|(want, _)| want == key));
        self.tasksrc_subs
            .retain(|key, _| wanted.iter().any(|(want, _)| want == key));

        for (key, secret) in wanted {
            if self.tasksrc.inputs.contains_key(&key) {
                continue;
            }
            // A secret is **never** seeded from the binding, because the binding does not hold
            // one: the material went to the secret store and what is filed is a reference. An
            // empty box is the honest drawing of a value the interface does not have.
            let seed = if secret {
                String::new()
            } else {
                self.tasksrc
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.filter.one(&key).map(str::to_string))
                    .unwrap_or_default()
            };
            let placeholder = if secret {
                "\u{2022}\u{2022}\u{2022}"
            } else {
                ""
            };
            let field = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(seed)
            });
            let commit_key = key.clone();
            let subscription = cx.subscribe_in(
                &field,
                window,
                move |this, input, event: &InputEvent, _window, cx| {
                    // Commit on Enter or blur, the same bargain every other free-text field in
                    // this window takes: a query typed and clicked away must stick.
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let value = input.read(cx).value().to_string();
                        this.commit_tasksrc_field(commit_key.clone(), value, cx);
                    }
                },
            );
            self.tasksrc.inputs.insert(key.clone(), field);
            self.tasksrc_subs.insert(key, subscription);
        }
    }

    fn commit_tasksrc_field(&mut self, key: String, value: String, cx: &mut Context<Self>) {
        let values = if value.trim().is_empty() {
            Vec::new()
        } else {
            vec![value]
        };
        self.set_task_filter(key, values, cx);
    }

    /// Read every buffer into the draft, for the save that is about to go out.
    fn gather_tasksrc_inputs(&mut self, cx: &mut Context<Self>) {
        let typed: Vec<(String, String)> = self
            .tasksrc
            .inputs
            .iter()
            .map(|(key, input)| (key.clone(), input.read(cx).value().to_string()))
            .collect();
        for (key, value) in typed {
            self.commit_tasksrc_field(key, value, cx);
        }
    }

    // ── What the host says back ─────────────────────────────────────

    /// One place for every reply in the family, so the stale-answer rule is written once.
    ///
    /// A reply whose id is not in `asking` is **discarded**, never drawn: the user has typed past
    /// the question it answers.
    pub(super) fn apply_task_providers(
        &mut self,
        query_id: TaskSrcQueryId,
        providers: Vec<ProviderInfo>,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.asking.remove(&query_id) != Some(Ask::Providers) {
            return;
        }
        self.tasksrc.providers = providers;
        cx.notify();
    }

    pub(super) fn apply_remote_containers(
        &mut self,
        query_id: TaskSrcQueryId,
        containers: Vec<RemoteContainer>,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.asking.remove(&query_id) != Some(Ask::Containers) {
            return;
        }
        self.tasksrc.containers = containers;
        cx.notify();
    }

    pub(super) fn apply_remote_lanes(
        &mut self,
        query_id: TaskSrcQueryId,
        lanes: Vec<RemoteLane>,
        facets: Facets,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.asking.remove(&query_id) != Some(Ask::Lanes) {
            return;
        }
        self.tasksrc.lanes = lanes;
        self.tasksrc.facets = facets;
        cx.notify();
    }

    pub(super) fn apply_task_source_test(
        &mut self,
        query_id: TaskSrcQueryId,
        count: usize,
        sample: Vec<RemoteItem>,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.asking.remove(&query_id) != Some(Ask::Test) {
            return;
        }
        self.tasksrc.testing = None;
        self.tasksrc.test = Some(TestResult {
            query_id,
            count,
            sample: sample.into_iter().map(|item| item.title).collect(),
            at: chrono::Utc::now(),
        });
        cx.notify();
    }

    pub(super) fn apply_task_source(
        &mut self,
        project_id: ProjectId,
        binding: Option<Binding>,
        state: SyncState,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.project != Some(project_id) {
            return;
        }
        self.tasksrc.saved = binding.clone();
        self.tasksrc.draft = binding;
        self.tasksrc.state = state;
        self.tasksrc.error = None;
        // A binding that just arrived has a board but no lanes and no facets yet, and every picker
        // under the filter is empty until they land.
        self.ask_containers(cx);
        self.ask_lanes(cx);
        cx.notify();
    }

    pub(super) fn apply_remote_items(
        &mut self,
        query_id: TaskSrcQueryId,
        items: Vec<RemoteItem>,
        linked: Vec<RemoteItemId>,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.asking.remove(&query_id) != Some(Ask::Items) {
            return;
        }
        if let Some(dialog) = self.tasksrc.import.as_mut()
            && dialog.query_id == Some(query_id)
        {
            dialog.loading = false;
            dialog.items = items;
            dialog.linked = linked;
        }
        cx.notify();
    }

    pub(super) fn apply_task_source_state(
        &mut self,
        project_id: ProjectId,
        state: SyncState,
        last_sync: Option<chrono::DateTime<chrono::Utc>>,
        drifted: Vec<TaskId>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.project != Some(project_id) {
            return;
        }
        self.tasksrc.state = state;
        self.tasksrc.last_sync = last_sync;
        self.tasksrc.drifted = drifted;
        self.tasksrc.error = error;
        cx.notify();
    }

    pub(super) fn apply_task_link(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        link: Option<TaskLink>,
        cx: &mut Context<Self>,
    ) {
        if self.tasksrc.project != Some(project_id) {
            return;
        }
        match link {
            Some(link) => {
                self.tasksrc.links.insert(task_id, link);
            }
            None => {
                self.tasksrc.links.remove(&task_id);
            }
        }
        cx.notify();
    }

    /// The task-source family.
    ///
    /// Nothing in it names a pane, so it routes by variant alone. Answers with the message when it
    /// belongs to another family, the chain's own discipline.
    pub(super) fn receive_tasksrc(
        &mut self,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::TaskProviders {
                query_id,
                providers,
            } => self.apply_task_providers(query_id, providers, cx),
            Message::RemoteContainers {
                query_id,
                containers,
            } => self.apply_remote_containers(query_id, containers, cx),
            Message::RemoteLanes {
                query_id,
                lanes,
                facets,
            } => self.apply_remote_lanes(query_id, lanes, facets, cx),
            Message::TaskSourceTest {
                query_id,
                count,
                sample,
            } => self.apply_task_source_test(query_id, count, sample, cx),
            Message::TaskSource {
                project_id,
                binding,
                state,
            } => self.apply_task_source(project_id, binding.map(|b| *b), state, cx),
            Message::RemoteItems {
                query_id,
                items,
                linked,
            } => self.apply_remote_items(query_id, items, linked, cx),
            Message::TaskSourceState {
                project_id,
                state,
                last_sync,
                drifted,
                error,
            } => self.apply_task_source_state(project_id, state, last_sync, drifted, error, cx),
            Message::TaskLinkChanged {
                project_id,
                task_id,
                link,
            } => self.apply_task_link(project_id, task_id, link.map(|l| *l), cx),
            Message::TaskSourceError { query_id, error } => {
                self.apply_task_source_error(query_id, error, cx)
            }
            other => return Some(other),
        }
        None
    }

    pub(super) fn apply_task_source_error(
        &mut self,
        query_id: TaskSrcQueryId,
        error: String,
        cx: &mut Context<Self>,
    ) {
        let ask = self.tasksrc.asking.remove(&query_id);
        if ask.is_none() {
            return;
        }
        if ask == Some(Ask::Test) {
            self.tasksrc.testing = None;
        }
        if ask == Some(Ask::Items)
            && let Some(dialog) = self.tasksrc.import.as_mut()
        {
            dialog.loading = false;
            dialog.error = Some(error.clone());
            cx.notify();
            return;
        }
        self.tasksrc.error = Some(error);
        cx.notify();
    }
}

/// The subscriptions that commit each text buffer, held beside the buffers so the two go and come
/// back together. Keyed by the field's own `key`, exactly as the buffers are.
pub type TaskSrcSubs = HashMap<String, gpui::Subscription>;
