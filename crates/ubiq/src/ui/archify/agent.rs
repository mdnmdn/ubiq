//! The agent loop's interface half (P5.3): the **Ask agent** and **New diagram** entries (the
//! viewer's `⋯` menu, and a button in the top row) and the prompts they start the agent on,
//! the tool-call status chip, the notification when an agent's validation turns a failing document
//! ok, and the camera reveal a diagnostic click asks for.
//!
//! The agent talks to the host's MCP server and never to this crate (D3); all the interface learns
//! of a tool call is `Event::Agent { rel, tool, ok }`, which `state.rs` files as an [`AgentCall`]
//! under the tab of that file. The prompt, the chip text and the failing-to-ok rule are pure and
//! unit-tested; the rest is the thin GPUI glue around the base's new-agent entry
//! (`AppState::open_new_agent`).

use std::time::{Duration, Instant};

use ubiq_archify::compile::peek;
use ubiq_archify::diag::Diagnostic;
use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, Styled, WeakEntity, Window,
};
use gpui_component::IconName;
use crate::app::AppState;
use crate::state::new_agent::NewAgentOpen;
use crate::theme;
use crate::ui::kit::ghost_button;
use ubiq_proto::notifications::{Family, NotificationRequest};

use crate::state::archify::compiled::Outcome;
use crate::state::archify::focus::reveal_delta;
use crate::state::archify::motion;
use crate::ui::archify::panels::where_of;
use crate::state::archify::ui;

/// The notification category.
pub const CATEGORY: &str = "archify";

/// The tool whose result decides "ok".
pub const VALIDATE: &str = "archify_validate";

/// How many diagnostics the prompt lists, and how much of a message it keeps.
const MAX_LISTED: usize = 8;
const MAX_MESSAGE: usize = 160;

/// The gap kept between a revealed box and the picture panel's edge (px).
const REVEAL_MARGIN: f32 = 32.0;

/// The latest MCP tool call the host reported for a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCall {
    pub tool: String,
    pub ok: bool,
    pub at: Instant,
}

/// What `state.rs` hands back for the window to raise: the text of a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
}

/// What the Ask agent button starts the agent on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub prompt: String,
    /// Start at once. Only when the prompt is complete: "create a new diagram" ends in a blank the
    /// user fills in the dialog, so it does not.
    pub autostart: bool,
}

// --------------------------------------------------------------------------------------------- //
// Pure
// --------------------------------------------------------------------------------------------- //

/// "3s ago", "2m ago", "1h ago".
pub fn ago(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    match s {
        0..60 => format!("{s}s ago"),
        60..3600 => format!("{}m ago", s / 60),
        _ => format!("{}h ago", s / 3600),
    }
}

/// The chip's text: `agent: archify_validate ✓ · 3s ago`.
pub fn chip_label(call: &AgentCall, now: Instant) -> String {
    let mark = if call.ok { '\u{2713}' } else { '\u{2717}' };
    format!(
        "agent: {} {mark} \u{b7} {}",
        call.tool,
        ago(now.saturating_duration_since(call.at))
    )
}

/// An agent's validation turned a document from failing to ok. `previous` is what the agent last
/// saw of it (or what the document was when Ask agent was pressed); `None` is unknown and never
/// notifies.
pub fn turned_ok(previous: Option<bool>, ok: bool) -> bool {
    previous == Some(false) && ok
}

/// `n errors, m warnings:` and a line per diagnostic, capped.
pub fn summary(diagnostics: &[Diagnostic]) -> String {
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == ubiq_archify::diag::Severity::Warning)
        .count();
    let errors = diagnostics.len() - warnings;
    let mut out = format!(
        "{errors} error{}, {warnings} warning{}:",
        plural(errors),
        plural(warnings)
    );
    for d in diagnostics.iter().take(MAX_LISTED) {
        out.push_str("\n- ");
        out.push_str(&d.code);
        if let Some(at) = where_of(d) {
            out.push_str(" @ ");
            out.push_str(&at);
        }
        out.push_str(": ");
        out.push_str(&clip(&d.message, MAX_MESSAGE));
    }
    if diagnostics.len() > MAX_LISTED {
        out.push_str(&format!("\n- (+{} more)", diagnostics.len() - MAX_LISTED));
    }
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `text` on one line, cut to `max` characters with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut cut: String = one.chars().take(max.saturating_sub(1)).collect();
    cut.push('\u{2026}');
    cut
}

/// The prompt for the open document: which file, its type, what is wrong with it now, and the
/// loop to follow (guide first, then validate until ok).
pub fn prompt_for_doc(rel: &str, diagram_type: Option<&str>, outcome: Option<&Outcome>) -> String {
    let kind = diagram_type.unwrap_or("unknown type");
    let mut prompt = format!("Work on the Archify {kind} diagram `{rel}` in this project.\n\n");
    prompt.push_str("1. Call `archify_guide` first and follow it.\n");
    match outcome {
        Some(o) if !o.ok => {
            prompt.push_str(&format!(
                "2. Fix the document, then call `archify_validate` with path \"{rel}\" and repair \
                 until it reports ok.\n\nIt does not validate yet. Diagnostics right now ({} \
                 profile), {}",
                o.profile,
                summary(&o.diagnostics)
            ));
        }
        Some(o) => {
            prompt.push_str(&format!(
                "2. It validates right now ({} profile). Review it against the guide, improve \
                 what the guide flags, and call `archify_validate` with path \"{rel}\" after \
                 every edit until it reports ok again.",
                o.profile
            ));
            if !o.diagnostics.is_empty() {
                prompt.push_str(&format!("\n\nWarnings right now, {}", summary(&o.diagnostics)));
            }
        }
        None => {
            prompt.push_str(&format!(
                "2. Call `archify_validate` with path \"{rel}\" to see where it stands, then \
                 repair until it reports ok."
            ));
        }
    }
    prompt
}

/// The prompt for a new diagram, for the user to complete. The file is a `.archify` (D30); the
/// agent picks the type that fits what the user says it should show.
pub fn prompt_for_new() -> String {
    "Create a new Archify diagram in this project, as `<name>.archify` at the project root. Pick \
     the type that fits what it shows (architecture, workflow, sequence, dataflow or lifecycle).\n\n\
     1. Call `archify_guide` first and follow it.\n\
     2. Write the file, then call `archify_validate` with its path and repair until it \
     reports ok.\n\nThe diagram should show: "
        .to_string()
}

// --------------------------------------------------------------------------------------------- //
// The notification
// --------------------------------------------------------------------------------------------- //

/// `Family::Ubiq` plus Archify's category: the family set is closed (`notifications-status.md`).
pub fn notification(notice: Notice) -> NotificationRequest {
    NotificationRequest::info(Family::Ubiq, notice.text)
        .with_actor("Archify")
        .with_category(CATEGORY)
}

// --------------------------------------------------------------------------------------------- //
// Elements
// --------------------------------------------------------------------------------------------- //

/// The status chip for a tool call: green on ok, red otherwise.
pub fn chip(call: &AgentCall) -> AnyElement {
    let colour = if call.ok {
        theme::success()
    } else {
        theme::danger()
    };
    crate::ui::archify::panels::flat_chip(chip_label(call, Instant::now()), colour).into_any_element()
}

/// The top row's **Ask agent** button. `on_click` comes from `cx.listener`, so it reaches
/// `AppState`.
pub fn ask_button(
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    ghost_button("archify-ask-agent", Some(IconName::Bot), "Ask agent", on_click)
        .h_full()
        .into_any_element()
}

/// Ask agent on the tab `key`: build the prompt from the file and what is wrong with it now, and
/// open the base's new-agent entry with the prompt. The same action the
/// `⋯` menu and the top row's button run.
pub fn ask(app: &mut AppState, key: &str, window: &mut Window, cx: &mut Context<AppState>) {
    let (rel, kind) = match app.file(key, cx) {
        Some(file) => {
            let kind = file
                .buffer()
                .and_then(|buffer| peek(&buffer.read(cx).value()).diagram_type);
            (file.path.clone(), kind)
        }
        None => return,
    };
    let Ask { prompt, autostart } = ui(cx).ask_for(key, &rel, kind.as_deref());
    start(app, prompt, autostart, window, cx);
}

/// "New diagram…": the agent starts on a prompt ending in a blank the user fills in the dialog.
pub fn ask_new(app: &mut AppState, window: &mut Window, cx: &mut Context<AppState>) {
    start(app, prompt_for_new(), false, window, cx);
}

fn start(
    app: &mut AppState,
    prompt: String,
    autostart: bool,
    window: &mut Window,
    cx: &mut Context<AppState>,
) {
    app.open_new_agent(
        NewAgentOpen {
            mcps: vec!["ubiq-archify".into(), "ubiq-ask".into()],
            tags: None,
            autostart,
            initial_prompt: Some(prompt),
        },
        window,
        cx,
    );
}

// --------------------------------------------------------------------------------------------- //
// Redraws and the camera
// --------------------------------------------------------------------------------------------- //

/// The chip says "3s ago" and nothing redraws a quiet window, so ask for a frame every second for
/// ten seconds after a call and every ten seconds for a few minutes more. One ticker at a time.
pub fn keep_fresh(cx: &mut Context<AppState>) {
    if std::mem::replace(&mut ui(cx).ticking, true) {
        return;
    }
    cx.spawn(async move |this: WeakEntity<AppState>, cx| {
        for step in 0..30 {
            let wait = Duration::from_secs(if step < 10 { 1 } else { 10 });
            cx.background_executor().timer(wait).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }
        let _ = this.update(cx, |_, cx| ui(cx).ticking = false);
    })
    .detach();
}

/// Bring the box a diagnostic click left on the picture `key` into view, once the panel is
/// measured. A box already clear of the margin stays; otherwise the camera glides to frame it
/// (`motion::frame_to_nodes`, P7.2; an instant jump when motion is off). `motion::drive`, called
/// beside this from render, supplies the frames.
pub fn drive_reveal(app: &AppState, key: &str, cx: &mut Context<AppState>) {
    let Some(rect) = ui(cx).reveal_pending(key) else {
        return;
    };
    let vp = app.viewport(key);
    if !vp.measured() {
        return; // not painted yet: the pending box waits for the first frame
    }
    ui(cx).view(key).reveal = None;
    let camera = vp.camera(vp.content, vp.panel_w, vp.panel_h);
    if reveal_delta(
        camera.scale,
        [camera.offset_x, camera.offset_y],
        [vp.panel_w, vp.panel_h],
        rect,
        REVEAL_MARGIN,
    )
    .is_none()
    {
        return; // already in view
    }
    let panel = [vp.panel_w, vp.panel_h];
    let Some(target) =
        motion::frame_to_nodes(panel, [vp.content.width, vp.content.height], &[rect])
    else {
        return;
    };
    motion::glide_to(app, key, target, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::diag::Subject;

    fn diag(code: &str, path: &str, message: &str) -> Diagnostic {
        Diagnostic::error(code, message)
            .with_subject(Subject::of("dataflow").with_path(path))
    }

    #[test]
    fn the_chip_says_the_tool_the_result_and_how_long_ago() {
        let at = Instant::now();
        let call = AgentCall {
            tool: "archify_validate".into(),
            ok: true,
            at,
        };
        assert_eq!(
            chip_label(&call, at + Duration::from_secs(3)),
            "agent: archify_validate \u{2713} \u{b7} 3s ago"
        );
        let failed = AgentCall { ok: false, ..call };
        assert_eq!(
            chip_label(&failed, at + Duration::from_secs(125)),
            "agent: archify_validate \u{2717} \u{b7} 2m ago"
        );
        assert_eq!(ago(Duration::from_secs(7300)), "2h ago");
        // A call stamped after "now" reads zero, not a panic.
        let future = AgentCall {
            at: at + Duration::from_secs(5),
            ..failed
        };
        assert!(chip_label(&future, at).ends_with("0s ago"));
    }

    #[test]
    fn only_a_known_failure_turning_ok_notifies() {
        assert!(turned_ok(Some(false), true));
        assert!(!turned_ok(Some(false), false));
        assert!(!turned_ok(Some(true), true), "ok to ok is not news");
        assert!(!turned_ok(None, true), "unknown before is not a failure");
    }

    #[test]
    fn the_summary_lists_codes_and_messages_and_caps_them() {
        let many: Vec<Diagnostic> = (0..11)
            .map(|i| diag("schema/required", &format!("/nodes/{i}"), "missing   \n id"))
            .collect();
        let text = summary(&many);
        assert!(text.starts_with("11 errors, 0 warnings:"));
        assert!(text.contains("- schema/required @ /nodes/0: missing id"));
        assert_eq!(text.lines().count(), 1 + MAX_LISTED + 1);
        assert!(text.ends_with("(+3 more)"));

        let long = summary(&[diag("x/y", "/a", &"w ".repeat(200))]);
        let line = long.lines().nth(1).unwrap();
        assert!(line.ends_with('\u{2026}'));
        assert!(line.chars().count() < 200);
        assert!(summary(&[]).starts_with("0 errors, 0 warnings:"));
    }

    #[test]
    fn the_document_prompt_names_the_file_the_type_the_diagnostics_and_the_loop() {
        let outcome = Outcome {
            ok: false,
            profile: "standard",
            diagnostics: vec![diag("schema/required", "/meta/title", "title is required")],
            scene: None,
            trace: false,
        };
        let p = prompt_for_doc("docs/a.dataflow.json", Some("dataflow"), Some(&outcome));
        assert!(p.contains("`docs/a.dataflow.json`") && p.contains("dataflow diagram"));
        assert!(p.contains("archify_guide") && p.contains("archify_validate"));
        assert!(p.contains("schema/required @ /meta/title: title is required"));
        assert!(p.contains("until it reports ok"));

        let clean = Outcome {
            diagnostics: Vec::new(),
            ok: true,
            ..outcome
        };
        let p = prompt_for_doc("a.workflow.json", None, Some(&clean));
        assert!(p.contains("validates right now") && p.contains("unknown type"));
        assert!(!p.contains("Warnings right now"));
        assert!(prompt_for_doc("a.workflow.json", None, None).contains("to see where it stands"));
    }

    #[test]
    fn the_new_diagram_prompt_names_an_archify_file_and_is_left_for_the_user_to_finish() {
        let p = prompt_for_new();
        assert!(p.contains("`<name>.archify`") && !p.contains(".json"));
        assert!(p.starts_with("Create a new Archify"));
        assert!(p.ends_with("should show: "));
    }

    #[test]
    fn the_notification_is_the_ubiq_family_with_archifys_category() {
        let n = notification(Notice {
            text: "fixed".into(),
        });
        assert_eq!(n.family, Family::Ubiq);
        assert_eq!(n.category.as_deref(), Some("archify"));
        assert_eq!(n.text, "fixed");
    }
}
