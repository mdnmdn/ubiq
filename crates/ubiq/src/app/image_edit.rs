//! What the image toolbar and its gestures do to the capture behind the tab.
//!
//! Every handler is a no-op on a tab that is not a session-authored capture, so the same
//! toolbar on a read-only image fails closed rather than failing loud.

use super::*;
use crate::state::image_edit::{ImageTool, Pending, ShapeKind};

impl AppState {
    /// The open tab `key` names in the project on screen, when there is one.
    fn image_file_mut(&mut self, key: &str, cx: &mut Context<Self>) -> Option<&mut OpenFile> {
        let project = self.project(cx)?;
        self.projects.get_mut(&project)?.editor.find_key_mut(key)
    }

    /// Show or hide a picture's annotation tools. Viewing drops the selection, so returning to
    /// Edit starts where a fresh one does; nothing about the scene changes either way.
    pub fn set_image_editing(&mut self, key: &str, editing: bool, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        file.image_editing = editing;
        if !editing && let Some(edit) = file.ensure_image_edit() {
            edit.selected = None;
            edit.pending = None;
        }
        cx.notify();
    }

    /// Pick up the tool. A selection does not follow across tools that draw.
    pub fn set_image_tool(&mut self, key: &str, tool: ImageTool, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        edit.tool = tool;
        edit.pending = None;
        cx.notify();
    }

    /// What the stroke picker answered, on whichever capture is active — and on the selected
    /// element, as one undoable step.
    pub fn set_active_image_stroke(&mut self, colour: gpui::Hsla, cx: &mut Context<Self>) {
        let Some(key) = self.active_file_key(cx) else {
            return;
        };
        let Some(file) = self.image_file_mut(&key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        edit.stroke = Some(crate::ui::viewer::image_edit::stroke_rgba8(colour));
        if edit.fill.is_some() {
            edit.fill = edit.stroke;
        }
        if edit.restyle_selected() {
            file.touch_image();
        }
        cx.notify();
    }

    /// The width the next element takes — and the selected one's, as one undoable step.
    pub fn cycle_image_width(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        let at = crate::state::image_edit::WIDTHS
            .iter()
            .position(|width| *width == edit.stroke_width)
            .unwrap_or(0);
        edit.stroke_width =
            crate::state::image_edit::WIDTHS[(at + 1) % crate::state::image_edit::WIDTHS.len()];
        if edit.restyle_selected() {
            file.touch_image();
        }
        cx.notify();
    }

    /// Whether new elements carry the stroke colour as a fill — and the selected one's.
    pub fn toggle_image_fill(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        edit.fill = match edit.fill {
            Some(_) => None,
            None => edit.stroke,
        };
        if edit.restyle_selected() {
            file.touch_image();
        }
        cx.notify();
    }

    /// Drop the selection. The toolbar's button; there is no key for it.
    pub fn delete_image_selection(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        if edit.delete_selected() {
            file.touch_image();
            cx.notify();
        }
    }

    /// Undo on the active tab's capture. Bound at Workbench depth, so a text buffer's own undo
    /// wins while one holds the keyboard and this only ever sees an image tab.
    pub fn undo_image_action(&mut self, _: &ImageUndo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active_file_key(cx) else {
            return;
        };
        self.undo_image(&key, cx);
    }

    /// Redo on the active tab's capture, by the same device.
    pub fn redo_image_action(&mut self, _: &ImageRedo, _: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.active_file_key(cx) else {
            return;
        };
        self.redo_image(&key, cx);
    }

    fn active_file_key(&self, cx: &App) -> Option<String> {
        self.editor(cx)?.active_file().map(|file| file.key())
    }

    pub fn undo_image(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        if edit.undo() {
            file.touch_image();
            cx.notify();
        }
    }

    pub fn redo_image(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        if edit.redo() {
            file.touch_image();
            cx.notify();
        }
    }

    /// A press on the picture. `true` took the gesture; `false` lets it reach pan underneath.
    pub fn image_down(
        &mut self,
        key: String,
        tool: ImageTool,
        at: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let consumed = match tool {
            ImageTool::Select => {
                let Some(file) = self.image_file_mut(&key, cx) else {
                    return;
                };
                let Some(edit) = file.ensure_image_edit() else {
                    return;
                };
                match edit.hit_test(at.0, at.1) {
                    Some(id) => {
                        let index = edit
                            .scene
                            .elements
                            .iter()
                            .position(|element| element.id == id);
                        let orig =
                            index.map(|at| (edit.scene.elements[at].x, edit.scene.elements[at].y));
                        edit.selected = Some(id.clone());
                        if let Some(orig) = orig {
                            edit.pending = Some(Pending::Move { id, orig, grab: at });
                        }
                        cx.notify();
                        true
                    }
                    // Empty space with Select is a pan, so the gesture is declined and the
                    // selection stands.
                    None => false,
                }
            }
            ImageTool::Text => {
                let Some(file) = self.image_file_mut(&key, cx) else {
                    return;
                };
                let Some(edit) = file.ensure_image_edit() else {
                    return;
                };
                edit.text_at = Some(at);
                self.open_file_dialog(FileDialog::ImageText { key }, "", window, cx);
                true
            }
            ImageTool::Freehand => {
                let Some(file) = self.image_file_mut(&key, cx) else {
                    return;
                };
                let Some(edit) = file.ensure_image_edit() else {
                    return;
                };
                edit.pending = Some(Pending::Draw { points: vec![at] });
                cx.notify();
                true
            }
            ImageTool::Crop => self.start_shape(&key, ShapeKind::Crop, at, cx),
            ImageTool::Copy => self.start_shape(&key, ShapeKind::Copy, at, cx),
            ImageTool::Rectangle => self.start_shape(&key, ShapeKind::Rect, at, cx),
            ImageTool::Ellipse => self.start_shape(&key, ShapeKind::Ellipse, at, cx),
            ImageTool::Arrow => self.start_shape(&key, ShapeKind::Arrow, at, cx),
        };
        if consumed {
            cx.stop_propagation();
        }
    }

    fn start_shape(
        &mut self,
        key: &str,
        kind: ShapeKind,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(file) = self.image_file_mut(key, cx) else {
            return false;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return false;
        };
        edit.pending = Some(Pending::Shape {
            kind,
            start: at,
            cur: at,
        });
        cx.notify();
        true
    }

    /// The pointer moved over the picture. Only a drag in progress answers.
    pub fn image_move(&mut self, key: &str, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        match edit.pending.clone() {
            None => {}
            Some(Pending::Shape { kind, start, .. }) => {
                edit.pending = Some(Pending::Shape {
                    kind,
                    start,
                    cur: at,
                });
                cx.notify();
            }
            Some(Pending::Draw { mut points }) => {
                if let Some(last) = points.last()
                    && ((last.0 - at.0).powi(2) + (last.1 - at.1).powi(2)).sqrt() > 2.0
                {
                    points.push(at);
                    edit.pending = Some(Pending::Draw { points });
                    cx.notify();
                }
            }
            Some(Pending::Move { id, orig, grab }) => {
                edit.live_move(&id, at.0 - grab.0, at.1 - grab.1, orig);
                cx.notify();
            }
        }
    }

    /// The pointer let go: commit the drag as one undoable step, or copy the region.
    pub fn image_up(&mut self, key: &str, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        let pending = edit.pending.take();
        match pending {
            None => {}
            Some(Pending::Shape { kind, start, .. }) => match kind {
                ShapeKind::Copy => self.copy_region(key, start, at, cx),
                ShapeKind::Crop | ShapeKind::Rect | ShapeKind::Ellipse | ShapeKind::Arrow => {
                    if edit.commit_shape(kind, start, at) {
                        file.touch_image();
                    }
                    cx.notify();
                }
            },
            Some(Pending::Draw { points }) => {
                if edit.commit_draw(points) {
                    file.touch_image();
                }
                cx.notify();
            }
            Some(Pending::Move { id, orig, grab }) => {
                edit.live_move(&id, at.0 - grab.0, at.1 - grab.1, orig);
                edit.commit_move(&id, orig);
                file.touch_image();
                cx.notify();
            }
        }
    }

    /// Flatten the dragged rectangle and hand it to the clipboard's image arm. Phase 4 owns the
    /// read path and the paste tab; this is its write call site, over the constructor it left
    /// for exactly this.
    fn copy_region(
        &mut self,
        key: &str,
        start: (f32, f32),
        end: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let (x0, y0, x1, y1) = (
            start.0.min(end.0),
            start.1.min(end.1),
            start.0.max(end.0),
            start.1.max(end.1),
        );
        let png = self
            .image_file_mut(key, cx)
            .and_then(|file| file.ensure_image_edit())
            .and_then(|edit| edit.flatten_rect(x0, y0, x1 - x0, y1 - y0));
        if let Some(png) = png {
            cx.write_to_clipboard(clipboard::clipboard_image_item(png));
        }
        cx.notify();
    }

    /// The text dialog answered: type the string where the click was.
    pub fn commit_image_text(&mut self, key: &str, text: String, cx: &mut Context<Self>) {
        let Some(file) = self.image_file_mut(key, cx) else {
            return;
        };
        let Some(edit) = file.ensure_image_edit() else {
            return;
        };
        if let Some(at) = edit.text_at.take()
            && edit.append_text(text, at)
        {
            file.touch_image();
        }
        cx.notify();
    }
}
