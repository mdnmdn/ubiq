//! The Databases settings section's handlers: the connection form, Test, the SQLite file chooser,
//! *New database…*, and removing a connection.
//!
//! The form is `DbState::form`, painted at the window root (`ui::db::conn_form`). Its text fields
//! are inputs built when it opens; everything else is on the model, and `edit_db_form` is the one
//! door a control that flips a flag goes through. **A password is only ever typed**: Save and Test
//! read the field and send what is there as a [`SecretEdit`], and the form is never seeded with one.

use super::*;
use crate::state::db::form::{DbConnForm, DbFormInputs, FormText};
use ubiq_db::conn::parse_connection_string_with;
use ubiq_proto::db::DbKind;

/// One text field of the form, with its text in it.
fn text_input(
    window: &mut Window,
    cx: &mut Context<AppState>,
    placeholder: &str,
    value: &str,
    masked: bool,
) -> Entity<InputState> {
    let placeholder = placeholder.to_string();
    let state = cx.new(|cx| {
        let state = InputState::new(window, cx).placeholder(placeholder);
        match masked {
            true => state.masked(true),
            false => state,
        }
    });
    if !value.is_empty() {
        state.update(cx, |state, cx| state.set_value(value, window, cx));
    }
    state
}

/// What the port field says when it is empty: the engine's own.
fn port_hint(kind: DbKind) -> String {
    match kind.default_port() {
        Some(port) => format!("{port}"),
        None => String::new(),
    }
}

impl AppState {
    /// A `DbFileCreated`: the file *New database…* asked for exists, and becomes the form's path.
    pub(super) fn on_db_file_created(
        &mut self,
        project: ProjectId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.form.as_mut())
        {
            form.config.kind = DbKind::Sqlite;
            form.config.path = Some(path.into());
            form.error = None;
        }
        cx.notify();
    }

    /// A `DbFileError`: the reason, drawn on the form.
    pub(super) fn on_db_file_error(
        &mut self,
        project: ProjectId,
        _path: String,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.db.form.as_mut())
        {
            form.error = Some(message);
        }
        cx.notify();
    }

    /// Build the widgets queued for the window: the password prompt's field, which is made when a
    /// connection first asks for one, and the tree's focus.
    pub(super) fn build_db_form_widgets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wants_prompt = self.db(cx).is_some_and(|db| {
            db.password_prompt.is_some() && db.tree.prompt_input.is_none()
        });
        if wants_prompt {
            let input = text_input(window, cx, "Password", "", true);
            if let Some(db) = self.db_mut(cx) {
                db.tree.prompt_input = Some(input.clone());
            }
            let handle = input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        if self.db(cx).is_some_and(|db| db.tree.focus.is_none()) {
            let focus = cx.focus_handle();
            if let Some(db) = self.db_mut(cx) {
                db.tree.focus = Some(focus);
            }
        }
    }

    // ── The form ────────────────────────────────────────────────────

    /// Raise the connection form: empty for a new connection, from the saved one for an edit.
    pub fn open_db_conn_form(
        &mut self,
        conn: Option<DbConnId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(db) = self.db(cx) else {
            return;
        };
        let mut form = match conn.and_then(|id| db.connection(id)) {
            Some(saved) => DbConnForm::editing(saved),
            None => DbConnForm::new(DbKind::Postgres),
        };
        // A saved password is written to the keychain only when asked; the default follows what
        // this install can do.
        form.remember = !matches!(db.keystore, Some(ubiq_proto::db::DbKeystore::Unavailable(_)));
        let text = form.text();
        let hint = port_hint(form.kind());
        form.inputs = Some(DbFormInputs {
            name: text_input(window, cx, "Name", &text.name, false),
            conn_string: text_input(
                window,
                cx,
                "postgres://user@host/db  \u{b7}  Server=h;Database=d  \u{b7}  /path/to/file.db",
                "",
                false,
            ),
            host: text_input(window, cx, "localhost", &text.host, false),
            port: text_input(window, cx, &hint, &text.port, false),
            database: text_input(window, cx, "Database", &text.database, false),
            user: text_input(window, cx, "User", &text.user, false),
            password: text_input(window, cx, "Password", "", true),
        });
        self.workbench.open_menu = None;
        if let Some(db) = self.db_mut(cx) {
            db.form = Some(form);
        }
        cx.notify();
    }

    /// Take the form down with nothing saved.
    pub fn close_db_form(&mut self, cx: &mut Context<Self>) {
        if let Some(db) = self.db_mut(cx) {
            db.form = None;
        }
        cx.notify();
    }

    /// Whether the form is up — what `ui::shell` and the overlay stack ask.
    pub fn db_form_open(&self, cx: &App) -> bool {
        self.db(cx).is_some_and(|db| db.form.is_some())
    }

    /// Flip a flag or set a field of the form that is not a text input.
    pub fn edit_db_form(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut DbConnForm)) {
        if let Some(form) = self.db_mut(cx).and_then(|db| db.form.as_mut()) {
            edit(form);
            form.error = None;
        }
        cx.notify();
    }

    /// Choose the engine. What was typed stays; the port field's hint follows the engine.
    pub fn pick_db_form_kind(&mut self, kind: DbKind, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_db_form(cx, |form| {
            form.config.kind = kind;
            if kind != DbKind::MsSql {
                form.config.integrated_auth = false;
                form.config.instance = None;
            }
        });
        if let Some(port) = self
            .db(cx)
            .and_then(|db| db.form.as_ref())
            .and_then(|form| form.inputs.as_ref())
            .map(|inputs| inputs.port.clone())
        {
            let hint = port_hint(kind);
            port.update(cx, |state, cx| state.set_placeholder(hint, window, cx));
        }
    }

    /// The form's text fields and the password typed, read off their inputs.
    fn read_db_form(&self, cx: &App) -> Option<(FormText, String, String)> {
        let inputs = self.db(cx)?.form.as_ref()?.inputs.clone()?;
        let get = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let text = FormText {
            name: get(&inputs.name),
            host: get(&inputs.host),
            port: get(&inputs.port),
            database: get(&inputs.database),
            user: get(&inputs.user),
            password: None,
        };
        Some((text, get(&inputs.password), get(&inputs.conn_string)))
    }

    /// Parse what was pasted into the connection-string box and fill the form from it. A string
    /// that does not parse is said on the form, and nothing is touched.
    pub fn parse_db_form_string(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((text, _, pasted)) = self.read_db_form(cx) else {
            return;
        };
        let Some(form) = self.db_mut(cx).and_then(|db| db.form.as_mut()) else {
            return;
        };
        let hint = form.kind();
        let parsed = match parse_connection_string_with(&pasted, Some(hint)) {
            Ok(parsed) => parsed,
            Err(error) => {
                form.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let filled = form.fill_from(parsed, &text.name);
        let (inputs, kind) = (form.inputs.clone(), form.kind());
        let Some(inputs) = inputs else {
            return;
        };
        let hint = port_hint(kind);
        let set = |input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App| {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        };
        set(&inputs.name, &filled.name, window, cx);
        set(&inputs.host, &filled.host, window, cx);
        set(&inputs.port, &filled.port, window, cx);
        set(&inputs.database, &filled.database, window, cx);
        set(&inputs.user, &filled.user, window, cx);
        if let Some(password) = &filled.password {
            set(&inputs.password, password, window, cx);
        }
        inputs
            .port
            .update(cx, |state, cx| state.set_placeholder(hint, window, cx));
        cx.notify();
    }

    /// Choose a SQLite file with the platform's chooser.
    pub fn browse_db_form_path(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| {
                let path = path.to_string_lossy().into_owned();
                this.edit_db_form(cx, |form| {
                    form.config.kind = DbKind::Sqlite;
                    form.config.path = Some(path.into());
                });
            })
            .ok();
        })
        .detach();
    }

    /// Ask the platform where a new database file goes, then have the host create it. The path
    /// reaches the form when `DbFileCreated` answers.
    pub fn new_db_form_file(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let start = WindowRegistry::read(cx)
            .project(project)
            .map(|snap| snap.record.path.clone())
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(Path::new(&start), Some("database.sqlite"));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.create_db_file(project, path.to_string_lossy().into_owned());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Try the form's connection without saving it. The answer lands on `DbState::probes`.
    pub fn test_db_form(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some((text, typed, _)) = self.read_db_form(cx) else {
            return;
        };
        let Some(form) = self.db_mut(cx).and_then(|db| db.form.as_mut()) else {
            return;
        };
        let config = match form.collect(&text) {
            Ok(config) => config,
            Err(error) => {
                form.error = Some(error);
                cx.notify();
                return;
            }
        };
        let (id, edit) = (form.id, form.secret_edit(&typed));
        form.error = None;
        let probe = self.test_db_connection(project, id, config, edit);
        if let Some(form) = self.db_mut(cx).and_then(|db| db.form.as_mut()) {
            form.probe = Some(probe);
        }
        cx.notify();
    }

    /// Save the form: a refusal is said on the form and keeps it up; an accepted one closes it and
    /// the host answers with the list. An edited connection's open tree is forgotten — the host has
    /// dropped its sessions and the next open dials the new settings.
    pub fn save_db_form(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some((text, typed, _)) = self.read_db_form(cx) else {
            return;
        };
        let Some(db) = self.db_mut(cx) else {
            return;
        };
        let Some(form) = db.form.as_mut() else {
            return;
        };
        let config = match form.collect(&text) {
            Ok(config) => config,
            Err(error) => {
                form.error = Some(error);
                cx.notify();
                return;
            }
        };
        let (id, edit, remember) = (form.id, form.secret_edit(&typed), form.remember);
        db.form = None;
        if let Some(id) = id {
            db.tree.reset(id);
        }
        self.save_db_connection(project, id, config, edit, remember);
        cx.notify();
    }

    // ── The list ────────────────────────────────────────────────────

    /// Test a saved connection from its row, with the password the host holds for it.
    pub fn test_db_row(&mut self, conn: DbConnId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(config) = self
            .db(cx)
            .and_then(|db| db.connection(conn))
            .map(|saved| saved.config.clone())
        else {
            return;
        };
        let probe = self.test_db_connection(project, Some(conn), config, SecretEdit::Keep);
        if let Some(db) = self.db_mut(cx) {
            db.tree.tests.insert(conn, probe);
        }
        cx.notify();
    }

    /// Ask whether to remove a connection; the row turns into the question.
    pub fn ask_remove_db(&mut self, conn: Option<DbConnId>, cx: &mut Context<Self>) {
        if let Some(db) = self.db_mut(cx) {
            db.tree.removing = conn;
        }
        cx.notify();
    }

    /// The question answered yes.
    pub fn confirm_remove_db(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(conn) = self.db_mut(cx).and_then(|db| db.tree.removing.take()) else {
            return;
        };
        self.delete_db_connection(project, conn);
        cx.notify();
    }
}
