//! The shared query editors: named SQL editors an agent writes in and the user watches, edits and
//! runs. **In the host's memory only**, per project; nothing here touches a disk or a bus.
//!
//! An editor is the agent's own — keyed `(agent_key, name)`, so two agents naming a panel the same
//! do not fight over it. Its `rev` is bumped on every change of text, and identical text is no
//! change. Each agent's last write is remembered as the `rev` it left (`agent_rev`): the editor has
//! *changed since that agent's last write* when `rev` is past it, or — for an agent that never
//! wrote — when anything at all was written.
//!
//! The user's edits carry the `rev` they started from, and one that started from an older `rev`
//! than the editor's is **stale**: an agent wrote in between, and the user's text would silently
//! undo it. It is refused, and the window is told the editor as it stands.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use ubiq_proto::db::DbEditor;
use ubiq_proto::ids::{DbConnId, DbSessionId, ProjectId};

/// How an agent's edit treats a change somebody else made since its own last write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    /// Write regardless.
    Force,
    /// Write, and hand back the text that was replaced if somebody else had changed it.
    OverwriteReturnPrevious,
    /// Write only if nobody else changed it; otherwise leave it and hand back what is there.
    KeepIfUserChanged,
}

/// Who last changed an editor's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditedBy {
    User,
    /// An agent, by its key.
    Agent(String),
}

/// What an agent is told about an editor after opening, reading or editing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorReport {
    /// The editor as it now stands: `editor.rev` and `editor.text` are the current ones.
    pub editor: DbEditor,
    /// This call created it.
    pub created: bool,
    /// Somebody other than this agent changed the text since its last write — measured *before*
    /// this call wrote anything.
    pub changed_since_last_write: bool,
    /// `None` until anybody has written.
    pub last_edited_by: Option<EditedBy>,
    /// [`EditMode::OverwriteReturnPrevious`]: the replaced text, when it had been changed.
    pub previous_text: Option<String>,
    /// [`EditMode::KeepIfUserChanged`]: nothing was written, because it had been changed.
    pub kept: bool,
}

/// What became of a user's edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserEdit {
    /// Written; everyone is to be told.
    Applied(DbEditor),
    /// Started from an older `rev`; refused, and the asker is to be told the editor as it stands.
    Stale(DbEditor),
    /// The same text: nothing to say.
    Unchanged,
    /// No editor has that session.
    Unknown,
}

struct Entry {
    editor: DbEditor,
    /// Each agent's `rev` as of its last write.
    agent_rev: HashMap<String, u64>,
    last: Option<EditedBy>,
}

impl Entry {
    fn changed_for(&self, agent_key: &str) -> bool {
        match self.agent_rev.get(agent_key) {
            Some(rev) => self.editor.rev > *rev,
            None => self.editor.rev > 0,
        }
    }

    /// Answers whether the text changed.
    fn set_text(&mut self, text: String, by: EditedBy) -> bool {
        if text == self.editor.text {
            return false;
        }
        self.editor.text = text;
        self.editor.rev += 1;
        self.last = Some(by);
        true
    }

    fn agent_write(&mut self, agent_key: &str, text: String) -> bool {
        let changed = self.set_text(text, EditedBy::Agent(agent_key.to_string()));
        self.agent_rev
            .insert(agent_key.to_string(), self.editor.rev);
        changed
    }

    fn report(&self, created: bool, changed: bool) -> EditorReport {
        EditorReport {
            editor: self.editor.clone(),
            created,
            changed_since_last_write: changed,
            last_edited_by: self.last.clone(),
            previous_text: None,
            kept: false,
        }
    }

    fn is(&self, agent_key: &str, name: &str) -> bool {
        self.editor.agent_key.as_deref() == Some(agent_key) && self.editor.name == name
    }
}

/// Every project's shared editors.
#[derive(Default)]
pub struct Editors {
    projects: Mutex<HashMap<ProjectId, Vec<Entry>>>,
}

impl Editors {
    fn projects(&self) -> MutexGuard<'_, HashMap<ProjectId, Vec<Entry>>> {
        self.projects.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A project's editors, oldest first.
    pub fn list(&self, project: ProjectId) -> Vec<DbEditor> {
        self.projects()
            .get(&project)
            .map(|entries| entries.iter().map(|e| e.editor.clone()).collect())
            .unwrap_or_default()
    }

    /// The agent's editor called `name`.
    pub fn get(&self, agent_key: &str, project: ProjectId, name: &str) -> Option<DbEditor> {
        self.projects()
            .get(&project)?
            .iter()
            .find(|e| e.is(agent_key, name))
            .map(|e| e.editor.clone())
    }

    /// Open the agent's editor `name` on `conn`, creating it if it is not there; `sql` is written
    /// as [`EditMode::Force`] would. Answers the report and whether the editor changed at all —
    /// created, re-pointed, retitled or rewritten — which is what a broadcast waits on.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        agent_key: &str,
        agent_title: &str,
        project: ProjectId,
        name: &str,
        conn: DbConnId,
        database: Option<String>,
        sql: Option<String>,
    ) -> (EditorReport, bool) {
        let mut projects = self.projects();
        let entries = projects.entry(project).or_default();
        let found = entries.iter().position(|e| e.is(agent_key, name));
        let created = found.is_none();
        if created {
            entries.push(Entry {
                editor: DbEditor {
                    name: name.to_string(),
                    session: DbSessionId::generate(),
                    conn,
                    database: database.clone(),
                    text: String::new(),
                    rev: 0,
                    agent_key: Some(agent_key.to_string()),
                    agent_title: Some(agent_title.to_string()),
                },
                agent_rev: HashMap::new(),
                last: None,
            });
        }
        let index = found.unwrap_or(entries.len() - 1);
        let entry = &mut entries[index];
        let changed_before = entry.changed_for(agent_key);
        let mut touched = created;
        if entry.editor.conn != conn || entry.editor.database != database {
            entry.editor.conn = conn;
            entry.editor.database = database;
            touched = true;
        }
        if entry.editor.agent_title.as_deref() != Some(agent_title) {
            entry.editor.agent_title = Some(agent_title.to_string());
            touched = true;
        }
        if let Some(sql) = sql {
            touched |= entry.agent_write(agent_key, sql);
        }
        (entry.report(created, changed_before), touched)
    }

    pub fn read(&self, agent_key: &str, project: ProjectId, name: &str) -> Option<EditorReport> {
        let projects = self.projects();
        let entry = projects
            .get(&project)?
            .iter()
            .find(|e| e.is(agent_key, name))?;
        Some(entry.report(false, entry.changed_for(agent_key)))
    }

    /// An agent's edit under `mode`. `None` when it has no editor of that name. The flag is
    /// whether the text changed.
    pub fn edit(
        &self,
        agent_key: &str,
        agent_title: &str,
        project: ProjectId,
        name: &str,
        sql: String,
        mode: EditMode,
    ) -> Option<(EditorReport, bool)> {
        let mut projects = self.projects();
        let entry = projects
            .get_mut(&project)?
            .iter_mut()
            .find(|e| e.is(agent_key, name))?;
        let changed = entry.changed_for(agent_key);
        entry.editor.agent_title = Some(agent_title.to_string());
        if mode == EditMode::KeepIfUserChanged && changed {
            let mut report = entry.report(false, true);
            report.kept = true;
            return Some((report, false));
        }
        let previous = entry.editor.text.clone();
        let touched = entry.agent_write(agent_key, sql);
        let mut report = entry.report(false, changed);
        if mode == EditMode::OverwriteReturnPrevious && changed {
            report.previous_text = Some(previous);
        }
        Some((report, touched))
    }

    /// The user typed in an editor's tab.
    pub fn user_edit(
        &self,
        project: ProjectId,
        session: DbSessionId,
        text: String,
        base_rev: u64,
    ) -> UserEdit {
        let mut projects = self.projects();
        let Some(entry) = projects
            .get_mut(&project)
            .and_then(|entries| entries.iter_mut().find(|e| e.editor.session == session))
        else {
            return UserEdit::Unknown;
        };
        if base_rev != entry.editor.rev {
            return UserEdit::Stale(entry.editor.clone());
        }
        if entry.set_text(text, EditedBy::User) {
            UserEdit::Applied(entry.editor.clone())
        } else {
            UserEdit::Unchanged
        }
    }
}
