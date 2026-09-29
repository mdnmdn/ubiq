//! The skills and MCP servers catalog's mutators: what each gesture on the two settings pages, the
//! repository browser, the MCP form and the registry search sends, and what the host's answers do
//! to the forms they were raised from.
//!
//! The layers themselves are [`crate::state::catalog::CatalogState`]; nothing here keeps a copy.
//! **Every write is a message and the answer is the list**: a page redraws from the `Catalog` the
//! host broadcasts after the change, and a form that was waiting on it closes when that arrives.
//! A refusal (`CatalogError`) leaves the form up with the host's sentence on it.

use super::*;
use crate::state::catalog::{
    CatalogLayer, McpFields, McpForm, RegistryBrowse, SkillBrowse, compose_mcp, mcp_fields,
    mcp_ready, registry_id, source_id,
};
use crate::state::file_picker::{
    Commit, PickKind, PickerCount, PickerOwner, PickerRequest, PickerView,
};
use ubiq_proto::catalog::{
    CatalogMcp, McpKind, RegistryMcpInfo, RemoteSkillInfo, SkillAdd, SkillInfo, SkillSourceInfo,
    SkillSourceKindInfo,
};

/// The typed fields the catalog's modals read on Save or Search. Held together so the window
/// carries one field rather than a dozen, and built once at boot.
pub struct CatalogInputs {
    /// The repository browser's query.
    pub skill_search: Entity<InputState>,
    /// The "Add source" question's two boxes.
    pub source_url: Entity<InputState>,
    pub source_subpath: Entity<InputState>,
    /// The MCP form's boxes.
    pub mcp_id: Entity<InputState>,
    pub mcp_command: Entity<InputState>,
    pub mcp_args: Entity<TextareaState>,
    pub mcp_env: Entity<TextareaState>,
    pub mcp_url: Entity<InputState>,
    pub mcp_headers: Entity<TextareaState>,
    pub mcp_description: Entity<InputState>,
    /// The paste box: another harness's MCP configuration, in whatever shape it wrote it.
    pub mcp_paste: Entity<TextareaState>,
    /// The registry search's query.
    pub registry_search: Entity<InputState>,
}

impl CatalogInputs {
    pub fn new(window: &mut Window, cx: &mut Context<AppState>) -> Self {
        let line = |window: &mut Window, cx: &mut Context<AppState>, hint: &'static str| {
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        };
        let area = |window: &mut Window, cx: &mut Context<AppState>, hint: &'static str| {
            cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder(hint)
                    .auto_grow(2, 5)
            })
        };
        Self {
            skill_search: line(window, cx, "Search skills\u{2026}"),
            source_url: line(window, cx, "https://github.com/org/skills.git"),
            source_subpath: line(window, cx, "Only under this path (optional)"),
            mcp_id: line(window, cx, "filesystem, docs\u{2026}"),
            mcp_command: line(window, cx, "npx"),
            mcp_args: area(window, cx, "-y\n@modelcontextprotocol/server-filesystem"),
            mcp_env: area(window, cx, "API_KEY=\u{2026}"),
            mcp_url: line(window, cx, "https://example.com/mcp"),
            mcp_headers: area(window, cx, "Authorization: Bearer \u{2026}"),
            mcp_description: line(window, cx, "What it is for (optional)"),
            mcp_paste: area(window, cx, "{ \"mcpServers\": { \u{2026} } }"),
            registry_search: line(window, cx, "Search the MCP registry\u{2026}"),
        }
    }

    /// The fields' focus handles, so a focus change redraws the underline the parent draws.
    pub fn handles(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        vec![
            self.skill_search.read(cx).focus_handle(cx),
            self.source_url.read(cx).focus_handle(cx),
            self.source_subpath.read(cx).focus_handle(cx),
            self.mcp_id.read(cx).focus_handle(cx),
            self.mcp_command.read(cx).focus_handle(cx),
            self.mcp_args.read(cx).focus_handle(cx),
            self.mcp_env.read(cx).focus_handle(cx),
            self.mcp_url.read(cx).focus_handle(cx),
            self.mcp_headers.read(cx).focus_handle(cx),
            self.mcp_description.read(cx).focus_handle(cx),
            self.mcp_paste.read(cx).focus_handle(cx),
            self.registry_search.read(cx).focus_handle(cx),
        ]
    }
}

fn clear(input: &Entity<InputState>, window: &mut Window, cx: &mut Context<AppState>) {
    input.update(cx, |state, cx| state.set_value("", window, cx));
}

fn set(input: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut Context<AppState>) {
    input.update(cx, |state, cx| state.set_value(text, window, cx));
}

fn set_area(
    input: &Entity<TextareaState>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<AppState>,
) {
    input.update(cx, |state, cx| state.set_value(text, window, cx));
}

impl AppState {
    // ── Asking ──────────────────────────────────────────────────────

    /// Ask for one layer. The answer is a `Catalog` for that scope, to everyone.
    pub fn ask_catalog(&mut self, scope: Option<ProjectId>) {
        self.bus.send(Message::ListCatalog { scope });
    }

    /// The application's Skills and MCP servers sections ask on arrival, for the reason the
    /// connections are: a change made in another window has to show up here.
    pub(crate) fn on_show_catalog(&mut self, _cx: &mut Context<Self>) {
        self.ask_catalog(None);
    }

    /// The project dialog's twins ask for their own layer, and the application's beside it: the
    /// inherited group draws from it.
    pub(crate) fn on_show_project_catalog(&mut self, _cx: &mut Context<Self>) {
        self.ask_catalog(None);
        if let Some(project) = self.editing_project() {
            self.ask_catalog(Some(project));
        }
    }

    /// The layer a New agent modal or a definition form draws its skills and catalog MCP servers
    /// from, besides the application's: the project the form is written for, else the one the
    /// window is on.
    pub fn form_catalog_scope(&self, cx: &App) -> Option<ProjectId> {
        let form = self.new_agent_form()?;
        form.project.or_else(|| match form.purpose {
            crate::state::new_agent::Purpose::Start => self.project(cx),
            crate::state::new_agent::Purpose::AgentDefinition => None,
        })
    }

    /// A form that offers skills and servers asks for both layers it draws them from as it opens.
    pub(super) fn ask_catalog_for_form(&mut self, cx: &mut Context<Self>) {
        self.ask_catalog(None);
        if let Some(project) = self.form_catalog_scope(cx) {
            self.ask_catalog(Some(project));
        }
    }

    // ── Answers ─────────────────────────────────────────────────────

    /// One layer arrived, whole. A form waiting on its write closes; a browser waiting on an
    /// install stops waiting.
    pub(super) fn apply_catalog(
        &mut self,
        scope: Option<ProjectId>,
        layer: CatalogLayer,
        cx: &mut Context<Self>,
    ) {
        let catalog = &mut self.workbench.settings.catalog;
        catalog.replace(scope, layer);
        if catalog
            .form
            .as_ref()
            .is_some_and(|form| form.saving && form.scope == scope)
        {
            catalog.form = None;
        }
        if let Some(browse) = catalog.browse.as_mut()
            && browse.scope == scope
        {
            browse.installing = None;
        }
        cx.notify();
    }

    /// The host refused something. The page shows the sentence; a form or a browser that was
    /// waiting stops waiting and keeps its place.
    pub(super) fn apply_catalog_error(&mut self, error: String, cx: &mut Context<Self>) {
        let catalog = &mut self.workbench.settings.catalog;
        if let Some(form) = catalog.form.as_mut() {
            form.saving = false;
            form.parsing = false;
            form.error = Some(error.clone());
        }
        if let Some(browse) = catalog.browse.as_mut() {
            browse.installing = None;
            browse.loading = false;
        }
        catalog.error = Some(error);
        cx.notify();
    }

    pub fn dismiss_catalog_error(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.catalog.error = None;
        cx.notify();
    }

    // ── Skills ──────────────────────────────────────────────────────

    /// Choose a folder on the machine the layer lives on: one skill (`Link`), or a folder to scan
    /// (`Folder`). Ubiq's own host browser, since the folder is the host's and not this window's.
    pub fn browse_skill_folder(
        &mut self,
        scope: Option<ProjectId>,
        scan: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (host, start) = match scope {
            Some(project) => (
                self.bus.host_of_project(project),
                WindowRegistry::read(cx)
                    .project(project)
                    .map(|snap| snap.record.path.clone()),
            ),
            None => (HostRef::Local, None),
        };
        let label = match host {
            HostRef::Local => "this machine".to_string(),
            HostRef::Remote(id) => self
                .remote_hosts()
                .into_iter()
                .find(|(remote, _)| *remote == id)
                .map(|(_, label)| label)
                .unwrap_or_else(|| "the host".to_string()),
        };
        self.begin_host_browse(host, label.clone(), start);
        let title = match scan {
            true => format!("Choose a folder of skills on {label}"),
            false => format!("Choose a skill folder on {label}"),
        };
        let request = PickerRequest::new(PickerOwner::SkillFolder { scope, scan }, title)
            .kind(PickKind::Folders)
            .count(PickerCount::Single)
            .commit(Commit::OnButton);
        self.open_file_picker(request, Vec::new(), PickerView::Tree, window, cx);
    }

    /// The folder the picker answered with becomes a skill (or a scanned folder) in the layer.
    pub fn accept_skill_folder(
        &mut self,
        scope: Option<ProjectId>,
        scan: bool,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.workbench.settings.catalog.error = None;
        let from = match scan {
            true => SkillAdd::Folder { path },
            false => SkillAdd::Link { path, id: None },
        };
        self.bus.send(Message::AddSkill { scope, from });
        cx.notify();
    }

    pub fn remove_skill(&mut self, scope: Option<ProjectId>, id: String, cx: &mut Context<Self>) {
        self.workbench.settings.catalog.error = None;
        self.bus.send(Message::RemoveSkill { scope, id });
        cx.notify();
    }

    pub fn remove_skill_folder(
        &mut self,
        scope: Option<ProjectId>,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.workbench.settings.catalog.error = None;
        self.bus.send(Message::RemoveSkillFolder { scope, path });
        cx.notify();
    }

    /// Open the repository browser for a layer and list what the sources offer.
    pub fn open_skill_browse(
        &mut self,
        scope: Option<ProjectId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        clear(&self.catalog_inputs.skill_search, window, cx);
        self.workbench.settings.catalog.error = None;
        self.workbench.settings.catalog.browse = Some(SkillBrowse::new(scope));
        // The sources are the application's, whichever layer an install lands in.
        self.ask_catalog(None);
        self.search_skills(cx);
    }

    pub fn close_skill_browse(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.catalog.browse = None;
        cx.notify();
    }

    /// Search the sources for what the query box says. An empty query lists everything.
    pub fn search_skills(&mut self, cx: &mut Context<Self>) {
        let query = self
            .catalog_inputs
            .skill_search
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(browse) = self.workbench.settings.catalog.browse.as_mut() else {
            return;
        };
        browse.asked = query.clone();
        browse.loading = true;
        let source = browse.source.clone();
        self.bus.send(Message::SearchSkills { query, source });
        cx.notify();
    }

    /// Narrow the browser to one source, or back to all of them, and search again.
    pub fn pick_skill_source(&mut self, source: Option<String>, cx: &mut Context<Self>) {
        if let Some(browse) = self.workbench.settings.catalog.browse.as_mut() {
            browse.source = source;
            browse.results.clear();
        }
        self.search_skills(cx);
    }

    pub(super) fn apply_skill_results(
        &mut self,
        query: String,
        results: Vec<RemoteSkillInfo>,
        problems: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(browse) = self.workbench.settings.catalog.browse.as_mut()
            && browse.asked == query
        {
            browse.results = results;
            browse.problems = problems;
            browse.loading = false;
        }
        cx.notify();
    }

    /// Install one result into the browser's layer. The answer is the layer.
    pub fn install_skill(&mut self, result: RemoteSkillInfo, cx: &mut Context<Self>) {
        let Some(browse) = self.workbench.settings.catalog.browse.as_mut() else {
            return;
        };
        browse.installing = Some(SkillBrowse::key(&result));
        let scope = browse.scope;
        self.workbench.settings.catalog.error = None;
        self.bus.send(Message::AddSkill {
            scope,
            from: SkillAdd::Remote {
                source: result.source,
                path: result.path,
                id: None,
            },
        });
        cx.notify();
    }

    // ── Sources ─────────────────────────────────────────────────────

    pub fn open_add_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        clear(&self.catalog_inputs.source_url, window, cx);
        clear(&self.catalog_inputs.source_subpath, window, cx);
        self.workbench.settings.catalog.add_source = true;
        let url = self.catalog_inputs.source_url.read(cx).focus_handle(cx);
        window.focus(&url, cx);
        cx.notify();
    }

    pub fn close_add_source(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.catalog.add_source = false;
        cx.notify();
    }

    /// Append the typed repository to the application's sources. The id is made from the URL and
    /// made unique; the host owns the file it lands in.
    pub fn save_add_source(&mut self, cx: &mut Context<Self>) {
        let url = self
            .catalog_inputs
            .source_url
            .read(cx)
            .value()
            .trim()
            .to_string();
        if url.is_empty() {
            return;
        }
        let subpath = Some(
            self.catalog_inputs
                .source_subpath
                .read(cx)
                .value()
                .trim()
                .to_string(),
        )
        .filter(|it| !it.is_empty());
        let mut sources = self.workbench.settings.catalog.global.sources.clone();
        let base = source_id(&url);
        let mut id = base.clone();
        let mut n = 2;
        while sources.iter().any(|it| it.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        sources.push(SkillSourceInfo {
            id,
            label: None,
            kind: SkillSourceKindInfo::Git {
                url,
                rev: None,
                subpath,
            },
        });
        self.workbench.settings.catalog.add_source = false;
        self.workbench.settings.catalog.error = None;
        self.bus.send(Message::SaveSkillSources { sources });
        cx.notify();
    }

    pub fn remove_skill_source(&mut self, id: String, cx: &mut Context<Self>) {
        let mut sources = self.workbench.settings.catalog.global.sources.clone();
        sources.retain(|it| it.id != id);
        self.workbench.settings.catalog.error = None;
        self.bus.send(Message::SaveSkillSources { sources });
        cx.notify();
    }

    // ── MCP servers ─────────────────────────────────────────────────

    /// Raise the MCP form for a layer: empty for an add, filled from the record for an edit.
    pub fn open_mcp_form(
        &mut self,
        scope: Option<ProjectId>,
        existing: Option<CatalogMcp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut form = McpForm::new(scope);
        let fields = match &existing {
            Some(mcp) => {
                form.previous_id = Some(mcp.id.clone());
                form.set_kind(mcp.kind);
                mcp_fields(mcp)
            }
            None => McpFields::default(),
        };
        form.fill = Some(fields);
        set_area(&self.catalog_inputs.mcp_paste, "", window, cx);
        self.workbench.settings.catalog.error = None;
        self.workbench.settings.catalog.form = Some(form);
        cx.notify();
    }

    /// Write what a fill left waiting into the form's boxes. Run once a frame, where there is a
    /// window to write with: an answer from the host arrives without one.
    pub(super) fn sync_catalog_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fields) = self
            .workbench
            .settings
            .catalog
            .form
            .as_mut()
            .and_then(|form| form.fill.take())
        else {
            return;
        };
        let inputs = &self.catalog_inputs;
        set(&inputs.mcp_id, &fields.id, window, cx);
        set(&inputs.mcp_command, &fields.command, window, cx);
        set_area(&inputs.mcp_args, &fields.args, window, cx);
        set_area(&inputs.mcp_env, &fields.env, window, cx);
        set(&inputs.mcp_url, &fields.url, window, cx);
        set_area(&inputs.mcp_headers, &fields.headers, window, cx);
        set(&inputs.mcp_description, &fields.description, window, cx);
    }

    pub fn close_mcp_form(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.catalog.form = None;
        cx.notify();
    }

    pub fn pick_mcp_kind(&mut self, kind: McpKind, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.settings.catalog.form.as_mut() {
            form.set_kind(kind);
        }
        cx.notify();
    }

    /// What the form's boxes say, as a record of the kind the pills say.
    pub fn mcp_form_record(&self, cx: &App) -> Option<CatalogMcp> {
        let form = self.workbench.settings.catalog.form.as_ref()?;
        let inputs = &self.catalog_inputs;
        let fields = McpFields {
            id: inputs.mcp_id.read(cx).value().to_string(),
            command: inputs.mcp_command.read(cx).value().to_string(),
            args: inputs.mcp_args.read(cx).value().to_string(),
            env: inputs.mcp_env.read(cx).value().to_string(),
            url: inputs.mcp_url.read(cx).value().to_string(),
            headers: inputs.mcp_headers.read(cx).value().to_string(),
            description: inputs.mcp_description.read(cx).value().to_string(),
        };
        Some(compose_mcp(form.kind(), &fields))
    }

    pub fn save_mcp_form(&mut self, cx: &mut Context<Self>) {
        let Some(mcp) = self.mcp_form_record(cx).filter(mcp_ready) else {
            return;
        };
        let Some(form) = self.workbench.settings.catalog.form.as_mut() else {
            return;
        };
        form.saving = true;
        form.error = None;
        let (scope, previous_id) = (form.scope, form.previous_id.clone());
        self.bus.send(Message::SaveCatalogMcp {
            scope,
            mcp: Box::new(mcp),
            previous_id,
        });
        cx.notify();
    }

    pub fn remove_catalog_mcp(
        &mut self,
        scope: Option<ProjectId>,
        id: String,
        cx: &mut Context<Self>,
    ) {
        self.workbench.settings.catalog.error = None;
        self.bus.send(Message::RemoveCatalogMcp { scope, id });
        cx.notify();
    }

    /// Ask the host to read the pasted configuration. Nothing is stored.
    pub fn parse_mcp_paste(&mut self, cx: &mut Context<Self>) {
        let text = self.catalog_inputs.mcp_paste.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        if let Some(form) = self.workbench.settings.catalog.form.as_mut() {
            form.parsing = true;
            form.error = None;
            form.picks.clear();
        }
        self.bus.send(Message::ParseMcpConfig { text });
        cx.notify();
    }

    /// The paste was read: one server fills the form, several are offered to pick from.
    pub(super) fn apply_mcp_parsed(
        &mut self,
        mut servers: Vec<CatalogMcp>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.workbench.settings.catalog.form.as_mut() else {
            return;
        };
        form.parsing = false;
        if let Some(error) = error {
            form.error = Some(error);
        } else if servers.is_empty() {
            form.error = Some("No MCP server found in what was pasted.".to_string());
        } else if servers.len() == 1 {
            let mcp = servers.remove(0);
            self.fill_form_from(&mcp, Vec::new(), cx);
        } else {
            form.picks = servers;
        }
        cx.notify();
    }

    /// Fill the form from one of the several servers a paste held.
    pub fn use_mcp_pick(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(mcp) = self
            .workbench
            .settings
            .catalog
            .form
            .as_ref()
            .and_then(|form| form.picks.get(index).cloned())
        else {
            return;
        };
        self.fill_form_from(&mcp, Vec::new(), cx);
    }

    /// Write every server a paste held, and close the form. One message each: the host answers
    /// each with the layer.
    pub fn add_all_mcp_picks(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.workbench.settings.catalog.form.take() else {
            return;
        };
        for mcp in form.picks.into_iter().filter(mcp_ready) {
            self.bus.send(Message::SaveCatalogMcp {
                scope: form.scope,
                mcp: Box::new(mcp),
                previous_id: None,
            });
        }
        cx.notify();
    }

    /// Fill the boxes and the pills from a record. An id the record does not have leaves what was
    /// typed alone.
    fn fill_form_from(
        &mut self,
        mcp: &CatalogMcp,
        hints: Vec<ubiq_proto::catalog::McpParamHint>,
        cx: &mut Context<Self>,
    ) {
        let mut fields = mcp_fields(mcp);
        if fields.id.is_empty() {
            fields.id = self.catalog_inputs.mcp_id.read(cx).value().to_string();
        }
        if let Some(form) = self.workbench.settings.catalog.form.as_mut() {
            form.fill = Some(fields);
            form.set_kind(mcp.kind);
            form.picks.clear();
            form.hints = hints;
            form.error = None;
        }
        cx.notify();
    }

    // ── The registry ────────────────────────────────────────────────

    /// Open the registry search for a layer and list what it offers first.
    pub fn open_mcp_registry(
        &mut self,
        scope: Option<ProjectId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        clear(&self.catalog_inputs.registry_search, window, cx);
        self.workbench.settings.catalog.registry = Some(RegistryBrowse {
            scope,
            ..RegistryBrowse::default()
        });
        let search = self
            .catalog_inputs
            .registry_search
            .read(cx)
            .focus_handle(cx);
        window.focus(&search, cx);
        self.search_mcp_registry(cx);
    }

    pub fn close_mcp_registry(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.catalog.registry = None;
        cx.notify();
    }

    /// Search from the top for what the query box says.
    pub fn search_mcp_registry(&mut self, cx: &mut Context<Self>) {
        let query = self
            .catalog_inputs
            .registry_search
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(registry) = self.workbench.settings.catalog.registry.as_mut() else {
            return;
        };
        registry.asked = query.clone();
        registry.loading = true;
        registry.appending = false;
        registry.error = None;
        self.bus.send(Message::SearchMcpRegistry {
            query,
            cursor: None,
        });
        cx.notify();
    }

    /// The next page of the same query.
    pub fn more_mcp_registry(&mut self, cx: &mut Context<Self>) {
        let Some(registry) = self.workbench.settings.catalog.registry.as_mut() else {
            return;
        };
        let Some(cursor) = registry.next_cursor.clone() else {
            return;
        };
        registry.loading = true;
        registry.appending = true;
        let query = registry.asked.clone();
        self.bus.send(Message::SearchMcpRegistry {
            query,
            cursor: Some(cursor),
        });
        cx.notify();
    }

    pub(super) fn apply_registry_results(
        &mut self,
        query: String,
        servers: Vec<RegistryMcpInfo>,
        next_cursor: Option<String>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(registry) = self.workbench.settings.catalog.registry.as_mut()
            && registry.asked == query
        {
            if registry.appending {
                registry.results.extend(servers);
            } else {
                registry.results = servers;
            }
            registry.next_cursor = next_cursor;
            registry.error = error;
            registry.loading = false;
            registry.appending = false;
        }
        cx.notify();
    }

    /// Take one way to run a registry server: it fills the MCP form, which opens if it was not
    /// already, and the search closes.
    pub fn pick_registry_option(
        &mut self,
        server: usize,
        option: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(registry) = self.workbench.settings.catalog.registry.take() else {
            return;
        };
        let Some((info, draft)) = registry
            .results
            .get(server)
            .and_then(|info| info.options.get(option).map(|draft| (info, draft.clone())))
        else {
            self.workbench.settings.catalog.registry = Some(registry);
            return;
        };
        if self.workbench.settings.catalog.form.is_none() {
            self.open_mcp_form(registry.scope, None, window, cx);
        }
        let mut mcp = draft.server;
        if mcp.id.is_empty() {
            mcp.id = registry_id(&info.name);
        }
        if mcp.description.is_none() {
            mcp.description = info.description.clone();
        }
        self.fill_form_from(&mcp, draft.params, cx);
    }

    // ── Skills and servers on the New agent form ────────────────────

    /// Tick or untick one skill on the open form. The list stays down, like the MCP one.
    pub fn toggle_new_agent_skill(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.toggle_skill(&id);
        }
        cx.notify();
    }

    /// The skills a form may offer: the application's and its target project's, the project's
    /// shadowing by id.
    pub fn form_skills(&self, cx: &App) -> Vec<SkillInfo> {
        self.workbench
            .settings
            .catalog
            .skills_in(self.form_catalog_scope(cx))
    }

    /// The catalog servers a form may offer, on [`Self::form_skills`]'s terms.
    pub fn form_catalog_mcps(&self, cx: &App) -> Vec<CatalogMcp> {
        self.workbench
            .settings
            .catalog
            .mcps_in(self.form_catalog_scope(cx))
    }
}
