//! The connection form's model — what Add and Edit are editing, before it is saved.
//!
//! The text fields live in window-owned inputs ([`DbFormInputs`]); everything else is on
//! [`DbConnForm::config`]. [`DbConnForm::collect`] folds the two into the config that is sent, and
//! [`DbConnForm::fill_from`] does the reverse for a pasted connection string, so both directions
//! are pure and tested without a window.
//!
//! **The password is write-only.** The form never holds one read back: a field the person types in,
//! a *Forget password* flag, and the [`PasswordState`] the list reported. An Edit that touches
//! nothing sends [`SecretEdit::Keep`].

use gpui::Entity;
use gpui_component::input::InputState;
use ubiq_proto::db::{ConnectionConfig, DbConnection, DbKind, PasswordState, SecretEdit};
use ubiq_proto::ids::{DbConnId, DbProbeId};
use ubiq_proto::messages::Secret;

/// The inputs a form types into. Built when the form opens, where there is a `Window`.
#[derive(Clone)]
pub struct DbFormInputs {
    pub name: Entity<InputState>,
    pub conn_string: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub database: Entity<InputState>,
    pub user: Entity<InputState>,
    /// Masked, and empty unless the person typed: the saved password is never shown.
    pub password: Entity<InputState>,
}

/// What the form's text fields hold, as plain strings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormText {
    pub name: String,
    pub host: String,
    pub port: String,
    pub database: String,
    pub user: String,
    /// A password a pasted connection string carried; `None` when the field should be left alone.
    pub password: Option<String>,
}

pub struct DbConnForm {
    /// `None` for a new connection — the host mints the id on the first save.
    pub id: Option<DbConnId>,
    /// Everything but the text fields (and never a password).
    pub config: ConnectionConfig,
    /// *Forget password* was pressed: send `Clear` unless a new one is typed.
    pub forget_password: bool,
    /// What the host reported for the connection being edited.
    pub saved: PasswordState,
    /// Whether to seal a typed password under this install's key.
    pub remember: bool,
    /// The Test in flight or last answered, looked up in `DbState::probes`.
    pub probe: Option<DbProbeId>,
    /// What Parse, Save or *New database…* last refused, in words.
    pub error: Option<String>,
    pub inputs: Option<DbFormInputs>,
}

impl DbConnForm {
    pub fn new(kind: DbKind) -> Self {
        Self {
            id: None,
            config: ConnectionConfig::new(kind),
            forget_password: false,
            saved: PasswordState::None,
            remember: true,
            probe: None,
            error: None,
            inputs: None,
        }
    }

    /// The form for an existing connection.
    pub fn editing(conn: &DbConnection) -> Self {
        let mut config = conn.config.clone();
        config.password = None;
        Self {
            id: Some(conn.id),
            config,
            saved: conn.password,
            ..Self::new(conn.config.kind)
        }
    }

    pub fn kind(&self) -> DbKind {
        self.config.kind
    }

    /// The text the fields start with.
    pub fn text(&self) -> FormText {
        FormText {
            name: self.config.name.clone(),
            host: self.config.host.clone().unwrap_or_default(),
            port: self.config.port.map(|p| p.to_string()).unwrap_or_default(),
            database: self.config.database.clone().unwrap_or_default(),
            user: self.config.user.clone().unwrap_or_default(),
            password: None,
        }
    }

    /// Adopt a parsed connection string. The kind, the SSL mode, read-only, the path and the
    /// remaining parameters replace the form's; the text fields come back for the caller to put in
    /// their inputs. The name the person already typed is kept — only an empty one is filled in.
    pub fn fill_from(&mut self, mut parsed: ConnectionConfig, current_name: &str) -> FormText {
        let password = parsed.password.take();
        let name = match current_name.trim().is_empty() {
            true => parsed.name.clone(),
            false => current_name.to_string(),
        };
        let text = FormText {
            name: name.clone(),
            host: parsed.host.clone().unwrap_or_default(),
            port: parsed.port.map(|p| p.to_string()).unwrap_or_default(),
            database: parsed.database.clone().unwrap_or_default(),
            user: parsed.user.clone().unwrap_or_default(),
            password: password.clone(),
        };
        parsed.name = name;
        self.config = parsed;
        if password.is_some() {
            self.forget_password = false;
        }
        self.error = None;
        text
    }

    /// The config to send: the form's own fields with the text folded in. A network engine's
    /// fields that do not belong to a file database (and the reverse) are dropped, so switching the
    /// kind back and forth never leaves a stale host on a SQLite connection.
    pub fn collect(&self, text: &FormText) -> Result<ConnectionConfig, String> {
        fn some(value: &str) -> Option<String> {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        let mut config = self.config.clone();
        config.password = None;
        config.name = text.name.trim().to_string();
        if config.kind.is_file_based() {
            config.host = None;
            config.port = None;
            config.instance = None;
            config.database = None;
            config.user = None;
            config.integrated_auth = false;
        } else {
            config.path = None;
            config.host = some(&text.host);
            config.database = some(&text.database);
            config.user = some(&text.user);
            config.port = match some(&text.port) {
                Some(port) => Some(
                    port.parse::<u16>()
                        .map_err(|_| format!("`{port}` is not a port"))?,
                ),
                None => None,
            };
            if config.integrated_auth {
                config.user = None;
            }
        }
        config.validate().map_err(|error| error.to_string())?;
        if config.name.is_empty() {
            config.name = config.default_name();
        }
        Ok(config)
    }

    /// What this save does to the password, given what is typed in the field.
    pub fn secret_edit(&self, typed: &str) -> SecretEdit {
        if self.config.integrated_auth || self.config.kind.is_file_based() {
            return match self.saved {
                PasswordState::None => SecretEdit::Keep,
                _ => SecretEdit::Clear,
            };
        }
        match (typed.is_empty(), self.forget_password) {
            (false, _) => SecretEdit::Set(Secret::new(typed)),
            (true, true) => SecretEdit::Clear,
            (true, false) => SecretEdit::Keep,
        }
    }

    /// The word the form says beside the password field.
    pub fn saved_word(&self) -> &'static str {
        match (self.forget_password, self.saved) {
            (true, _) => "will be forgotten",
            (false, PasswordState::Saved) => "saved",
            (false, PasswordState::Session) => "kept for this session",
            (false, PasswordState::Missing) => "saved, but cannot be opened \u{2014} type it again",
            (false, PasswordState::None) => "not saved",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_db::conn::{SslMode, parse_connection_string};

    #[test]
    fn a_pasted_url_fills_every_field_and_the_password_goes_to_its_own_field() {
        let mut form = DbConnForm::new(DbKind::MySql);
        let parsed = parse_connection_string(
            "postgres://ada:s3cret@db.internal:5433/shop?sslmode=require&application_name=ubiq&readonly=true",
        )
        .unwrap();
        let text = form.fill_from(parsed, "");
        assert_eq!(form.kind(), DbKind::Postgres);
        assert_eq!(
            text,
            FormText {
                name: "shop @ db.internal".into(),
                host: "db.internal".into(),
                port: "5433".into(),
                database: "shop".into(),
                user: "ada".into(),
                password: Some("s3cret".into()),
            }
        );
        assert_eq!(form.config.ssl, SslMode::Require);
        assert!(form.config.read_only);
        assert_eq!(form.config.params.get("application_name").unwrap(), "ubiq");
        assert_eq!(form.config.password, None, "the form never holds the password");
        // Folding the text back gives the connection that was pasted, name included.
        let config = form.collect(&text).unwrap();
        assert_eq!(config.host.as_deref(), Some("db.internal"));
        assert_eq!(config.port, Some(5433));
        assert_eq!(config.password, None);
        assert_eq!(config.name, "shop @ db.internal");
    }

    #[test]
    fn a_name_the_person_typed_survives_a_paste() {
        let mut form = DbConnForm::new(DbKind::Postgres);
        let parsed = parse_connection_string("mysql://root@localhost/app").unwrap();
        let text = form.fill_from(parsed, "my local");
        assert_eq!(text.name, "my local");
        assert_eq!(form.kind(), DbKind::MySql);
        assert_eq!(form.collect(&text).unwrap().name, "my local");
    }

    #[test]
    fn a_pasted_sqlite_path_fills_the_path_and_clears_the_network_fields() {
        let mut form = DbConnForm::new(DbKind::Postgres);
        let text = form.fill_from(parse_connection_string("/data/app.sqlite3").unwrap(), "");
        assert_eq!(form.kind(), DbKind::Sqlite);
        assert_eq!(
            form.config.path.as_deref().and_then(|p| p.to_str()),
            Some("/data/app.sqlite3")
        );
        assert_eq!(text.host, "");
        assert_eq!(text.name, "app.sqlite3");
        // A stale host typed before the kind changed is not carried onto a file database.
        let stale = FormText {
            host: "leftover".into(),
            ..text
        };
        assert_eq!(form.collect(&stale).unwrap().host, None);
    }

    #[test]
    fn a_form_that_cannot_be_saved_says_why() {
        let form = DbConnForm::new(DbKind::Sqlite);
        assert!(form.collect(&FormText::default()).unwrap_err().contains("path"));
        let form = DbConnForm::new(DbKind::Postgres);
        let text = FormText {
            port: "http".into(),
            ..Default::default()
        };
        assert!(form.collect(&text).unwrap_err().contains("port"));
        // Nothing but a kind is enough for a server: `localhost` and the default port.
        assert!(form.collect(&FormText::default()).is_ok());
    }

    #[test]
    fn the_password_edit_is_keep_unless_the_person_acted() {
        let mut form = DbConnForm::new(DbKind::Postgres);
        form.saved = PasswordState::Saved;
        assert_eq!(form.secret_edit(""), SecretEdit::Keep);
        assert_eq!(form.secret_edit("hunter2"), SecretEdit::Set(Secret::new("hunter2")));
        form.forget_password = true;
        assert_eq!(form.secret_edit(""), SecretEdit::Clear);
        assert_eq!(form.secret_edit("new"), SecretEdit::Set(Secret::new("new")));
        assert_eq!(form.saved_word(), "will be forgotten");
        form.forget_password = false;
        assert_eq!(form.saved_word(), "saved");
    }

    #[test]
    fn editing_starts_from_the_saved_connection_and_asks_for_no_password() {
        let mut config = ConnectionConfig::new(DbKind::MsSql);
        config.name = "erp".into();
        config.host = Some("sql1".into());
        config.port = Some(1444);
        let conn = DbConnection {
            id: DbConnId::generate(),
            config,
            password: PasswordState::Saved,
        };
        let form = DbConnForm::editing(&conn);
        assert_eq!(form.id, Some(conn.id));
        assert_eq!(form.text().port, "1444");
        assert_eq!(form.secret_edit(""), SecretEdit::Keep);
        assert_eq!(form.collect(&form.text()).unwrap(), conn.config);
    }
}
