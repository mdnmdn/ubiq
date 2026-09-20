//! The knowledge base's own mutators: what a click on a row asks for, what the settings dialog's
//! fields commit, and the one function that writes the whole source list.
//!
//! Nothing here holds a source's tree or its document — that is [`KbState`], reached through
//! [`AppState::kb_mut`]. What lives here is the wire between a gesture and a message: a click
//! becomes a [`crate::state::KbPressed`] the state answers with, and this turns that answer into
//! whatever the host needs asked, exactly as `app/explorer.rs` turns an `ExplorerPressed` into a
//! `ProjectTree`.
//!
//! **The "Add source" form lives here too.** Its arithmetic is [`crate::state::kb::KbSourceForm`];
//! what is here is the three things only the window can do for it — raise Ubiq's own folder picker
//! against the host the project runs on, ask that host to list a repository's branches, and append
//! the answered source to the configured list through the one `SetKbSources` sender.

use super::*;
use crate::app::explorer::{child_path, leaf_of};
use crate::state::file_picker::{Commit, PickKind, PickerCount, PickerRequest, PickerView};
use crate::state::kb::{KbAction, KbKind, KbList, KbMenuRow, KbSourceForm, KbUrlCheck};
use ubiq_proto::files::EntryKind;
use ubiq_proto::ids::RepoQueryId;
use ubiq_proto::repos::RepoSource;

impl AppState {
    /// What this project's knowledge base is configured as. Answered with `KbSourcesListed`, which
    /// `receive_kb` hands to [`KbState::accept`].
    pub fn ask_kb_sources(&mut self, project: ProjectId) {
        self.bus.send(Message::KbSources {
            project_id: project,
        });
    }

    /// One open document of the project on screen, by its tab key. What a KB panel draws and what
    /// its tab reports are both this — [`AppState::file`] for the documents half.
    pub fn kb_doc(&self, key: &str, cx: &App) -> Option<&OpenFile> {
        self.open_project(cx)?.kb.doc(key)
    }

    /// A press on one row of the KB explorer.
    pub fn click_kb_row(&mut self, source: KbSourceId, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        match open.kb.click(source, &path) {
            KbPressed::Listing { source, path } => {
                open.kb.mark_loading(source, &path);
                self.bus.send(Message::KbTree {
                    project_id: project,
                    source,
                    rel_path: path,
                    depth: 1,
                });
            }
            KbPressed::Open(key) => {
                // The tab appears on the click rather than on the answer — `select_file`'s rule,
                // said here: a click with no visible effect invites a second one, and a read that
                // fails needs somewhere to say so. A document already open is not read again: its
                // tab holds its bytes, and whatever has been typed into them would go with a
                // second read.
                let markdown_open = self.workbench.settings.ui.markdown_open.layout();
                let tab = key.tab_key();
                open.kb.selected = Some(key.clone());
                let fresh = open.kb.open_doc(&key, markdown_open);
                if fresh {
                    // The tab and its panel open together: each document is its own panel, so a
                    // tab with none is a document with nowhere to be drawn.
                    self.pending_panels
                        .push(PanelEdit::Open(PanelKind::Kb(tab)));
                    self.bus.send(Message::ReadKbFile {
                        project_id: project,
                        source: key.source,
                        rel_path: key.path,
                        max_bytes: Some(MAX_FILE_BYTES),
                    });
                } else {
                    // A read that failed is retried by clicking the row again — the tab is the
                    // only place the refusal is shown, so it has to be the place it is answered.
                    let retry = open
                        .kb
                        .doc(&tab)
                        .is_some_and(|doc| matches!(doc.body, FileBody::Failed(_)));
                    if retry {
                        if let Some(doc) = open.kb.doc_mut(&tab) {
                            doc.reload();
                        }
                        self.bus.send(Message::ReadKbFile {
                            project_id: project,
                            source: key.source,
                            rel_path: key.path,
                            max_bytes: Some(MAX_FILE_BYTES),
                        });
                    }
                    // Already a panel the dock holds, so it is brought forward rather than added
                    // twice — which tab a group displays is the dock's to say.
                    self.pending_panels
                        .push(PanelEdit::Reveal(PanelKind::Kb(tab)));
                }
            }
            KbPressed::Sync(source) => {
                self.sync_kb_source(source, cx);
            }
            KbPressed::Toggled => {}
        }
        cx.notify();
    }

    /// Fetch or refresh a source that has to be fetched — a folder needs no such ask, and the host
    /// treats one as a no-op rather than the interface filtering it out here.
    pub fn sync_kb_source(&mut self, source: KbSourceId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::SyncKbSource {
            project_id: project,
            source,
        });
    }

    /// Turn everything the host answered since the last frame into a document's body —
    /// `app/editor.rs`'s `attach_arrived_files`, said again for the knowledge base because an
    /// `EditorState` needs a `Window` and `KbFileContents` does not carry one.
    pub(super) fn attach_kb_docs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for arrival in std::mem::take(&mut self.pending_kb_docs) {
            self.attach_kb_doc(arrival, window, cx);
        }
    }

    fn attach_kb_doc(&mut self, arrival: KbArrival, window: &mut Window, cx: &mut Context<Self>) {
        let KbArrival {
            project_id,
            key,
            contents,
        } = arrival;
        let tab = key.tab_key();
        let Some(open) = self.projects.get(&project_id) else {
            return;
        };
        // A tab that already holds bytes is never overwritten, exactly as a file's is not:
        // whatever has been typed into it would go with them.
        if !open.kb.doc(&tab).is_some_and(|doc| doc.is_loading()) {
            return;
        }
        // A source Ubiq may not write to is read exactly as it is drawn, and simply never saved:
        // `savable_kb_doc` is where that is decided, so the buffer here is the same one either
        // way and the read-only half keeps the IDE's highlighting rather than losing it.
        if contents.is_binary {
            if let Some(doc) = self
                .projects
                .get_mut(&project_id)
                .and_then(|open| open.kb.doc_mut(&tab))
            {
                doc.set_binary();
            }
            cx.notify();
            return;
        }

        // An image is its own bytes, handed straight to `ui/viewer/image.rs` exactly as a project
        // file's are (`T-32`): it is not text, and a buffer would be a lossy decode of it. Unlike
        // a project file it stays plain `FileBody::Bytes` rather than `OpenFile::set_image`'s
        // `ImageEdit` — annotating a knowledge-base picture would need `WriteKbFile` to carry
        // bytes, which it does not yet, so the toggle that offers annotation tools is never drawn
        // here and Save is never asked to write back what it cannot send.
        if crate::state::editor::ViewerKind::of(&key.path) == crate::state::editor::ViewerKind::Image
        {
            if let Some(doc) = self
                .projects
                .get_mut(&project_id)
                .and_then(|open| open.kb.doc_mut(&tab))
            {
                doc.set_bytes(contents.bytes);
            }
            cx.notify();
            return;
        }

        // Everything else — Markdown, Mermaid, Excalidraw, draw.io, the plain editor — is a
        // buffer, the same one a project file of that viewer kind gets: the diagram's source text,
        // which is what its `Edit` layout hands to the web-panel bridge (`ui/viewer/web.rs`).
        let text = String::from_utf8_lossy(&contents.bytes).into_owned();
        let language = FileLanguage::of(&key.path);
        let buffer = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(ui::editor::highlighter_language(language))
                .line_number(true)
                .soft_wrap(true)
                .default_value(text.clone())
        });

        let watched = tab.clone();
        let change = cx.subscribe_in(
            &buffer,
            window,
            move |this, buffer, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let typed = buffer.read(cx).value().to_string();
                if let Some(open) = this.projects.get_mut(&project_id)
                    && let Some(doc) = open.kb.doc_mut(&watched)
                {
                    doc.refresh_dirty(&typed);
                }
                cx.notify();
            },
        );

        if let Some(doc) = self
            .projects
            .get_mut(&project_id)
            .and_then(|open| open.kb.doc_mut(&tab))
        {
            doc.attach(buffer, text, contents.truncated, contents.version, change);
        }
        cx.notify();
    }

    /// Whether a document's source is one Ubiq may write back to. **The one place
    /// `KbSource::is_writable` is read** to decide whether a save is offered at all: a document on
    /// a read-only source is still opened, highlighted and read, and simply never written.
    pub fn savable_kb_doc(&self, key: &str, cx: &App) -> bool {
        let Some(open) = self.open_project(cx) else {
            return false;
        };
        let Some(doc) = open.kb.doc(key) else {
            return false;
        };
        doc.kb_source
            .and_then(|source| open.kb.source(source))
            .is_some_and(|view| view.status.source.is_writable())
    }

    /// Write one document's buffer back through `WriteKbFile`. A no-op with nothing typed since
    /// the last save, with a save already in flight, or over a source Ubiq may not write to — a
    /// stray keybinding must not send a second write racing the first.
    pub fn save_kb_doc(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        if !self.savable_kb_doc(key, cx) {
            return;
        }
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(doc) = open.kb.doc_mut(key) else {
            return;
        };
        if !doc.dirty() || doc.is_saving() {
            return;
        }
        let Some(buffer) = doc.buffer().cloned() else {
            return;
        };
        let Some(doc_key) = KbDocKey::of(doc) else {
            return;
        };
        let contents = buffer.read(cx).value().to_string();
        doc.mark_saving(contents.clone());
        self.bus.send(Message::WriteKbFile {
            project_id: project,
            source: doc_key.source,
            rel_path: doc_key.path,
            contents,
        });
        cx.notify();
    }

    /// A KB panel became the displayed tab of its group, so its document is the one the explorer
    /// highlights — `AppState::activate_file`'s direction, said for the documents half.
    pub fn activate_kb_doc(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(doc_key) = open.kb.doc(key).and_then(KbDocKey::of) else {
            return;
        };
        open.kb.selected = Some(doc_key);
        cx.notify();
    }

    /// Close one document's tab from the state side — the tab menu's Close.
    ///
    /// The panel goes with it as a queued `PanelEdit`, never through `ui::dock::close_panel`:
    /// this runs inside an `AppState` update, and that function takes a second lease on the same
    /// entity to reach the dock.
    pub fn close_kb_doc(&mut self, key: &str, cx: &mut Context<Self>) {
        let closed = self
            .project(cx)
            .and_then(|project| self.projects.get_mut(&project))
            .is_some_and(|open| open.kb.close_doc(key));
        if closed {
            // A diagram may have opened a web-panel session over this tab (`T-32`); it goes with
            // the tab exactly as a file's does in `force_close_tab`.
            self.close_web_session(key);
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::Kb(key.to_string())));
        }
        cx.notify();
    }

    /// A KB panel left the dock for good, so its tab closes with it.
    ///
    /// **A document's × closes the tab and nothing else**, the way a file's does: there is no
    /// harness behind it for `TabClose` to be about, and the bytes are the host's either way.
    pub fn closed_kb_panel(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(project) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&project)
        {
            open.kb.close_doc(key);
        }
        self.close_web_session(key);
        self.panels.remove(&PanelKind::Kb(key.to_string()));
        cx.notify();
    }

    // ── The right-click menu ────────────────────────────────────────

    /// Raise the KB explorer's menu at the pointer. A row the tree no longer holds raises nothing:
    /// a menu with no row behind it would offer commands aimed at a path that has gone.
    pub fn open_kb_menu(
        &mut self,
        source: KbSourceId,
        path: String,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let opened = self
            .kb_mut(cx)
            .is_some_and(|kb| kb.open_menu(source, &path, at.0, at.1));
        if opened {
            self.workbench.open_menu = Some(MenuId::Kb);
        }
        cx.notify();
    }

    /// Take the menu down with the window's own `close_menu`, which peels every menu at once.
    pub(super) fn drop_kb_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(kb) = self.kb_mut(cx) {
            kb.menu = None;
        }
    }

    /// The menu's own outside click, carrying the menu it was drawn for — `dismiss_explorer_menu`'s
    /// discipline: a dismiss for a menu already replaced by the right-click that raised the next
    /// one does nothing.
    pub fn dismiss_kb_menu(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let gone = match self.kb_mut(cx) {
            Some(kb) => {
                kb.close_menu(epoch);
                kb.menu.is_none()
            }
            None => false,
        };
        if gone && self.workbench.open_menu == Some(MenuId::Kb) {
            self.workbench.open_menu = None;
        }
        cx.notify();
    }

    /// Act on the row the menu drew at `index`. The index is into [`KbMenu::entries`], so the menu
    /// is taken down and read here rather than by whoever drew it.
    pub fn pick_kb_action(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(menu) = ({
            let Some(kb) = self.kb_mut(cx) else {
                return;
            };
            let menu = kb.menu.clone();
            kb.menu = None;
            menu
        }) else {
            return;
        };
        self.workbench.open_menu = None;
        let Some(action) = menu.entries().get(index).copied() else {
            cx.notify();
            return;
        };

        let source = menu.source;
        let path = menu.path.clone();
        match action {
            // The host's machine, not the window's: a source may sit on a drone, and `RevealKbPath`
            // is what makes "show me this" mean the same thing in both cases.
            KbAction::OpenInSystem => self.bus.send(Message::RevealKbPath {
                project_id: project,
                source,
                rel_path: path,
            }),
            KbAction::Sync => self.sync_kb_source(source, cx),
            // The one place the interface learns an absolute host path, and it is *asked for*
            // rather than volunteered: no listing carries one, so the clipboard is the only thing
            // that ever holds one and only because the user asked. The answer lands in
            // `receive_kb`'s `KbPath` arm.
            KbAction::CopyPath => self.bus.send(Message::AskKbPath {
                project_id: project,
                source,
                rel_path: path,
            }),
            // Only a source row and a folder row offer these, so the row's own path *is* the
            // folder the new entry lands in.
            KbAction::NewFile | KbAction::NewFolder => {
                let dir = action == KbAction::NewFolder;
                self.open_file_dialog(
                    FileDialog::KbNew {
                        source,
                        parent: path,
                        dir,
                    },
                    "",
                    window,
                    cx,
                );
            }
            // Ubiq's label for the source, not the folder behind it — which is why it is seeded
            // from the configured name and commits through `SetKbSources`.
            KbAction::RenameSource => {
                let seed = self
                    .kb(cx)
                    .and_then(|kb| kb.source(source))
                    .map(|view| view.name().to_string())
                    .unwrap_or_default();
                self.open_file_dialog(FileDialog::KbRenameSource { source }, &seed, window, cx);
            }
            KbAction::Rename => {
                let leaf = leaf_of(&path).to_string();
                self.open_file_dialog(FileDialog::KbRename { source, path }, &leaf, window, cx);
            }
            KbAction::Delete => {
                let dir = menu.row == KbMenuRow::Dir;
                self.open_file_dialog(FileDialog::KbRemove { source, path, dir }, "", window, cx);
            }
            KbAction::Separator => {}
        }
        cx.notify();
    }

    /// Answer whichever of the knowledge base's four questions is up, with whatever was typed.
    ///
    /// Called from `confirm_file_dialog`, which owns the field and takes the dialog down
    /// afterwards — this only turns an answer into a message.
    pub(super) fn confirm_kb_dialog(
        &mut self,
        dialog: FileDialog,
        project: ProjectId,
        typed: String,
        cx: &mut Context<Self>,
    ) {
        match dialog {
            FileDialog::KbNew {
                source,
                parent,
                dir,
            } => {
                if typed.is_empty() {
                    return;
                }
                self.bus.send(Message::CreateKbEntry {
                    project_id: project,
                    source,
                    rel_path: child_path(&parent, &typed),
                    kind: match dir {
                        true => EntryKind::Dir,
                        false => EntryKind::File,
                    },
                });
            }
            FileDialog::KbRename { source, path } => {
                if typed.is_empty() || typed == leaf_of(&path) {
                    return;
                }
                // A name, never a path: the host refuses one holding a separator, and the dialog
                // does not try to guess a move out of it.
                self.bus.send(Message::RenameKbEntry {
                    project_id: project,
                    source,
                    rel_path: path,
                    new_name: typed,
                });
            }
            FileDialog::KbRenameSource { source } => {
                if typed.is_empty() {
                    return;
                }
                let Some(mut sources) =
                    self.projects.get(&project).map(|open| open.kb.configured())
                else {
                    return;
                };
                let Some(entry) = sources.iter_mut().find(|existing| existing.id == source) else {
                    return;
                };
                if entry.name == typed {
                    return;
                }
                entry.name = typed;
                self.write_kb_sources(project, sources);
            }
            FileDialog::KbRemove { source, path, .. } => {
                self.bus.send(Message::DeleteKbEntry {
                    project_id: project,
                    source,
                    rel_path: path,
                });
            }
            _ => {}
        }
        cx.notify();
    }

    /// Raise the project settings dialog open on the KB nav, asking for the configuration first
    /// when nobody has yet.
    pub fn open_kb_settings(&mut self, cx: &mut Context<Self>) {
        self.open_edit_project(cx);
        let Some(settings) = self.workbench.project_settings.as_mut() else {
            return;
        };
        settings.nav = ProjectNav::Kb;
        cx.notify();
    }

    // ── The "Add source" form ───────────────────────────────────────

    /// Raise the modal, empty. The two fields it types into are the window's, so what a previous
    /// answer left in them is cleared here rather than carried into this one.
    pub fn open_kb_source_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_project().is_none() {
            return;
        }
        self.workbench.open_menu = None;
        self.workbench.kb_source = Some(KbSourceForm::default());
        for field in [self.kb_name_input.clone(), self.kb_url_input.clone()] {
            field.update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Take it down with nothing written. The browse session goes too: a picker left half-open
    /// behind a modal that is gone has nowhere to put its answer.
    pub fn close_kb_source_form(&mut self, cx: &mut Context<Self>) {
        self.workbench.kb_source = None;
        self.close_host_browse();
        cx.notify();
    }

    /// Choose what is being added. Everything the other kind filled in goes with the switch — see
    /// [`KbSourceForm::set_kind`] — so the fields are emptied to match.
    pub fn pick_kb_source_kind(
        &mut self,
        kind: KbKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.workbench.kb_source.as_mut() else {
            return;
        };
        form.set_kind(kind);
        form.open = None;
        let name = form.name.clone();
        self.kb_url_input
            .clone()
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.fill_kb_source_name(name, window, cx);
        cx.notify();
    }

    /// Put a seeded name into the field the user has not typed in. A no-op once they have — the
    /// form itself keeps that rule, and this only follows what it decided.
    fn fill_kb_source_name(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.kb_name_input.clone();
        if field.read(cx).value().as_ref() != name.as_str() {
            field.update(cx, |state, cx| state.set_value(&name, window, cx));
        }
    }

    /// The name field was typed in. From here the name is the user's and is never seeded again.
    pub fn retype_kb_source_name(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(form) = self.workbench.kb_source.as_mut() else {
            return;
        };
        if form.name == name {
            return;
        }
        form.rename(name);
        cx.notify();
    }

    /// The URL field changed. Nothing is sent: a branch listing costs a network round trip, so it
    /// waits for the Check button rather than firing at every keystroke.
    pub fn retype_kb_source_url(
        &mut self,
        url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.workbench.kb_source.as_mut() else {
            return;
        };
        form.set_url(url);
        let name = form.name.clone();
        self.fill_kb_source_name(name, window, cx);
        cx.notify();
    }

    /// Raise **Ubiq's own** folder picker over the host the project runs on.
    ///
    /// Not the platform dialog: a knowledge base source is a folder on the machine the *host* is,
    /// which the window's own chooser cannot see past. `open_remote_project_picker` is the same
    /// two steps — begin a browse session, raise a `PickKind::Folders` picker over it — with a
    /// different owner on the end.
    ///
    /// Opens on the project's own root rather than the host's default starting place: a source is
    /// almost always found inside or beside the project it is being added to, not wherever the
    /// host's browse defaults to.
    pub fn browse_kb_source_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let host = self.bus.host_of_project(project);
        let label = match host {
            HostRef::Local => "this machine".to_string(),
            HostRef::Remote(id) => self
                .remote_hosts()
                .into_iter()
                .find(|(remote, _)| *remote == id)
                .map(|(_, label)| label)
                .unwrap_or_else(|| "the host".to_string()),
        };
        let start = WindowRegistry::read(cx)
            .project(project)
            .map(|snap| snap.record.path.clone());
        self.begin_host_browse(host, label.clone(), start);
        let request =
            PickerRequest::new(PickerOwner::KbFolder, format!("Choose a folder on {label}"))
                .kind(PickKind::Folders)
                .count(PickerCount::Single)
                .commit(Commit::OnButton);
        self.open_file_picker(request, Vec::new(), PickerView::Tree, window, cx);
    }

    /// Fold the folder the picker answered with into the form. Called from `commit_file_picker`,
    /// which is the only thing that knows a `PickerOwner::KbFolder` dialog has closed.
    pub fn accept_kb_source_folder(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.workbench.kb_source.as_mut() else {
            return;
        };
        form.set_path(path);
        let name = form.name.clone();
        self.fill_kb_source_name(name, window, cx);
        cx.notify();
    }

    /// Ask the host what branches the typed URL has — `Message::ListRepoBranches`, the clone
    /// modal's own ask, answered by `RepoBranches` or `RepoError` in `receive_repo`.
    ///
    /// It doubles as the check that the repository is reachable at all, which is why the button
    /// says Check rather than "list branches": an answer of any kind is what says the URL is real.
    pub fn check_kb_source_url(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.workbench.kb_source.as_mut() else {
            return;
        };
        let url = form.url.trim().to_string();
        if url.is_empty() {
            return;
        }
        let query_id = RepoQueryId::generate();
        form.branches_query = Some(query_id);
        form.branches.clear();
        form.check = KbUrlCheck::Checking;
        self.bus.send(Message::ListRepoBranches {
            query_id,
            source: RepoSource::Url(url),
        });
        cx.notify();
    }

    pub fn pick_kb_source_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.kb_source.as_mut() {
            form.branch = branch;
            form.open = None;
        }
        cx.notify();
    }

    pub fn pick_kb_source_store(&mut self, store: KbStore, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.kb_source.as_mut() {
            form.store = store;
            form.open = None;
        }
        cx.notify();
    }

    pub fn pick_kb_source_access(&mut self, access: KbAccess, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.kb_source.as_mut() {
            form.access = access;
            form.open = None;
        }
        cx.notify();
    }

    /// Open or shut one of the form's lists, clearing and focusing the shared picker field —
    /// `open_picker_menu`'s gesture, said for a list whose open state lives on the form.
    pub fn toggle_kb_source_list(
        &mut self,
        list: KbList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.workbench.kb_source.as_mut() else {
            return;
        };
        let opening = form.open != Some(list);
        form.open = opening.then_some(list);
        if opening {
            let search = self.picker_search.clone();
            search.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub fn dismiss_kb_source_list(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.kb_source.as_mut() {
            form.open = None;
        }
        cx.notify();
    }

    /// Append the answered source to the configured list and write the whole list.
    ///
    /// A form that is not answerable writes nothing: Confirm is drawn faint in that state, and
    /// this is the same rule kept on the acting side so a keyboard confirm cannot slip past it.
    pub fn confirm_kb_source(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let Some(source) = self
            .workbench
            .kb_source
            .as_ref()
            .and_then(KbSourceForm::as_source)
        else {
            return;
        };
        let Some(mut sources) = self.projects.get(&project).map(|open| open.kb.configured()) else {
            return;
        };
        sources.push(source);
        self.set_kb_sources(sources, cx);
        self.close_kb_source_form(cx);
    }

    /// Drop a source from the configuration.
    pub fn remove_kb_source(&mut self, source: KbSourceId, cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let Some(mut sources) = self.projects.get(&project).map(|open| open.kb.configured()) else {
            return;
        };
        let before = sources.len();
        sources.retain(|existing| existing.id != source);
        if sources.len() == before {
            return;
        }
        self.set_kb_sources(sources, cx);
    }

    /// Write one source's filter, a no-op when it did not change — so a blur that changed nothing
    /// sends nothing, the same discipline `set_project_search_excludes`'s callers keep.
    pub fn set_kb_source_filter(
        &mut self,
        source: KbSourceId,
        filter: String,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let Some(mut sources) = self.projects.get(&project).map(|open| open.kb.configured()) else {
            return;
        };
        let Some(entry) = sources.iter_mut().find(|existing| existing.id == source) else {
            return;
        };
        if entry.filter == filter {
            return;
        }
        entry.filter = filter;
        self.set_kb_sources(sources, cx);
    }

    /// The one sender of `SetKbSources`, for whichever project the settings dialog is editing. The
    /// host answers with `KbSourcesListed`, which is what actually moves `KbState` — this only
    /// asks; see the family's banner in `ubiq-proto/src/messages.rs` for why a whole-list write is
    /// one message rather than one row.
    pub(super) fn set_kb_sources(&mut self, sources: Vec<KbSource>, _cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        self.write_kb_sources(project, sources);
    }

    /// The same write for a project the settings dialog is *not* editing — the explorer's own
    /// Rename source, which is asked with no settings page open and so has no `editing_project`.
    fn write_kb_sources(&mut self, project: ProjectId, sources: Vec<KbSource>) {
        self.bus.send(Message::SetKbSources {
            project_id: project,
            sources,
        });
    }

    /// Keep one filter field per configured source, seeded and subscribed the moment a source
    /// arrives with none — called once a frame from [`Render`], on `settle_nav`'s discipline: **no
    /// `cx.notify()`**, because this runs inside the same render it would otherwise spin.
    pub(super) fn ensure_kb_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.project_settings.is_none() {
            return;
        }
        let Some(project) = self.editing_project() else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let sources = open.kb.configured();

        self.kb_filter_inputs
            .retain(|id, _| sources.iter().any(|source| source.id == *id));
        self.kb_filter_subs
            .retain(|id, _| sources.iter().any(|source| source.id == *id));

        for source in &sources {
            if self.kb_filter_inputs.contains_key(&source.id) {
                continue;
            }
            let field = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("*.md *.excalidraw")
                    .default_value(source.filter.clone())
            });
            let id = source.id;
            let subscription = cx.subscribe_in(
                &field,
                window,
                move |this, input, event: &InputEvent, _window, cx| {
                    // A filter typed and clicked away must stick — the same reasoning `boot.rs`'s
                    // comma-separated search-excludes field gives its own commit-on-blur.
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let value = input.read(cx).value().to_string();
                        this.set_kb_source_filter(id, value, cx);
                    }
                },
            );
            self.kb_filter_inputs.insert(source.id, field);
            self.kb_filter_subs.insert(source.id, subscription);
        }
    }
}
