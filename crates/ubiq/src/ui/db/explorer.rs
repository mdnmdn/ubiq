//! The databases explorer panel — the left edge of DB mode — and the password prompt a connection
//! raises.
//!
//! The KB explorer's shape (`ui/kb/mod.rs`) with the IDE explorer's filter and keys: `kit::panel`
//! under a header carrying + and the gear, `kit::filter_bar` over the window-owned `db_filter`, and
//! under it the lazy tree `connection → database → [schema] → object group → object → column`,
//! drawn with `kit::file_row` and `kit::twisty`. A connection row says where it stands; a table row
//! carries its approximate count at the far end. The filter narrows what is **loaded** and never
//! fetches (`state::db::tree::visible`).
//!
//! Keys: the panel's own live once the tree holds the keyboard, the field keeps every key a field
//! keeps, and three cross the boundary — Down and Tab step from the field onto the tree, Escape
//! clears the query from either, Enter opens what the query landed on.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClickEvent, Context, Focusable as _, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, ParentElement, Rgba, StatefulInteractiveElement, Styled, Window,
    div, point, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::db::{DbConnState, DbKeystore, ObjectKind};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::db::DbState;
use crate::state::db::tree::{DbKey, Load, Node, NodeKind, Row, approx_label, visible};
use crate::state::overlay::Layer;
use crate::theme::{self, Family, Role};
use crate::ui::empty;
use crate::ui::kit::{
    ContextItem, UbiqIcon, badge, check_box, context_menu, elided_with, file_row, filter_bar,
    icon_button_tip, modal, panel, panel_header, primary_button, twisty,
};

/// The key context the panel is answered in, and the one the component library gives the field
/// inside it.
const CONTEXT: &str = "DbExplorer";
const FIELD_CONTEXT: &str = "DbExplorer > Input";
/// The password prompt's, and its field's.
const PASSWORD_CONTEXT: &str = "DbPassword";
const PASSWORD_FIELD_CONTEXT: &str = "DbPassword > Input";

gpui::actions!(
    ubiq_db_explorer,
    [
        DbUp,
        DbDown,
        DbOut,
        DbInto,
        DbEnter,
        DbDismiss,
        DbFocusTree,
        DbFocusFilter,
        DbPasswordSubmit
    ]
);

/// The keys the panel answers to, and where each is live. The IDE explorer's rule
/// ([`crate::ui::explorer::key_bindings`]): the tree's keys are bound at the panel and reachable
/// once the tree holds the keyboard; the field's are bound at the field's depth, because the
/// component library binds at the deepest node and a binding on the panel alone would never be
/// reached while the caret is in it.
pub fn key_bindings() -> Vec<KeyBinding> {
    fn panel<A: gpui::Action>(key: &str, action: A) -> KeyBinding {
        KeyBinding::new(key, action, Some(CONTEXT))
    }
    fn field<A: gpui::Action>(key: &str, action: A) -> KeyBinding {
        KeyBinding::new(key, action, Some(FIELD_CONTEXT))
    }
    vec![
        panel("up", DbUp),
        panel("down", DbDown),
        panel("left", DbOut),
        panel("right", DbInto),
        panel("enter", DbEnter),
        panel("escape", DbDismiss),
        panel("tab", DbFocusFilter),
        panel("shift-tab", DbFocusFilter),
        field("down", DbFocusTree),
        field("tab", DbFocusTree),
        field("enter", DbEnter),
        field("escape", DbDismiss),
        KeyBinding::new("enter", DbPasswordSubmit, Some(PASSWORD_CONTEXT)),
        KeyBinding::new("enter", DbPasswordSubmit, Some(PASSWORD_FIELD_CONTEXT)),
    ]
}

fn answer(this: &mut AppState, key: DbKey, window: &mut Window, cx: &mut Context<AppState>) {
    if !this.press_db_key(key, window, cx) {
        cx.propagate();
    }
}

/// The panel body. Called by the dock's adapter for `PanelKind::DbExplorer`.
pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let header = panel_header(
        "Databases",
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(icon_button_tip(
                "db-add",
                IconName::Plus,
                "Add connection",
                true,
                cx.listener(|this, _, window, cx| this.open_db_conn_form(None, window, cx)),
            ))
            .child(icon_button_tip(
                "db-settings",
                IconName::Settings,
                "Database settings",
                true,
                cx.listener(|this, _, _, cx| this.open_db_settings(cx)),
            )),
    );

    let body = panel().border_r_1().border_color(theme::border());
    let Some(db) = app.db(cx) else {
        return body.child(header).into_any_element();
    };
    // Waiting for the list is not the same as an empty one: drawing the entry point over a list
    // that has not arrived would offer to add what may already be there.
    if !db.loaded {
        return body.child(header).into_any_element();
    }
    if db.connections.is_empty() {
        return body.child(header).child(add_page(cx)).into_any_element();
    }

    // The tree is the densest of the content surfaces, which is what `Role::Dense` names.
    let font = f32::from(theme::font(Family::Content, Role::Dense));
    let filter = db.tree.filter.clone();
    let shown = visible(&db.tree.roots, &filter);
    let nothing = shown.is_empty() && !filter.trim().is_empty();
    let rows: Vec<AnyElement> = shown
        .iter()
        .enumerate()
        .map(|(ix, row)| line(ix, row, db, font, cx))
        .collect();

    let mut body = body
        .id("db-explorer")
        .key_context(CONTEXT)
        .on_action(cx.listener(|this, _: &DbUp, window, cx| answer(this, DbKey::Up, window, cx)))
        .on_action(
            cx.listener(|this, _: &DbDown, window, cx| answer(this, DbKey::Down, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &DbOut, window, cx| answer(this, DbKey::Left, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &DbInto, window, cx| answer(this, DbKey::Right, window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &DbEnter, window, cx| answer(this, DbKey::Enter, window, cx)),
        )
        .on_action(cx.listener(|this, _: &DbDismiss, window, cx| {
            answer(this, DbKey::Dismiss, window, cx)
        }))
        .on_action(cx.listener(|this, _: &DbFocusTree, window, cx| {
            this.focus_db_tree(window, cx)
        }))
        .on_action(cx.listener(|this, _: &DbFocusFilter, window, cx| {
            this.focus_db_filter(window, cx)
        }))
        .child(header)
        .child(filter_bar(
            Input::new(&app.db_filter).appearance(false),
            div(),
            false,
        ))
        .child(
            div()
                .id("db-tree")
                // The tree is a focus of its own, separate from the filter above it. Every key
                // the panel binds is live only from here.
                .when_some(db.tree.focus.as_ref(), |this, focus| this.track_focus(focus))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .track_scroll(&db.tree.scroll)
                .children(nothing.then(|| {
                    div()
                        .px_3()
                        .py_2()
                        .text_size(theme::font(Family::Chrome, Role::Label))
                        .text_color(theme::text_faint())
                        .child("Nothing matches")
                }))
                .children(rows),
        );

    // The menu the project explorer draws, drawn here: same kit panel, same index-into-`entries`
    // pick, same epoch-carrying dismissal.
    if app.workbench.open_menu == Some(MenuId::Db)
        && let Some(menu) = db.menu.clone()
    {
        let epoch = menu.epoch;
        let items: Vec<ContextItem> = menu
            .entries()
            .into_iter()
            .map(|action| match action.is_separator() {
                true => ContextItem::separator(),
                false => ContextItem::new(action.label()),
            })
            .collect();
        body = body.child(context_menu(
            "db-menu",
            point(px(menu.x), px(menu.y)),
            items,
            crate::ui::indexed(&cx.entity(), |this, index, window, cx| {
                this.pick_db_action(index, window, cx);
            }),
            crate::ui::handler(&cx.entity(), move |this, _, cx| {
                this.dismiss_db_menu(epoch, cx)
            }),
        ));
    }

    body.into_any_element()
}

/// The first thing a project's databases say. Filled rather than ghosted: there is one action on
/// this screen and nothing else to do until it is taken.
fn add_page(cx: &mut Context<AppState>) -> AnyElement {
    empty::empty_page(
        "No connections",
        "Add the PostgreSQL, MySQL, SQLite or SQL Server databases this project works with, and \
         browse and query them here.",
        UbiqIcon::ModeDb,
        Some(
            primary_button(
                "db-add-connection",
                Some(IconName::Plus),
                "Add connection",
                cx.listener(|this, _, _, cx| this.open_db_settings(cx)),
            )
            .into_any_element(),
        ),
    )
    .into_any_element()
}

/// The mark a row leads with, and its colour.
fn mark(node: &Node, state: &DbConnState) -> (Icon, Rgba) {
    match &node.kind {
        NodeKind::Connection => (
            Icon::new(UbiqIcon::ModeDb),
            match state {
                DbConnState::Connected { .. } => theme::success(),
                DbConnState::Connecting | DbConnState::NeedsPassword => theme::warning(),
                DbConnState::Failed(_) => theme::danger(),
                DbConnState::Idle => theme::text_faint(),
            },
        ),
        NodeKind::Database(_) => (Icon::new(IconName::HardDrive), theme::text_muted()),
        NodeKind::Schema { .. } => (Icon::new(IconName::Folder), theme::text_muted()),
        NodeKind::Group(_) => (Icon::new(IconName::Folder), theme::text_faint()),
        NodeKind::Object(obj) => match obj.kind {
            ObjectKind::Table => (Icon::new(IconName::LayoutDashboard), theme::accent()),
            ObjectKind::Synonym => (Icon::new(IconName::ExternalLink), theme::text_muted()),
            _ => (Icon::new(IconName::Eye), theme::info()),
        },
        NodeKind::Column { pk: true } => (Icon::new(IconName::Asterisk), theme::warning()),
        NodeKind::Column { pk: false } => (Icon::new(IconName::Dash), theme::text_faint()),
        NodeKind::Note => (Icon::new(IconName::Dash), theme::text_faint()),
    }
}

/// What a connection says beside its name, in the colour its state earns. Connected and idle say
/// nothing: the mark's colour already does, and a word on every row would be noise.
fn state_word(state: &DbConnState) -> Option<(&'static str, Rgba, String)> {
    match state {
        DbConnState::Connecting => Some(("connecting", theme::warning(), "connecting".into())),
        DbConnState::NeedsPassword => Some((
            "password needed",
            theme::warning(),
            "the connection needs a password".into(),
        )),
        DbConnState::Failed(failure) => Some(("failed", theme::danger(), failure.message.clone())),
        DbConnState::Connected { .. } | DbConnState::Idle => None,
    }
}

/// One row of the flattened tree.
fn line(
    ix: usize,
    row: &Row<'_>,
    db: &DbState,
    font: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let node = row.node;
    let conn = node.conn;
    let state = db.state_of(conn).clone();
    let selected = db.tree.selected.as_deref() == Some(node.id.as_str());

    let click_id = node.id.clone();
    let menu_id = node.id.clone();
    let mut line = file_row(("db-row", ix), row.depth, selected, false, false, font)
        .on_click(
            cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_db_row(conn, click_id.clone(), event.click_count(), window, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_db_menu(
                    conn,
                    menu_id.clone(),
                    (f32::from(event.position.x), f32::from(event.position.y)),
                    cx,
                );
            }),
        );

    line = match node.expandable() {
        true => {
            let toggled = node.id.clone();
            line.child(twisty(
                ("db-twisty", ix),
                row.open,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_db_node(conn, toggled.clone(), cx);
                }),
            ))
        }
        // Keeps a column's label under its table's, where the twisty would be.
        false => line.child(div().size(px(16.)).flex_none()),
    };

    let (icon, tint) = mark(node, &state);
    line = line.child(icon.with_size(Size::XSmall).text_color(tint));

    let failed = match &node.load {
        Load::Failed(message) => Some(message.clone()),
        _ => None,
    };
    let (text, tooltip) = match (&node.kind, &failed, &state) {
        (_, Some(message), _) => (theme::danger(), message.clone()),
        (NodeKind::Connection, _, DbConnState::Connected { server }) if !server.is_empty() => {
            (theme::text(), format!("{} \u{b7} {server}", node.label))
        }
        (NodeKind::Column { .. } | NodeKind::Group(_) | NodeKind::Note, _, _) => {
            (theme::text_muted(), node.label.clone())
        }
        _ => (theme::text(), node.label.clone()),
    };
    line = line.child(elided_with(
        ("db-name", ix),
        node.label.clone(),
        tooltip,
        text,
        px(font),
    ));

    let meta = theme::font(Family::Content, Role::Meta);
    if node.load == Load::Loading {
        line = line.child(
            div()
                .flex_none()
                .text_size(meta)
                .text_color(theme::text_faint())
                .child("\u{2026}"),
        );
    }
    if !node.detail.is_empty() {
        line = line.child(
            div()
                .flex_none()
                .text_size(meta)
                .text_color(theme::text_faint())
                .child(node.detail.clone()),
        );
    }

    if matches!(node.kind, NodeKind::Connection) {
        if let Some((word, colour, why)) = state_word(&state) {
            line = line.child(
                elided_with(
                    ("db-state", ix),
                    word,
                    why,
                    colour,
                    theme::font(Family::Chrome, Role::Meta),
                )
                .flex_none(),
            );
        }
        if db.is_read_only(conn) {
            line = line.child(
                div()
                    .flex_none()
                    .px_1()
                    .bg(theme::db_read_only_soft())
                    .child(badge("read-only", theme::db_read_only())),
            );
        }
        if matches!(state, DbConnState::Failed(_)) {
            line = line.child(icon_button_tip(
                ("db-retry", ix),
                IconName::RotateCw,
                "Retry",
                true,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.connect_db(conn, cx);
                }),
            ));
        }
    }

    // The estimate, pushed to the far end by the label's own `flex_1`.
    if let NodeKind::Object(obj) = &node.kind
        && let Some(count) = approx_label(obj)
    {
        line = line.child(
            div()
                .id(("db-count", ix))
                .flex_none()
                .text_size(meta)
                .text_color(theme::text_faint())
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new("approximate (from statistics)")
                        .build(window, cx)
                })
                .child(count),
        );
    }

    line.into_any_element()
}

/// The question a connection raised by answering `NeedsPassword`: the password, and whether to
/// keep it. Painted at the window root over whatever raised the connection.
pub fn password_prompt(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(db) = app.db(cx) else {
        return div().into_any_element();
    };
    let (Some(prompt), Some(input)) = (db.password_prompt.clone(), db.tree.prompt_input.clone())
    else {
        // The frame between the prompt arriving and `build_db_widgets` making its field.
        return div().into_any_element();
    };
    let name = db
        .connection(prompt.conn)
        .map(|conn| conn.config.name.clone())
        .unwrap_or_default();
    let sealable = !matches!(db.keystore, Some(DbKeystore::Unavailable(_)));
    let view = cx.entity();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);

    let mut body = div()
        .key_context(PASSWORD_CONTEXT)
        .on_action(cx.listener(|this, _: &DbPasswordSubmit, window, cx| {
            this.submit_db_password(window, cx)
        }))
        .flex()
        .flex_col()
        .gap_2()
        .pt_3()
        .child(crate::ui::kit::modal_note(
            "The connection needs a password Ubiq does not hold. It is sent to the host and used \
             to connect; it is never shown again.",
        ))
        .child(
            crate::ui::kit::field(theme::border(), focused)
                .h(px(30.))
                .px_2()
                .child(Input::new(&input).appearance(false).mask_toggle()),
        );
    body = match sealable {
        true => body.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(check_box(
                    "db-password-remember",
                    prompt.remember,
                    cx.listener(|this, _, _, cx| this.toggle_db_password_remember(cx)),
                ))
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child("Remember it on this machine"),
                ),
        ),
        false => body.child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::warning())
                .child("This install cannot keep a password; it is held for this run only."),
        ),
    };

    let footer = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .justify_end()
        .gap_2()
        .child(crate::ui::kit::ghost_button(
            "db-password-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.cancel_db_password(cx)),
        ))
        .child(primary_button(
            "db-password-confirm",
            None,
            "Connect",
            cx.listener(|this, _, window, cx| this.submit_db_password(window, cx)),
        ))
        .into_any_element();

    modal(
        "db-password",
        theme::accent(),
        &format!("Password for {name}"),
        body.into_any_element(),
        footer,
        crate::ui::dismiss(&view, Layer::DbPassword, |this, _, cx| {
            this.cancel_db_password(cx)
        }),
        window,
    )
}
