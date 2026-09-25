//! The Settings tab: how much autonomy this mission has (§6.2, M13, M22, M23, M24).
//!
//! **Every control is one `SetMissionField`.** The window asks, the host writes, and
//! `MissionChanged` comes back — nothing here edits the record it is drawing, and nothing here
//! journals: a switch of execution mode is journaled by the host (M22), which is the only place
//! that knows it actually happened.
//!
//! **Each setting says what it does in a sentence the user can act on.** These are the knobs that
//! decide how much a mission does without asking; a mislabelled one is expensive, so the note
//! beside each says what changes about the mission's behaviour rather than restating the field
//! name.
//!
//! The agent-kinds table is [`super::full::agent_kinds`] — the same table the Agents tab draws,
//! drawn once and shown in both places, because a second copy is a second set of controls to keep
//! in step.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{ExecutionMode, MissionField, MissionRecord, OnFinish, SpawnPolicy};

use crate::app::AppState;
use crate::theme;
use crate::ui::eid2;
use crate::ui::kit::{check_box, choice_pill, setting_row};
use crate::ui::mission::panel::section_bar;

/// The whole form, in the order a mission is reasoned about: what it must clear before it runs,
/// how it runs, and what it may spawn.
pub fn render(task_id: TaskId, mission: &MissionRecord, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(section_bar("Gates", None))
        .child(rows(vec![
            setting_row(
                "Require plan approval",
                "Refining \u{2192} In progress waits for you. The gate where you commit agents and \
                 spend: with it off, the coordinator moves the mission on by itself.",
                switch(
                    task_id,
                    eid2("mission-set", task_id, "require-plan"),
                    mission.require_plan,
                    MissionField::RequirePlan,
                    cx,
                ),
            ),
            setting_row(
                "Refine automatically",
                "Requirements \u{2192} Refining moves without asking. Nothing is committed there, \
                 which is why it is on by default.",
                switch(
                    task_id,
                    eid2("mission-set", task_id, "auto-refine"),
                    mission.auto_refine,
                    MissionField::AutoRefine,
                    cx,
                ),
            ),
        ]))
        .child(section_bar("Execution", None))
        .child(rows(vec![
            setting_row(
                "Execution mode",
                "Manual: you and the coordinator hand tasks out, and readiness is advice. Auto: \
                 the scheduler assigns the mission's ready tasks itself, while it is In progress.",
                pills(
                    ("mission-execution", task_id),
                    [ExecutionMode::Manual, ExecutionMode::Auto],
                    mission.execution,
                    |mode| match mode {
                        ExecutionMode::Manual => "manual",
                        ExecutionMode::Auto => "auto",
                    },
                    MissionField::Execution,
                    cx,
                ),
            ),
            setting_row(
                "Parallelism",
                "How many of this mission's tasks the scheduler may have in flight at once in auto \
                 mode. Each one costs an agent.",
                count(
                    ("mission-parallelism", task_id),
                    mission.parallelism,
                    1,
                    12,
                    MissionField::Parallelism,
                    cx,
                ),
            ),
            setting_row(
                "When an agent finishes",
                "Reuse or stop: hand it another ready task with enough matching labels, else stop \
                 it. Stop: always stop it. Either way its conversation stays resumable.",
                pills(
                    ("mission-on-finish", task_id),
                    [OnFinish::ReuseOrStop, OnFinish::Stop],
                    mission.on_finish,
                    |on_finish| match on_finish {
                        OnFinish::ReuseOrStop => "reuse or stop",
                        OnFinish::Stop => "stop",
                    },
                    MissionField::OnFinish,
                    cx,
                ),
            ),
            setting_row(
                "Tasks per agent",
                "How many tasks one agent may be handed here before a fresh one is spawned \
                 instead \u{2014} a fresh context beats a bloated one.",
                count(
                    ("mission-max-tasks", task_id),
                    mission.max_tasks_per_agent,
                    1,
                    20,
                    MissionField::MaxTasksPerAgent,
                    cx,
                ),
            ),
            setting_row(
                "Attempts per task",
                "How many times a failed attempt may release a task back to Ready before it is \
                 blocked and you are told.",
                count(
                    ("mission-max-attempts", task_id),
                    mission.max_attempts,
                    1,
                    10,
                    MissionField::MaxAttempts,
                    cx,
                ),
            ),
        ]))
        .child(section_bar("Spawning", None))
        .child(rows(vec![
            setting_row(
                "When an agent asks for another agent",
                "Ask: a row in Needs you, where you may change the kind first. Auto: launched \
                 without asking, up to the limit below. Never: refused. The scheduler's own \
                 launches are not asked about \u{2014} choosing auto mode is that consent.",
                pills(
                    ("mission-spawn-policy", task_id),
                    [SpawnPolicy::Ask, SpawnPolicy::Auto, SpawnPolicy::Never],
                    mission.spawn_policy,
                    |policy| match policy {
                        SpawnPolicy::Ask => "ask",
                        SpawnPolicy::Auto => "auto",
                        SpawnPolicy::Never => "never",
                    },
                    MissionField::SpawnPolicy,
                    cx,
                ),
            ),
            setting_row(
                "Spawn limit",
                "How many agents may be running from those automatic requests at once. Ignored \
                 unless the policy above is auto.",
                count(
                    ("mission-spawn-limit", task_id),
                    mission.spawn_limit,
                    1,
                    12,
                    MissionField::SpawnLimit,
                    cx,
                ),
            ),
            setting_row(
                "Default kind",
                "Which kind a request that names none resolves to. With none set, a request \
                 without a kind gets whatever profile it named.",
                default_kind(task_id, mission, cx),
            ),
        ]))
        // The table itself, with its own section bar — the Agents tab's, drawn here unchanged.
        .child(super::full::agent_kinds(task_id, mission, cx))
        .into_any_element()
}

/// The column the rows sit in. `setting_row` draws its own rule and its own vertical rhythm, so
/// all this owes it is the padding the rest of the view has.
fn rows(children: Vec<AnyElement>) -> AnyElement {
    div()
        .px_3()
        .flex()
        .flex_none()
        .flex_col()
        .children(children)
        .into_any_element()
}

/// A yes/no setting: the tick box, written the moment it is clicked.
fn switch(
    task_id: TaskId,
    id: gpui::ElementId,
    on: bool,
    field: impl Fn(bool) -> MissionField + 'static,
    cx: &mut Context<AppState>,
) -> AnyElement {
    check_box(
        id,
        on,
        cx.listener(move |this, _, _, cx| this.set_mission_field(task_id, field(!on), cx)),
    )
    .into_any_element()
}

/// One of a short set of values, exactly one lit.
fn pills<T: Copy + PartialEq + 'static>(
    id: (&'static str, TaskId),
    values: impl IntoIterator<Item = T>,
    held: T,
    label: impl Fn(T) -> &'static str + 'static,
    field: impl Fn(T) -> MissionField + Copy + 'static,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let task_id = id.1;
    let pills: Vec<AnyElement> = values
        .into_iter()
        .map(|value| {
            let text = label(value);
            choice_pill(
                eid2(id.0, id.1, text),
                text,
                value == held,
                cx.listener(move |this, _, _, cx| {
                    this.set_mission_field(task_id, field(value), cx)
                }),
            )
            .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .children(pills)
        .into_any_element()
}

/// A whole number between two nudges, clamped to what the field can mean.
fn count(
    id: (&'static str, TaskId),
    held: usize,
    least: usize,
    most: usize,
    field: impl Fn(usize) -> MissionField + Copy + 'static,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let task_id = id.1;
    let down = held.saturating_sub(1).max(least);
    let up = (held + 1).min(most);

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(choice_pill(
            eid2(id.0, id.1, "down"),
            "\u{2212}",
            false,
            cx.listener(move |this, _, _, cx| this.set_mission_field(task_id, field(down), cx)),
        ))
        .child(
            div()
                .w(px(34.))
                .flex()
                .flex_none()
                .justify_center()
                .child(crate::ui::kit::mono(held.to_string(), theme::text())),
        )
        .child(choice_pill(
            eid2(id.0, id.1, "up"),
            "+",
            false,
            cx.listener(move |this, _, _, cx| this.set_mission_field(task_id, field(up), cx)),
        ))
        .into_any_element()
}

/// The kinds the mission has, plus *none* — the one control whose values come off the record.
fn default_kind(
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let held = mission.default_kind.clone();
    let mut pills: Vec<AnyElement> = vec![
        choice_pill(
            eid2("mission-default-kind", task_id, "none"),
            "none",
            held.is_none(),
            cx.listener(move |this, _, _, cx| {
                this.set_mission_field(task_id, MissionField::DefaultKind(None), cx)
            }),
        )
        .into_any_element(),
    ];
    for kind in &mission.agent_kinds {
        let name = kind.name.clone();
        let lit = held.as_deref() == Some(name.as_str());
        let pick = name.clone();
        pills.push(
            choice_pill(
                eid2("mission-default-kind", task_id, name.clone()),
                name,
                lit,
                cx.listener(move |this, _, _, cx| {
                    this.set_mission_field(
                        task_id,
                        MissionField::DefaultKind(Some(pick.clone())),
                        cx,
                    )
                }),
            )
            .into_any_element(),
        );
    }

    div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .children(pills)
        .into_any_element()
}
