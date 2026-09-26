//! The "Add source" modal: every question a new knowledge base source answers, in one place.
//!
//! **The New agent modal's shape, deliberately.** A `kit::modal` with a title and a footer row, a
//! first full-width [`Picker`] that everything below it is downstream of, and every row under that
//! drawn **inert** — faint and taking no click — until the first is answered. Hidden rows would
//! make the form's shape jump as it is filled in, which is a form the reader has to re-read after
//! every answer; `crate::ui::new_agent` states the rule and this follows it.
//!
//! **The folder field is answered by Ubiq's own picker, never the platform's.** A source's folder
//! lives on the machine the *host* runs on, which the window's own chooser cannot see past — so
//! the icon beside the field raises the host-backed dialog `crate::app::host_browse` drives. The
//! field itself is read-only: the path is the host's word for it, not something to be typed.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, ElementId, Entity, Focusable, IntoElement, ParentElement, Styled,
    Window, div, px,
};
use gpui_component::IconName;
use gpui_component::input::Input;

use ubiq_proto::kb::{KbAccess, KbStore};

use crate::app::AppState;
use crate::state::kb::{KbKind, KbList, KbSourceForm, KbUrlCheck};
use crate::state::navigator::subsequence;
use crate::state::overlay::Layer;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    Picker, PickerStyle, elided, ghost_button, hint_row, label_hint, primary_button, slab,
};
use crate::ui::sink::style::{framed_active, input_on};
use crate::ui::{handler, indexed};

/// How wide every control in the right-hand column is drawn. One width for all of them, so the
/// column reads as a column rather than as a ragged edge — `ui::new_agent`'s own constant, said
/// again here because the kit knows nothing about either form.
const CONTROL_WIDTH: f32 = 230.;

/// The modal. Its body is [`body`]; only the title and the footer are its own.
pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(form) = app.workbench.kb_source.clone() else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let ready = form.is_answerable();

    let footer = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .justify_end()
        .gap_2()
        .child(ghost_button(
            "kb-source-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_kb_source_form(cx)),
        ))
        .child(
            primary_button(
                "kb-source-confirm",
                None,
                "Add source",
                cx.listener(|this, _, _, cx| this.confirm_kb_source(cx)),
            )
            .when(!ready, |button| button.opacity(0.5)),
        )
        .into_any_element();

    crate::ui::kit::modal(
        "kb-source",
        theme::accent(),
        "Add a source",
        body(app, &form, window, cx),
        footer,
        // Its own pickers' lists and the folder dialog it raises are both painted over it, so an
        // outside click while either is up is theirs: `ui::dismiss` yields to whatever sits above
        // this rung.
        crate::ui::dismiss(&view, Layer::KbSource, |this, _, cx| {
            this.close_kb_source_form(cx)
        }),
        window,
    )
}

/// The rows, top to bottom: what it is called, what kind it is, the one thing that kind needs, and
/// what may be written to it.
fn body(
    app: &AppState,
    form: &KbSourceForm,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let live = form.kind.is_some();

    let mut rows = div().flex().flex_col().gap_1().pt_1();

    // 1 ─ what the explorer's top-level row will say. Above the kind rather than below it because
    // it is the one answer every kind has, and because it is seeded from the others: a reader who
    // watches it fill itself in learns what the seeding rule is without being told.
    rows = rows.child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pb_2()
            .child(label_hint(
                "kb-source-name-hint",
                "Name",
                "What the knowledge base explorer calls this source. Filled in from the folder or \
                 the repository until you type your own.",
            ))
            .child(
                framed_active(theme::border(), input_on(&app.kb_name_input, window, cx))
                    .h(px(28.))
                    .items_center()
                    .child(Input::new(&app.kb_name_input).appearance(false)),
            ),
    );

    // 2 ─ the question everything below is downstream of, drawn as a question of its own: its
    // label above it, the control the full width of the body, and a gap under it that says the
    // rest is a consequence of this answer.
    let kind_rows: Vec<(String, Option<KbKind>)> = [KbKind::Folder, KbKind::Git, KbKind::Wiki]
        .into_iter()
        .map(|kind| (kind.label().to_string(), Some(kind)))
        .collect();
    rows = rows.child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pb_2()
            .child(label_hint(
                "kb-source-kind-hint",
                "Kind",
                "A folder is read where it lies; a repository is cloned and refreshed by Ubiq; a \
                 wiki is Ubiq's own, and starts empty.",
            ))
            .child(picker_of(
                app,
                &view,
                form,
                "kb-source-kind",
                "Choose\u{2026}",
                kind_rows,
                form.kind,
                KbList::Kind,
                true,
                window,
                cx,
                |this, kind, window, cx| this.pick_kb_source_kind(kind, window, cx),
            )),
    );

    // 3 ─ the folder, for a folder source. Read-only and browsed rather than typed: the path is
    // the host's word for a place on the host's disk, and a typed one the host cannot resolve is
    // a source that fails after it is saved rather than before.
    if form.kind == Some(KbKind::Folder) {
        let path = form.path.clone();
        rows = rows.child(hint_row(
            "kb-source-folder-hint",
            "Folder",
            "Chosen on the machine the host runs on \u{2014} not this one, unless they are the \
             same machine.",
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .w(px(CONTROL_WIDTH))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.))
                        .h(px(26.))
                        .px_2()
                        .items_center()
                        .bg(theme::surface())
                        .border_l(px(theme::accent_edge()))
                        .border_color(theme::border())
                        .child(match path {
                            Some(path) => elided(
                                "kb-source-folder-path",
                                path,
                                theme::text(),
                                theme::font(Family::Chrome, Role::Label),
                            )
                            .into_any_element(),
                            None => div()
                                .text_size(theme::font(Family::Chrome, Role::Label))
                                .text_color(theme::text_faint())
                                .child("No folder chosen")
                                .into_any_element(),
                        }),
                )
                .child(crate::ui::kit::icon_button(
                    "kb-source-browse",
                    IconName::FolderOpen,
                    false,
                    cx.listener(|this, _, window, cx| this.browse_kb_source_folder(window, cx)),
                ))
                .into_any_element(),
        ));
    }

    // 4 ─ the three questions a repository asks, in a block of their own: where it is, which
    // branch, and where the checkout is kept. A block is what says they are one answer.
    if form.kind == Some(KbKind::Git) {
        let mut repo = slab(theme::border()).px_2().py_1().gap_1();

        repo = repo.child(hint_row(
            "kb-source-url-hint",
            "Repository URL",
            "Checked before it is saved: Ubiq asks the host for its branches, and an answer of any \
             kind is what says the URL is reachable.",
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .w(px(CONTROL_WIDTH))
                .child(
                    framed_active(theme::border(), input_on(&app.kb_url_input, window, cx))
                        .h(px(26.))
                        .flex_1()
                        .min_w(px(0.))
                        .items_center()
                        .child(Input::new(&app.kb_url_input).appearance(false)),
                )
                .child(ghost_button(
                    "kb-source-check",
                    None,
                    "Check",
                    cx.listener(|this, _, _, cx| this.check_kb_source_url(cx)),
                ))
                .into_any_element(),
        ));

        // What the check came back with, said in the status group's own colours — a count is not
        // a success and an error is not a warning unless the colour says so.
        if let Some((word, colour)) = check_line(&form.check) {
            repo = repo.child(
                div()
                    .pb_1()
                    .child(elided(
                        "kb-source-check-note",
                        word,
                        colour,
                        theme::font(Family::Chrome, Role::Meta),
                    ))
                    .into_any_element(),
            );
        }

        let checked = matches!(form.check, KbUrlCheck::Checked(_));
        let mut branch_rows: Vec<(String, Option<Option<String>>)> =
            vec![("The repository's default".to_string(), Some(None))];
        branch_rows.extend(
            form.branches
                .iter()
                .map(|branch| (branch.clone(), Some(Some(branch.clone())))),
        );
        repo = repo.child(picker_row(
            app,
            &view,
            form,
            "kb-source-branch",
            "Branch",
            "Which branch the checkout follows. Answered by the check above.",
            "The repository's default",
            branch_rows,
            Some(form.branch.clone()),
            KbList::Branch,
            checked,
            window,
            cx,
            |this, branch, _window, cx| this.pick_kb_source_branch(branch, cx),
        ));

        let store_rows: Vec<(String, Option<KbStore>)> = vec![
            ("Cache".to_string(), Some(KbStore::Cache)),
            ("Ubiq's project folder".to_string(), Some(KbStore::Internal)),
            ("Inside the project".to_string(), Some(KbStore::Project)),
        ];
        repo = repo.child(picker_row(
            app,
            &view,
            form,
            "kb-source-store",
            "Store",
            "Cache is a temporary folder, refetched when the system sweeps it. Ubiq's project \
             folder is inside Ubiq's own, and is the default. Inside the project is a `.ubiq/local/kb` \
             folder in the project itself.",
            "Ubiq's project folder",
            store_rows,
            Some(form.store),
            KbList::Store,
            live,
            window,
            cx,
            |this, store, _window, cx| this.pick_kb_source_store(store, cx),
        ));
        rows = rows.child(repo);
    }

    // 5 ─ what Ubiq may do to it. A picker rather than a checkbox, because it sits in the same
    // column as the rest and "Read only" reads better as a stated answer than as an unticked box.
    // A wiki is fixed on read-write: there is nothing fetched or user-picked under it to protect.
    let wiki = form.kind == Some(KbKind::Wiki);
    let access_rows: Vec<(String, Option<KbAccess>)> = vec![
        ("Read only".to_string(), Some(KbAccess::ReadOnly)),
        ("Read and write".to_string(), Some(KbAccess::ReadWrite)),
    ];
    rows = rows.child(picker_row(
        app,
        &view,
        form,
        "kb-source-access",
        "Access",
        match wiki {
            true => {
                "A wiki is always read-write: it is Ubiq's own, and nothing outside Ubiq \
                     writes to it."
            }
            false => "Whether documents in this source may be edited through Ubiq.",
        },
        "Read only",
        access_rows,
        Some(form.access()),
        KbList::Access,
        live && !wiki,
        window,
        cx,
        |this, access, _window, cx| this.pick_kb_source_access(access, cx),
    ));

    rows.into_any_element()
}

/// What the check came back with, as the line under the URL field: a colour from the status group
/// and the words that colour is about. `Idle` says nothing — a field nobody has checked yet is not
/// a state worth a sentence.
fn check_line(check: &KbUrlCheck) -> Option<(String, gpui::Rgba)> {
    match check {
        KbUrlCheck::Idle => None,
        KbUrlCheck::Checking => Some(("Checking\u{2026}".to_string(), theme::text_faint())),
        KbUrlCheck::Checked(0) => Some((
            "Reached, and it has no branches yet.".to_string(),
            theme::warning(),
        )),
        KbUrlCheck::Checked(1) => Some(("Reached \u{b7} 1 branch".to_string(), theme::success())),
        KbUrlCheck::Checked(count) => {
            Some((format!("Reached \u{b7} {count} branches"), theme::success()))
        }
        KbUrlCheck::Failed(error) => Some((error.clone(), theme::danger())),
    }
}

/// One labelled row holding one dropdown.
///
/// A row with `enabled: false` is drawn faint and wired to nothing, which is what "inert" means
/// here — the list cannot open, so it cannot be picked from either.
#[allow(clippy::too_many_arguments)]
fn picker_row<T: Clone + PartialEq + 'static>(
    app: &AppState,
    view: &Entity<AppState>,
    form: &KbSourceForm,
    id: &'static str,
    label: &str,
    note: &str,
    placeholder: &str,
    rows: Vec<(String, Option<T>)>,
    chosen: Option<T>,
    list: KbList,
    enabled: bool,
    window: &Window,
    cx: &App,
    pick: impl Fn(&mut AppState, T, &mut Window, &mut Context<AppState>) + 'static,
) -> AnyElement {
    let picker = picker_of(
        app,
        view,
        form,
        id,
        placeholder,
        rows,
        chosen,
        list,
        enabled,
        window,
        cx,
        pick,
    );
    div()
        .when(!enabled, |this| this.opacity(0.5))
        .child(hint_row(
            ElementId::Name(format!("{id}-hint").into()),
            label,
            note,
            div()
                .flex_none()
                .w(px(CONTROL_WIDTH))
                .child(picker)
                .into_any_element(),
        ))
        .into_any_element()
}

/// The dropdown itself, without the row around it — what the kind question draws at the body's
/// full width and what [`picker_row`] draws in the right-hand column.
#[allow(clippy::too_many_arguments)]
fn picker_of<T: Clone + PartialEq + 'static>(
    app: &AppState,
    view: &Entity<AppState>,
    form: &KbSourceForm,
    id: &'static str,
    placeholder: &str,
    rows: Vec<(String, Option<T>)>,
    chosen: Option<T>,
    list: KbList,
    enabled: bool,
    window: &Window,
    cx: &App,
    pick: impl Fn(&mut AppState, T, &mut Window, &mut Context<AppState>) + 'static,
) -> Picker {
    let open = enabled && form.open == Some(list);
    let trigger = rows
        .iter()
        .find(|(_, value)| value.is_some() && *value == chosen)
        .map(|(label, _)| label.clone())
        .unwrap_or_else(|| placeholder.to_string());

    let needle = app.picker_search.read(cx).value().trim().to_lowercase();
    let shown: Vec<(String, Option<T>)> = if open && !needle.is_empty() {
        rows.into_iter()
            .filter(|(label, value)| value.is_some() && subsequence(&needle, label))
            .collect()
    } else {
        rows
    };

    let selected = shown
        .iter()
        .position(|(_, value)| value.is_some() && *value == chosen);
    let values: Vec<Option<T>> = shown.iter().map(|(_, value)| value.clone()).collect();
    let labels: Vec<String> = shown.into_iter().map(|(label, _)| label).collect();

    let mut picker = Picker::new(ElementId::Name(id.into()), trigger)
        .items(labels)
        .open(open)
        .style(PickerStyle::Field)
        .above_modal();
    if let Some(selected) = selected {
        picker = picker.selected(selected);
    }
    if enabled {
        let focused = app
            .picker_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        picker = picker
            .search(&app.picker_search, focused)
            .on_toggle(handler(view, move |this, window, cx| {
                this.toggle_kb_source_list(list, window, cx)
            }))
            .on_pick(indexed(view, move |this, index, window, cx| {
                if let Some(Some(value)) = values.get(index).cloned() {
                    pick(this, value, window, cx);
                }
            }))
            .on_dismiss(handler(view, |this, _window, cx| {
                this.dismiss_kb_source_list(cx)
            }));
    }
    picker
}
