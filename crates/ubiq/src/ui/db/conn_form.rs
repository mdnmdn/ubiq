//! The connection form: Add and Edit a database connection, in one modal painted at the window root
//! (declared from `settings.rs`, which raises it; `ui::shell` paints it, `Layer::DbForm` guards its
//! dismissal).
//!
//! Top to bottom: the name, the engine, a box that takes a pasted connection string and fills the
//! rest, the fields that engine has (a file and two ways to get one for SQLite; host, port,
//! database, user and the password for a server), SSL, read-only, the Agents section (access, default, description), and the answer to the last Test.
//!
//! **The password field is write-only.** It is empty on open, even when one is saved; what it says
//! beside the field is `saved` / `not saved` from the list's `PasswordState`, and the form sends
//! `Keep` unless something is typed or *Forget password* is pressed.

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Context, Entity, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::IconName;
use gpui_component::input::{Input, InputState};

use ubiq_proto::db::{DbAgentAccess, DbKeystore, DbKind, SslMode};

use crate::app::AppState;
use crate::state::db::form::{DbConnForm, DbFormInputs};
use crate::state::overlay::Layer;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{
    check_box, choice_pill, elided, ghost_button, hint_row, label_hint, primary_button, toggle_pill,
};
use crate::ui::sink::style::{framed_active, input_on};

/// How wide every control in the right-hand column is drawn. One width, so the column reads as a
/// column rather than a ragged edge — the knowledge base form's own rule.
const CONTROL_WIDTH: f32 = 250.;

/// The modal.
pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(db) = app.db(cx) else {
        return div().into_any_element();
    };
    let Some(form) = db.form.as_ref() else {
        return div().into_any_element();
    };
    let view = cx.entity();

    // Test's answer, looked up by the probe the form sent.
    let tested = form.probe.map(|probe| match db.probes.get(&probe) {
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

    let mut answer = div().flex().flex_col().flex_1().min_w(px(0.));
    if let Some((line, colour)) = tested {
        answer = answer.child(elided(
            "db-form-tested",
            line,
            colour,
            theme::font(Family::Chrome, Role::Meta),
        ));
    }
    if let Some(error) = form.error.clone() {
        answer = answer.child(elided(
            "db-form-error",
            error,
            theme::danger(),
            theme::font(Family::Chrome, Role::Meta),
        ));
    }

    let footer = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .gap_2()
        .child(ghost_button(
            "db-form-test",
            None,
            "Test",
            cx.listener(|this, _, _, cx| this.test_db_form(cx)),
        ))
        .child(answer)
        .child(ghost_button(
            "db-form-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_db_form(cx)),
        ))
        .child(primary_button(
            "db-form-save",
            None,
            "Save",
            cx.listener(|this, _, _, cx| this.save_db_form(cx)),
        ))
        .into_any_element();

    crate::ui::kit::modal(
        "db-form",
        theme::accent(),
        match form.id {
            Some(_) => "Edit connection",
            None => "Add connection",
        },
        body(form, db.keystore.as_ref(), window, cx),
        footer,
        crate::ui::dismiss(&view, Layer::DbForm, |this, _, cx| this.close_db_form(cx)),
        window,
    )
}

fn body(
    form: &DbConnForm,
    keystore: Option<&DbKeystore>,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(inputs) = form.inputs.as_ref() else {
        return div().into_any_element();
    };
    let kind = form.kind();
    let file = kind.is_file_based();
    let mut rows = div().flex().flex_col().gap_1().pt_1();

    // 1 ─ what the explorer's top-level row will say.
    rows = rows.child(field_row(
        "db-form-name",
        "Name",
        "What the explorer calls this connection. Filled from the connection string when empty.",
        &inputs.name,
        window,
        cx,
    ));

    // 2 ─ the engine, drawn as a question of its own: everything below is downstream of it.
    let mut kinds = div().flex().flex_wrap().gap_1();
    for (ix, candidate) in DbKind::ALL.into_iter().enumerate() {
        kinds = kinds.child(choice_pill(
            ("db-form-kind", ix),
            candidate.display_name(),
            kind == candidate,
            cx.listener(move |this, _, window, cx| this.pick_db_form_kind(candidate, window, cx)),
        ));
    }
    rows = rows.child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pb_2()
            .child(label_hint(
                "db-form-kind-hint",
                "Engine",
                "PostgreSQL, MySQL and MariaDB, SQLite or SQL Server.",
            ))
            .child(kinds),
    );

    // 3 ─ a pasted connection string, parsed in the interface: it is pure, and the person sees the
    // fields fill before anything is saved.
    rows = rows.child(hint_row(
        "db-form-string-hint",
        "Connection string",
        "A URL, an ADO.NET, ODBC or JDBC string, a libpq key=value line, or a SQLite path. Parse \
         fills the fields below from it.",
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .w(px(CONTROL_WIDTH))
            .child(
                framed_active(theme::border(), input_on(&inputs.conn_string, window, cx))
                    .h(px(26.))
                    .flex_1()
                    .min_w(px(0.))
                    .items_center()
                    .child(Input::new(&inputs.conn_string).appearance(false)),
            )
            .child(ghost_button(
                "db-form-parse",
                None,
                "Parse",
                cx.listener(|this, _, window, cx| this.parse_db_form_string(window, cx)),
            ))
            .into_any_element(),
    ));

    if file {
        // 4a ─ a file: chosen with the platform's chooser, or made.
        let path = form
            .config
            .path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        rows = rows.child(hint_row(
            "db-form-path-hint",
            "Database file",
            "An existing SQLite file, or a new one created empty.",
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_1()
                .w(px(CONTROL_WIDTH))
                .child(
                    div()
                        .flex()
                        .h(px(26.))
                        .px_2()
                        .items_center()
                        .bg(theme::surface())
                        .border_l(px(theme::accent_edge()))
                        .border_color(theme::border())
                        .child(match path {
                            Some(path) => elided(
                                "db-form-path",
                                path,
                                theme::text(),
                                theme::font(Family::Chrome, Role::Label),
                            )
                            .into_any_element(),
                            None => div()
                                .text_size(theme::font(Family::Chrome, Role::Label))
                                .text_color(theme::text_faint())
                                .child("No file chosen")
                                .into_any_element(),
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(ghost_button(
                            "db-form-browse",
                            Some(IconName::FolderOpen),
                            "Browse\u{2026}",
                            cx.listener(|this, _, _, cx| this.browse_db_form_path(cx)),
                        ))
                        .child(ghost_button(
                            "db-form-new-file",
                            Some(IconName::Plus),
                            "New database\u{2026}",
                            cx.listener(|this, _, _, cx| this.new_db_form_file(cx)),
                        )),
                )
                .into_any_element(),
        ));
    } else {
        // 4b ─ a server.
        rows = rows
            .child(field_row(
                "db-form-host",
                "Host",
                "A host name or address. Empty means localhost.",
                &inputs.host,
                window,
                cx,
            ))
            .child(field_row(
                "db-form-port",
                "Port",
                "Empty means the engine's own port.",
                &inputs.port,
                window,
                cx,
            ))
            .child(field_row(
                "db-form-database",
                "Database",
                "The database to open first.",
                &inputs.database,
                window,
                cx,
            ));
        if kind == DbKind::MsSql {
            rows = rows.child(hint_row(
                "db-form-integrated-hint",
                "Authentication",
                "Windows integrated authentication sends no user or password.",
                div()
                    .flex_none()
                    .w(px(CONTROL_WIDTH))
                    .child(toggle_pill(
                        "db-form-integrated",
                        "Integrated",
                        theme::accent(),
                        form.config.integrated_auth,
                        cx.listener(|this, _, _, cx| {
                            this.edit_db_form(cx, |form| {
                                form.config.integrated_auth = !form.config.integrated_auth
                            })
                        }),
                    ))
                    .into_any_element(),
            ));
        }
        if !form.config.integrated_auth {
            rows = rows
                .child(field_row(
                    "db-form-user",
                    "User",
                    "",
                    &inputs.user,
                    window,
                    cx,
                ))
                .child(password_row(
                    form,
                    inputs.password.clone(),
                    keystore,
                    window,
                    cx,
                ));
        }
        // SSL, as the four steps all the network engines can say.
        let mut modes = div().flex().flex_wrap().gap_1();
        for (ix, (label, mode)) in [
            ("Off", SslMode::Disable),
            ("Prefer", SslMode::Prefer),
            ("Require", SslMode::Require),
            ("Verify", SslMode::VerifyFull),
        ]
        .into_iter()
        .enumerate()
        {
            modes = modes.child(choice_pill(
                ("db-form-ssl", ix),
                label,
                form.config.ssl == mode,
                cx.listener(move |this, _, _, cx| {
                    this.edit_db_form(cx, |form| form.config.ssl = mode)
                }),
            ));
        }
        rows = rows.child(hint_row(
            "db-form-ssl-hint",
            "Encryption",
            "Off is plain TCP; Prefer uses TLS when the server offers it; Require insists but does \
             not check the certificate; Verify checks it.",
            div()
                .flex_none()
                .w(px(CONTROL_WIDTH))
                .child(modes)
                .into_any_element(),
        ));
    }

    // 5 ─ read-only, for either engine: the host holds every statement on the connection to it.
    rows = rows.child(hint_row(
        "db-form-readonly-hint",
        "Access",
        "A read-only connection refuses every statement that writes, whatever a tab says.",
        div()
            .flex_none()
            .w(px(CONTROL_WIDTH))
            .child(toggle_pill(
                "db-form-readonly",
                "Read-only",
                theme::db_read_only(),
                form.config.read_only,
                cx.listener(|this, _, _, cx| {
                    this.edit_db_form(cx, |form| form.set_read_only(!form.config.read_only))
                }),
            ))
            .into_any_element(),
    ));

    // 6 ─ what an agent may do, through the SQL MCP servers.
    rows = rows.child(agents_section(form, inputs, window, cx));

    rows.into_any_element()
}

/// The Agents section: access, whether it is the default, and a description.
fn agents_section(
    form: &DbConnForm,
    inputs: &DbFormInputs,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let label = theme::font(Family::Chrome, Role::Label);
    let mut levels = div().flex().flex_wrap().gap_1();
    for (ix, (name, access)) in [
        ("None", DbAgentAccess::None),
        ("Read-only", DbAgentAccess::Ro),
        ("Read-write", DbAgentAccess::Rw),
    ]
    .into_iter()
    .enumerate()
    {
        // Read-write is not offered on a read-only connection.
        let enabled = access != DbAgentAccess::Rw || form.can_write();
        levels = levels.child(
            choice_pill(
                ("db-form-agent-access", ix),
                name,
                form.agent.access == access,
                cx.listener(move |this, _, _, cx| {
                    this.edit_db_form(cx, |form| form.set_agent_access(access))
                }),
            )
            .when(!enabled, |this| this.opacity(0.4).cursor_default()),
        );
    }
    let mut access_col = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1()
        .w(px(CONTROL_WIDTH))
        .child(levels);
    if form.read_only_is_best_effort() {
        access_col = access_col.child(div().text_size(label).text_color(theme::warning()).child(
            "Read-only is best-effort on SQL Server: use a login that has read-only rights.",
        ));
    }

    let can_default = form.can_default();
    let default_box = check_box(
        "db-form-agent-default",
        form.agent.default,
        cx.listener(|this, _, _, cx| {
            this.edit_db_form(cx, |form| {
                if form.can_default() {
                    form.agent.default = !form.agent.default
                }
            })
        }),
    );
    let default_row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(match can_default {
            true => default_box,
            false => default_box.opacity(0.4).cursor_default(),
        })
        .child(
            div()
                .text_size(label)
                .text_color(match can_default {
                    true => theme::text(),
                    false => theme::text_faint(),
                })
                .child("Default for agents"),
        );

    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(hint_row(
            "db-form-agent-access-hint",
            "Agents",
            "What an agent may do with this connection through the SQL servers. Read-write needs a \
             connection that is not read-only.",
            access_col.into_any_element(),
        ))
        .child(hint_row(
            "db-form-agent-default-hint",
            "Default",
            "The connection an agent gets when it names none. One per project; setting it here \
             clears it elsewhere. Needs some access.",
            div()
                .flex_none()
                .w(px(CONTROL_WIDTH))
                .child(default_row)
                .into_any_element(),
        ))
        .child(field_row(
            "db-form-agent-description",
            "Description",
            "What the connection is for, as the agent reads it.",
            &inputs.description,
            window,
            cx,
        ))
        .into_any_element()
}

/// The write-only password: a field that is empty unless typed in, what the host holds, and the
/// two choices about keeping it.
fn password_row(
    form: &DbConnForm,
    input: Entity<InputState>,
    keystore: Option<&DbKeystore>,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let sealable = !matches!(keystore, Some(DbKeystore::Unavailable(_)));
    let saved = form.saved != ubiq_proto::db::PasswordState::None;
    let label = theme::font(Family::Chrome, Role::Label);
    let mut column = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1()
        .w(px(CONTROL_WIDTH))
        .child(
            framed_active(theme::border(), input_on(&input, window, cx))
                .h(px(26.))
                .items_center()
                .child(Input::new(&input).appearance(false).mask_toggle()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(match form.saved {
                            ubiq_proto::db::PasswordState::Missing => theme::warning(),
                            _ => theme::text_faint(),
                        })
                        .child(form.saved_word()),
                )
                .when(saved && !form.forget_password, |this| {
                    this.child(ghost_button(
                        "db-form-forget",
                        None,
                        "Forget password",
                        cx.listener(|this, _, _, cx| {
                            this.edit_db_form(cx, |form| form.forget_password = true)
                        }),
                    ))
                }),
        );
    column = match sealable {
        true => column.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(check_box(
                    "db-form-remember",
                    form.remember,
                    cx.listener(|this, _, _, cx| {
                        this.edit_db_form(cx, |form| form.remember = !form.remember)
                    }),
                ))
                .child(
                    div()
                        .text_size(label)
                        .text_color(theme::text())
                        .child("Remember on this machine"),
                ),
        ),
        false => column.child(
            div()
                .text_size(label)
                .text_color(theme::warning())
                .child("Not kept: this install has no keychain, so it lasts until Ubiq closes."),
        ),
    };
    hint_row(
        "db-form-password-hint",
        "Password",
        "Write-only: a saved password is used by the host and never shown. Leave it empty to keep \
         it.",
        column.into_any_element(),
    )
}

/// A labelled text field in the right-hand column.
fn field_row(
    id: &'static str,
    label: &str,
    hint: &str,
    input: &Entity<InputState>,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    hint_row(
        id,
        label,
        // A field with nothing to explain still gets a `?` of its own; `hint_row` wants a sentence.
        match hint.is_empty() {
            true => label,
            false => hint,
        },
        framed_active(theme::border(), input_on(input, window, cx))
            .h(px(26.))
            .flex_none()
            .w(px(CONTROL_WIDTH))
            .items_center()
            .child(Input::new(input).appearance(false))
            .into_any_element(),
    )
}
