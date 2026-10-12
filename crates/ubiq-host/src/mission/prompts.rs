//! What the runner says to the agents it launches, composed host-side (contract §3.3).
//!
//! Workers get what they need inline — the task, the plan and the guidance — because they have no
//! mission server to read it through (Q2). The planner and the mission reviewer carry
//! `ubiq-mission` and `ubiq-plan`, so their prompts point at the tools instead of pasting.

use ubiq_proto::mission::RunEvent;
use ubiq_proto::work::TaskRecord;

use super::runner::since_spawn;

/// What a task is called in a sentence: its key, else its title in quotes.
pub fn name(task: &TaskRecord) -> String {
    task.key
        .clone()
        .unwrap_or_else(|| format!("'{}'", task.title))
}

fn section(title: &str, body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        String::new()
    } else {
        format!("\n\n## {title}\n\n{body}")
    }
}

/// The planner's brief: the mission, what it adopted, and how the planning goes.
pub fn planner(parent: &TaskRecord, included: &[&TaskRecord], has_plan: bool) -> String {
    let mut out = format!(
        "You are the planner of the mission {} — \"{}\" (task id {}).\n\n\
         Work with the user to settle what the mission is, and write the plan with ubiq-plan's \
         write_plan (task_id {}). Ask what only the user can decide with ubiq-ask's \
         ask_user_question. Put longer detail in mission documents (ubiq-mission's \
         write_document). Answer the annotation threads the user leaves on the plan.\n\n\
         When the plan is ready, call ubiq-mission's request_phase with phase 'breakdown' and a \
         summary; the user approves it. After the approval, draft the subtasks with \
         create_mission_tasks — titles, descriptions, prerequisites, `approve` where the user \
         must confirm, review tasks (review_of) and tasks assigned to a person ('human') for \
         checks only a person can make — then call request_phase with phase 'running'. You do \
         not run the tasks: Ubiq's runner does.",
        name(parent),
        parent.title,
        parent.id,
        parent.id,
    );
    out.push_str(&section("Requirements", &parent.description));
    if !parent.attachments.is_empty() {
        let list = parent
            .attachments
            .iter()
            .map(|a| format!("- {}", a.target))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&section("Attached documents", &list));
    }
    if !included.is_empty() {
        let list = included
            .iter()
            .map(|t| format!("- {} {} ({})", name(t), t.title, t.id))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&section(
            "Tasks already in the mission (fold them into the breakdown)",
            &list,
        ));
    }
    if has_plan {
        out.push_str(
            "\n\nA starting plan is already saved as the plan's first revision. Read it with \
             read_plan and refine it — do not write one from scratch.",
        );
    }
    out
}

/// Said to the planner when the user approves the plan.
pub const PLAN_APPROVED: &str = "Plan approved. Draft the subtasks in backlog now with \
     create_mission_tasks, then call request_phase with phase 'running' and a summary.";

/// Said to a planner revived after a restart.
pub const RESTARTED: &str = "Ubiq restarted. Read the plan and the mission state, and continue.";

/// A prompt for a planner that may be a fresh one: the line, then the brief to fall back on.
pub fn planner_with_brief(line: &str, brief: &str) -> String {
    format!("{line}\n\nIf you do not remember this mission, here is its brief.\n\n{brief}")
}

/// A note for the planner to redo the breakdown.
pub fn redo_breakdown(note: &str) -> String {
    format!(
        "Redo the breakdown. The tasks are still in backlog: change, add or abandon them as \
         needed, then call request_phase with phase 'running'.{}",
        section("The user's note", note)
    )
}

/// A note for the planner to revise a running mission.
pub fn replan(note: &str) -> String {
    format!(
        "Replan. The mission is paused in breakdown. Revise the plan, then the tasks that have \
         not started (they are back in backlog); add new ones or abandon what is no longer \
         needed. Tasks already done or in progress stay as they are. When the breakdown is \
         ready, call request_phase with phase 'running'.{}",
        section("The user's note", note)
    )
}

fn task_block(task: &TaskRecord) -> String {
    let mut out = format!("{} — \"{}\" (task id {})", name(task), task.title, task.id);
    out.push_str(&section("Description", &task.description));
    if !task.steps.is_empty() {
        let list = task
            .steps
            .iter()
            .map(|s| format!("- {}", s.title))
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&section("Todos", &list));
    }
    out
}

/// A worker's opening prompt: the task, the plan and the guidance.
pub fn worker(task: &TaskRecord, plan: &str, guidance: &str) -> String {
    format!(
        "You have been given one task of a mission: {}\n\n\
         Work it, and only it. When you start, move it to 'in progress' with use-task's \
         change_state; when it is done, move it to 'in review'. Leave a comment (add_comment) for \
         anything the user should know. Ask with ask_user_question only for what you cannot \
         decide yourself.{}{}",
        task_block(task),
        section("The mission's plan", plan),
        section("Guidance from the user", guidance),
    )
}

/// A reviewer's opening prompt: what to review, and how to answer.
pub fn reviewer(
    task: &TaskRecord,
    reviewed: Option<&TaskRecord>,
    plan: &str,
    guidance: &str,
) -> String {
    let under = reviewed.map_or_else(String::new, |t| {
        section("The task under review", &task_block(t))
    });
    format!(
        "You are reviewing work in a mission. Your review task is {}{}\n\n\
         Check the work (read the task's comments with use-task's get_task, and the code). When \
         you have decided, call use-task's submit_review on your review task ({}) with verdict \
         'pass', or 'fail' and findings that say exactly what to fix. Then stop.{}{}",
        task_block(task),
        under,
        task.id,
        section("The mission's plan", plan),
        section("Guidance from the user", guidance),
    )
}

/// The final review over the whole mission.
pub fn mission_reviewer(parent: &TaskRecord) -> String {
    format!(
        "You are the final reviewer of the mission {} — \"{}\" (task id {}). Read the brief and \
         the documents with ubiq-mission, the plan with read_plan, and check the work against \
         them. Create a task with create_mission_tasks for each thing that must be fixed. Then \
         call use-task's submit_review on the mission ({}) with verdict 'pass' or 'fail' and \
         your findings, and stop.",
        name(parent),
        parent.title,
        parent.id,
        parent.id,
    )
}

/// The prompt a Retry launches with: the opening prompt, plus every piece of feedback written
/// since the last launch.
pub fn continuing(opening: &str, task: &TaskRecord) -> String {
    let feedback: Vec<String> = since_spawn(task)
        .filter(|c| {
            matches!(
                c.event,
                Some(RunEvent::Rejected)
                    | Some(RunEvent::HumanFailed)
                    | Some(RunEvent::ReviewVerdict { pass: false })
            )
        })
        .map(|c| format!("- {}", c.text.trim()))
        .collect();
    format!(
        "Continue this task: a previous agent started it and stopped before finishing. Look at \
         what is already done (the task's comments, the working tree) and carry on from there.{}\
         \n\n{opening}",
        section("Feedback to address", &feedback.join("\n")),
    )
}

/// Feedback delivered to a live worker as its next turn.
pub fn feedback(task: &TaskRecord, text: &str) -> String {
    format!(
        "Feedback on {}: the task is back in progress. Address this, then move it to 'in review' \
         again.\n\n{}",
        name(task),
        text.trim()
    )
}

/// Guidance delivered to a live worker.
pub fn guidance(text: &str) -> String {
    format!(
        "New guidance from the user for the whole mission:\n\n{}",
        text.trim()
    )
}
