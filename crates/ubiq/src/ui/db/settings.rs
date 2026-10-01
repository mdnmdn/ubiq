//! The Databases section of the project settings dialog.
//!
//! One row per connection — name, engine, where it points, a read-only marker, what the host holds
//! for its password — with Test, Edit and Remove, a warning row when this install cannot keep a
//! password, and *Add connection*, which raises [`conn_form`] (a modal painted at the window root,
//! like the knowledge base's source form). **Remove is asked in the row**: the row turns into the
//! question, so no second modal is stacked over the page.
//!
//! The section hangs off a record, as the knowledge base's does: the sink's fixture page and the
//! create form draw nothing here.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Rgba, SharedString, Styled, Window, div, px};
use gpui_component::IconName;

use ubiq_proto::db::{DbConnection, DbKeystore, DbKind, PasswordState};

use crate::app::AppState;
use crate::state::db::DbState;
use crate::state::workbench::ProjectSettingsMode;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{elided, ghost_button, heading, mono, primary_button};
use crate::ui::sink::project::Form;

#[path = "conn_form.rs"]
pub mod conn_form;

/// The section's body, a plain column — the container owns the scroll.
pub fn render(
    app: &AppState,
    form: Form,
    _window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let live_record = form == Form::Live
        && matches!(
            app.workbench
                .project_settings
                .as_ref()
                .map(|settings| &settings.mode),
            Some(ProjectSettingsMode::Edit { .. })
        );
    if !live_record {
        return div().into_any_element();
    }

    let mut column = div().flex().flex_col().gap_1p5().child(heading(
        "Databases",
        "The connections this project's databases are reached by \u{2014} PostgreSQL, MySQL and \
         MariaDB, SQLite and SQL Server. They are shared with the project when it keeps its data \
         in the project folder; passwords never are, and stay on this machine.",
    ));

    let Some(db) = app.db(cx) else {
        return column.into_any_element();
    };

    if let Some(DbKeystore::Unavailable(why)) = &db.keystore {
        column = column.child(
            div()
                .py_1p5()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::warning())
                .child(SharedString::from(format!(
                    "This install cannot keep passwords ({why}). A password you type is held until \
                     Ubiq closes."
                ))),
        );
    }

    let rows: Vec<AnyElement> = match db.loaded {
        false => vec![note("Loading\u{2026}")],
        true if db.connections.is_empty() => vec![note("No connections yet. Add one below.")],
        true => db
            .connections
            .iter()
            .map(|conn| connection_row(db, conn, cx))
            .collect(),
    };

    column
        .child(div().flex().flex_col().children(rows))
        .child(
            div().flex().items_center().pt_3().child(primary_button(
                "project-db-add",
                Some(IconName::Plus),
                "Add connection",
                cx.listener(|this, _, window, cx| this.open_db_conn_form(None, window, cx)),
            )),
        )
        .into_any_element()
}

fn note(text: &str) -> AnyElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(theme::text_faint())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

/// Where a connection points, in one line: the file for SQLite, `host:port/database` otherwise.
pub fn describe(conn: &DbConnection) -> String {
    let config = &conn.config;
    if config.kind == DbKind::Sqlite {
        let path = config
            .path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        return format!("{} \u{b7} {path}", config.kind.display_name());
    }
    let mut target = config.effective_host().to_string();
    if let Some(instance) = &config.instance {
        target = format!("{target}\\{instance}");
    }
    if let Some(port) = config.effective_port() {
        target = format!("{target}:{port}");
    }
    if let Some(database) = config.database.as_deref().filter(|d| !d.is_empty()) {
        target = format!("{target}/{database}");
    }
    format!("{} \u{b7} {target}", config.kind.display_name())
}

/// What the host holds for the password, as a word and the colour it earns. A file database and
/// integrated auth have none to hold, and say nothing.
fn password_word(conn: &DbConnection) -> Option<(&'static str, Rgba)> {
    if conn.config.kind.is_file_based() || conn.config.integrated_auth {
        return None;
    }
    Some(match conn.password {
        PasswordState::Saved => ("saved", theme::text_faint()),
        PasswordState::None => ("not saved", theme::text_faint()),
        PasswordState::Session => ("session", theme::text_faint()),
        PasswordState::Missing => ("needs password", theme::warning()),
    })
}

fn connection_row(db: &DbState, conn: &DbConnection, cx: &mut Context<AppState>) -> AnyElement {
    let id = conn.id;
    let meta = theme::font(Family::Chrome, Role::Meta);

    // The answer to this row's last Test: pending, or what came back, in the status group's colours.
    let tested = db
        .tree
        .tests
        .get(&id)
        .map(|probe| match db.probes.get(probe) {
            None => ("Testing\u{2026}".to_string(), theme::text_faint()),
            Some(Ok(version)) => (
                match version.is_empty() {
                    true => "Connected".to_string(),
                    false => format!("Connected \u{b7} {version}"),
                },
                theme::success(),
            ),
            Some(Err(failure)) => (failure.message.clone(), theme::danger()),
        });

    let removing = db.tree.removing == Some(id);
    let actions = match removing {
        true => div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Label))
                    .text_color(theme::danger())
                    .child("Remove this connection?"),
            )
            .child(ghost_button(
                crate::ui::eid("project-db-keep", id),
                None,
                "Keep",
                cx.listener(|this, _, _, cx| this.ask_remove_db(None, cx)),
            ))
            .child(primary_button(
                crate::ui::eid("project-db-remove-yes", id),
                None,
                "Remove",
                cx.listener(|this, _, _, cx| this.confirm_remove_db(cx)),
            )),
        false => div()
            .flex()
            .items_center()
            .gap_1()
            .child(ghost_button(
                crate::ui::eid("project-db-test", id),
                None,
                "Test",
                cx.listener(move |this, _, _, cx| this.test_db_row(id, cx)),
            ))
            .child(ghost_button(
                crate::ui::eid("project-db-edit", id),
                None,
                "Edit",
                cx.listener(move |this, _, window, cx| {
                    this.open_db_conn_form(Some(id), window, cx)
                }),
            ))
            .child(ghost_button(
                crate::ui::eid("project-db-remove", id),
                None,
                "Remove",
                cx.listener(move |this, _, _, cx| this.ask_remove_db(Some(id), cx)),
            )),
    };

    let mut text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .gap_1()
        .child(elided(
            crate::ui::eid("project-db-name", id),
            conn.config.name.clone(),
            theme::text(),
            theme::font(Family::Chrome, Role::Body),
        ))
        .child(mono(describe(conn), theme::text_faint()).text_size(meta));
    if let Some((line, colour)) = tested {
        text = text.child(elided(
            crate::ui::eid("project-db-tested", id),
            line,
            colour,
            meta,
        ));
    }

    div()
        .flex()
        .items_center()
        .gap_2()
        .py_1p5()
        .border_b_1()
        .border_color(theme::border())
        .child(text)
        .children(conn.config.read_only.then(|| {
            div()
                .flex_none()
                .px_1()
                .bg(theme::db_read_only_soft())
                .child(crate::ui::kit::badge("read-only", theme::db_read_only()))
        }))
        .children(password_word(conn).map(|(word, colour)| {
            div()
                .flex_none()
                .text_size(meta)
                .text_color(colour)
                .child(word)
        }))
        .child(actions)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::db::ConnectionConfig;
    use ubiq_proto::ids::DbConnId;

    fn saved(config: ConnectionConfig, password: PasswordState) -> DbConnection {
        DbConnection {
            id: DbConnId::generate(),
            config,
            password,
        }
    }

    #[test]
    fn a_row_says_where_the_connection_points() {
        let mut config = ConnectionConfig::new(DbKind::Postgres);
        config.host = Some("db.internal".into());
        config.database = Some("orders".into());
        assert_eq!(
            describe(&saved(config, PasswordState::Saved)),
            "PostgreSQL \u{b7} db.internal:5432/orders"
        );
        let mut file = ConnectionConfig::new(DbKind::Sqlite);
        file.path = Some("data/cache.sqlite".into());
        assert_eq!(
            describe(&saved(file, PasswordState::None)),
            "SQLite \u{b7} data/cache.sqlite"
        );
    }

    #[test]
    fn a_file_database_has_no_password_word_and_a_lost_one_is_flagged() {
        let file = saved(ConnectionConfig::new(DbKind::Sqlite), PasswordState::None);
        assert!(password_word(&file).is_none());
        let lost = saved(ConnectionConfig::new(DbKind::MySql), PasswordState::Missing);
        assert_eq!(password_word(&lost).map(|(word, _)| word), Some("needs password"));
        let kept = saved(ConnectionConfig::new(DbKind::MySql), PasswordState::Saved);
        assert_eq!(password_word(&kept).map(|(word, _)| word), Some("saved"));
    }
}
