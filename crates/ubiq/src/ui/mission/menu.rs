//! The mission `⋯` — pause/resume, replan, open on board, complete, abandon.
//!
//! **Every row is drawn.** `state::mission::MissionMenuRow` is the list; a row that means nothing
//! in the mission's current phase is drawn dead with a tooltip saying why, so the menu keeps its
//! shape from phase to phase.
//!
//! Drawn as a child of the panel that raised it, the explorer's own way: `kit::context_menu` is
//! `deferred` and `anchored`, so a menu opened inside a dock panel covers the window instead of
//! being clipped to it.

use gpui::{AnyElement, Context, IntoElement, point, px};

use ubiq_proto::ids::TaskId;

use crate::app::AppState;
use crate::state::mission::MissionMenuRow;
use crate::ui::{self, kit};

/// The open menu for this mission, or nothing — either because no menu is down or because the one
/// that is belongs to another mission panel.
pub fn overlay(app: &AppState, task_id: TaskId, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let (open, at) = app.workbench.mission_menu?;
    if open != task_id {
        return None;
    }

    let phase = app.mission_anywhere(task_id)?.phase;
    let items: Vec<_> = MissionMenuRow::all()
        .into_iter()
        .map(|row| {
            let mut item = kit::ContextItem::new(row.label(phase));
            if !row.enabled(phase) {
                item = item.disabled();
            }
            if let Some(note) = row.note(phase) {
                item = item.tooltip(note);
            }
            item
        })
        .collect();

    let view = cx.entity();
    Some(
        kit::context_menu(
            ui::eid("mission-menu", task_id),
            point(px(at.0), px(at.1)),
            items,
            ui::indexed(&view, |this, index, _window, cx| {
                this.pick_mission_menu(index, cx);
            }),
            ui::handler(&view, |this, _, cx| this.dismiss_mission_menu(cx)),
        )
        .into_any_element(),
    )
}
