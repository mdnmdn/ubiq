//! The five questions a file gesture asks.
//!
//! Painted from the window's root rather than from the explorer, because one of them — the
//! untitled buffer's save-as — is the editor's and neither panel should have to know about the
//! other's. Which one is up is `WorkbenchState::file_dialog`, and what is typed into any of them
//! is the window's one `file_name` field.
//!
//! Every one of them is kit calls. The only hand-rolled body is the folder move's, because it is
//! the one dialog with a control in it: a tick box that stops it asking again for ten minutes.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, px, relative};

use crate::app::AppState;
use crate::state::FileDialog;
use crate::theme;
use crate::ui::kit::{
    check_box, confirm_modal, ghost_button, modal, modal_note, primary_button, prompt_modal,
};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let typed = app.file_name.read(cx).value().trim().to_string();

    match app.workbench.file_dialog.clone() {
        None => div().into_any_element(),
        Some(FileDialog::New { parent, dir }) => {
            let where_ = match parent.is_empty() {
                true => "the project's top level".to_string(),
                false => parent,
            };
            prompt_modal(
                "app-file-new",
                if dir { "New folder" } else { "New file" },
                Some(&format!("It is made in {where_}.")),
                "Name",
                &app.file_name,
                "Create",
                !typed.is_empty(),
                crate::ui::handler(&view, |this, _, cx| this.confirm_file_dialog(cx)),
                crate::ui::handler(&view, |this, _, cx| this.close_file_dialog(cx)),
                window,
                cx,
            )
        }
        Some(FileDialog::Rename { path }) => {
            let leaf = leaf_of(&path).to_string();
            prompt_modal(
                "app-file-rename",
                "Rename",
                Some("Anything open on it follows the new name."),
                "Name",
                &app.file_name,
                "Rename",
                !typed.is_empty() && typed != leaf,
                crate::ui::handler(&view, |this, _, cx| this.confirm_file_dialog(cx)),
                crate::ui::handler(&view, |this, _, cx| this.close_file_dialog(cx)),
                window,
                cx,
            )
        }
        Some(FileDialog::SaveAs { .. }) => prompt_modal(
            "app-file-save-as",
            "Save as",
            Some("Where in the project this buffer is written. Nothing there is overwritten."),
            "Path",
            &app.file_name,
            "Save",
            !typed.is_empty(),
            crate::ui::handler(&view, |this, _, cx| this.confirm_file_dialog(cx)),
            crate::ui::handler(&view, |this, _, cx| this.close_file_dialog(cx)),
            window,
            cx,
        ),
        Some(FileDialog::Remove { path, dir, trash }) => {
            let contents = match dir {
                true => " Everything inside it goes too.",
                false => "",
            };
            confirm_modal(
                "app-file-remove",
                if trash { "Move to Trash" } else { "Delete" },
                &format!("{path}?{contents}"),
                if trash {
                    "Move to Trash"
                } else {
                    "Delete permanently"
                },
                true,
                crate::ui::handler(&view, |this, _, cx| this.confirm_file_dialog(cx)),
                crate::ui::handler(&view, |this, _, cx| this.close_file_dialog(cx)),
                window,
            )
        }
        Some(FileDialog::Move { path, into }) => {
            let unasked = app.workbench.move_unasked_until.is_some();
            let target = match into.is_empty() {
                true => "the project's top level".to_string(),
                false => into,
            };
            let body = div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_3()
                .child(modal_note(&format!(
                    "Move {path} into {target}? Everything inside it moves with it."
                )))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(check_box(
                            "app-file-move-unasked",
                            unasked,
                            cx.listener(|this, _, _, cx| this.toggle_move_unasked(cx)),
                        ))
                        .child(
                            div()
                                .w(relative(1.))
                                .text_size(px(12.5))
                                .text_color(theme::text_muted())
                                .child("Don't ask again for 10 minutes"),
                        ),
                )
                .into_any_element();

            let footer = div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "app-file-move-cancel",
                    None,
                    "Cancel",
                    cx.listener(|this, _, _, cx| this.close_file_dialog(cx)),
                ))
                .child(primary_button(
                    "app-file-move-confirm",
                    None,
                    "Move",
                    cx.listener(|this, _, _, cx| this.confirm_file_dialog(cx)),
                ))
                .into_any_element();

            modal(
                "app-file-move",
                theme::accent(),
                "Move folder",
                body,
                footer,
                crate::ui::handler(&view, |this, _, cx| this.close_file_dialog(cx)),
                window,
            )
        }
    }
}

fn leaf_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((_, leaf)) => leaf,
        None => path,
    }
}
