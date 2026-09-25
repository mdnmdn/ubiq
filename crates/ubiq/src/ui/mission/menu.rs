//! The mission `⋯` — complete, abandon, pause all, open on board, open on Teams, execution mode
//! (§6.1).
//!
//! **Every row is drawn, and most of them are dead.** `state::mission::MissionMenuRow` is the
//! list and says which ones this build has a message behind; the rest carry a tooltip saying what
//! they are waiting on rather than being left out, so the menu keeps its shape as the stages land.
//!
//! Drawn as a child of the panel that raised it, the explorer's own way: `kit::context_menu` is
//! `deferred` and `anchored`, so a menu opened inside a dock panel covers the window instead of
//! being clipped to it.

use gpui::{AnyElement, Context, IntoElement, point, px};

use ubiq_proto::ids::TaskId;

use crate::app::AppState;
use crate::state::mission::{MissionMenuRow, MissionSpawnRow, SpawnStage};
use crate::ui::{self, kit};

/// The open menu for this mission, or nothing — either because no menu is down or because the one
/// that is belongs to another mission panel.
pub fn overlay(app: &AppState, task_id: TaskId, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let (open, at) = app.workbench.mission_menu?;
    if open != task_id {
        return None;
    }

    let items: Vec<_> = MissionMenuRow::all()
        .into_iter()
        .map(|row| {
            let mut item = kit::ContextItem::new(row.label());
            if !row.enabled() {
                item = item.disabled();
            }
            if let Some(note) = row.note() {
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

/// The *Spawn ▾* menu (§6.1), in whichever of its two stages it is on.
///
/// **Rows are matched by position**, against the very list `AppState::mission_spawn_rows` /
/// `mission_attach_candidates` answer — the click resolves against the same call this draws, so
/// the two can never be two readings of one question.
pub fn spawn_overlay(
    app: &AppState,
    task_id: TaskId,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let menu = app.workbench.mission_spawn_menu?;
    if menu.task != task_id {
        return None;
    }

    let items: Vec<_> = match menu.stage {
        SpawnStage::Kinds => app
            .mission_spawn_rows(task_id)
            .into_iter()
            .map(|row| match row {
                MissionSpawnRow::Coordinator => kit::ContextItem::new("Coordinator")
                    .tooltip("Launches a coordinator and crowns it. A mission has one at a time."),
                MissionSpawnRow::Kind(name) => kit::ContextItem::new(name),
                MissionSpawnRow::Any => kit::ContextItem::new("Any agent\u{2026}")
                    .tooltip("The New agent form, pre-filled with this mission."),
                MissionSpawnRow::Attach => kit::ContextItem::new("Attach running agent\u{2026}"),
            })
            .collect(),
        SpawnStage::Attach => {
            let rows: Vec<_> = app
                .mission_attach_candidates(task_id, cx)
                .into_iter()
                .filter_map(|id| app.work(cx).and_then(|work| work.agent(id)).cloned())
                .map(|agent| kit::ContextItem::new(app.agent_title(&agent).to_string()))
                .collect();
            match rows.is_empty() {
                true => vec![
                    kit::ContextItem::new("Nothing running to attach")
                        .disabled()
                        .tooltip("Every agent this window is holding is already on the mission."),
                ],
                false => rows,
            }
        }
    };

    let view = cx.entity();
    Some(
        kit::context_menu(
            ui::eid("mission-spawn-menu", task_id),
            point(px(menu.at.0), px(menu.at.1)),
            items,
            ui::indexed(&view, |this, index, window, cx| {
                this.pick_mission_spawn_menu(index, window, cx);
            }),
            ui::handler(&view, |this, _, cx| this.dismiss_mission_spawn_menu(cx)),
        )
        .into_any_element(),
    )
}

/// The kind picker: what a pending spawn will be launched as, or what a kinds-table row resolves
/// to. One menu for both, because it is one question — see `state::mission::KindPick`.
pub fn kind_overlay(
    app: &AppState,
    task_id: TaskId,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let (open, target, at) = app.workbench.mission_kind_menu?;
    if open != task_id {
        return None;
    }

    let picks = app.mission_kind_picks(task_id, target);
    let items: Vec<_> = match picks.is_empty() {
        true => vec![
            kit::ContextItem::new("No agents to choose from")
                .disabled()
                .tooltip("Write an agent in settings and it will be offered here."),
        ],
        false => picks
            .iter()
            .map(|pick| kit::ContextItem::new(pick.label()))
            .collect(),
    };

    let view = cx.entity();
    Some(
        kit::context_menu(
            ui::eid("mission-kind-menu", task_id),
            point(px(at.0), px(at.1)),
            items,
            ui::indexed(&view, |this, index, _window, cx| {
                this.pick_mission_kind_menu(index, cx);
            }),
            ui::handler(&view, |this, _, cx| this.dismiss_mission_kind_menu(cx)),
        )
        .into_any_element(),
    )
}
