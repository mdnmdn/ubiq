//! What is known about one conversation, in one panel.
//!
//! **A reading, not a form.** Every line here is something the host already said — the work record
//! it broadcasts and the conversation state the transcript is drawn from — so nothing in this file
//! sends a message, and a conversation that has not launched yet simply has fewer answers to give
//! rather than a panel that waits for them.
//!
//! The three buttons at the foot are the exception, and they are still not questions: a path the
//! host reported is handed to the desktop's own file manager. Ubiq names none of them and reads
//! none of them — see `WorkAgent::config_dir`, which is the host repeating the library's answer
//! precisely so this panel can offer the folder without knowing what a harness keeps in it.

use gpui::{
    AnyElement, ClipboardItem, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::IconName;

use ubiq_proto::work::WorkAgent;

use crate::app::AppState;
use crate::state::conversation::Conversation;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::handler;
use crate::ui::kit::{elided, ghost_button, icon_button, modal, section_label};

/// How wide a row's label column is. One number so every section's values line up down the panel,
/// which is the whole reason a reading like this is readable at a glance.
const LABEL_WIDTH: f32 = 96.;

/// The panel, or nothing at all when none is open.
///
/// Raised from the conversation's own three-dots menu, which every surface that hosts a
/// conversation draws — so this is called from [`super::render`] rather than from the shell, and
/// keyed on the agent the way the end-conversation confirm beside it is.
pub fn render(
    app: &AppState,
    conversation: &Conversation,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    if app.conversation_info != Some(id) {
        return div().into_any_element();
    }
    let entity = cx.entity();
    // The owning project's record: the Teams inspector reaches this panel for any open project's
    // card under the window span, and the project on screen holds no row for a foreign one.
    let record = app.teams_agent(id, cx);
    let harness = harness_label(conversation, record);

    let mut body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(profile(conversation, record))
        .child(account(conversation, record))
        .child(tokens(conversation, record))
        .child(context(conversation, record));
    if let Some(record) = record {
        body = body.child(session(record, cx));
    }
    if app.conversation_info_capabilities {
        // Read once for the frame and threaded down, the way the settings page threads its own
        // `now_ms` into every block that words an age: a clock read inside a render is a reading
        // that differs between two lines of the same panel.
        let now_ms = chrono::Utc::now().timestamp_millis();
        body = body.child(capabilities(app, &harness, now_ms));
    }

    modal(
        "conversation-info",
        theme::accent(),
        "Conversation info",
        body.into_any_element(),
        footer(app, record, &harness, cx),
        handler(&entity, |this, _, cx| this.dismiss_conversation_info(cx)),
        window,
    )
}

/// Which harness answers this conversation, as what, and under which permission mode.
///
/// The harness is the display label the host minted, never an id: a `WorkAgent` carries the label
/// and the window has nothing else in hand — the same reason `super::keeps_sessions` matches on it.
fn profile(conversation: &Conversation, record: Option<&WorkAgent>) -> AnyElement {
    let harness = harness_label(conversation, record);
    let model = conversation
        .model
        .clone()
        .or_else(|| record.map(|agent| agent.model.clone()))
        .filter(|model| !model.is_empty());
    let mut section = group("Profile").child(row("info-harness", "Harness", unknown(harness)));
    section = section.child(row(
        "info-model",
        "Model",
        unknown(model.unwrap_or_default()),
    ));
    section = section.child(row(
        "info-mode",
        "Mode",
        unknown(conversation.mode.clone().unwrap_or_default()),
    ));
    section.into_any_element()
}

/// The identity the run actually resolved to.
///
/// **Empty is an answer, not a blank.** An account that resolved to nothing means the harness ran
/// as the user's own home, which is a different fact from "not known yet" and reads as one here —
/// a blank line would leave the reader to guess which of the two it was looking at.
fn account(conversation: &Conversation, record: Option<&WorkAgent>) -> AnyElement {
    let account = record
        .map(|agent| agent.account.clone())
        .filter(|account| !account.is_empty())
        .or_else(|| Some(conversation.account.clone()).filter(|a| !a.is_empty()));
    let value = account.unwrap_or_else(|| "none \u{2014} the user's own home".to_string());
    group("Account")
        .child(row("info-account", "Account", value))
        .into_any_element()
}

/// What this conversation has billed, split the way the harness reported it.
///
/// [`Conversation::spend`] is the full breakdown and `WorkAgent::tokens` is a single rounded
/// count — the first is preferred wherever it exists, and the second is what a record with no
/// structured spend behind it still has to say.
fn tokens(conversation: &Conversation, record: Option<&WorkAgent>) -> AnyElement {
    let mut section = group("Tokens");
    match conversation.spend.as_ref() {
        Some(spend) => {
            section = section
                .child(row("info-spend-total", "Total", count(spend.total())))
                .child(row("info-spend-in", "Input", count(spend.input)))
                .child(row("info-spend-out", "Output", count(spend.output)))
                .child(row("info-spend-think", "Thinking", count(spend.thinking)))
                .child(row(
                    "info-spend-read",
                    "Cache read",
                    count(spend.cache_read),
                ))
                .child(row(
                    "info-spend-write",
                    "Cache write",
                    count(spend.cache_creation),
                ));
        }
        None => {
            let tokens = record.map(|agent| agent.tokens).unwrap_or_default();
            section = section.child(row("info-tokens", "Total", count(tokens as u64)));
        }
    }
    // Only where a delegate actually spent something. A conversation that spawned none looks
    // exactly as it did before — the discipline every other element of this view follows.
    if !conversation.spend_by_subagent.is_empty() {
        section = section.child(section_label("By subagent").into_any_element());
        for (name, spend) in &conversation.spend_by_subagent {
            section = section.child(row(
                ElementId::Name(format!("info-subagent-{name}").into()),
                name,
                count(spend.total()),
            ));
        }
    }
    section.into_any_element()
}

/// How full the window is, and what the last report said about it.
///
/// Nothing here holds a context-window constant: the size is per model and the harness is the only
/// thing that knows which model answered, so both numbers are the host's and neither is derived.
fn context(conversation: &Conversation, record: Option<&WorkAgent>) -> AnyElement {
    let pct = record.map(|agent| agent.context_pct).unwrap_or_default();
    let mut section = group("Context").child(row("info-context-pct", "Filled", format!("{pct}%")));
    if let Some(usage) = conversation.usage.as_ref() {
        section = section.child(row(
            "info-context-used",
            "Window",
            format!("{} / {}", count(usage.used), count(usage.size)),
        ));
    }
    section.into_any_element()
}

/// The conversation's id, and the one control in the panel that is not a button at the foot.
///
/// A copy rather than a field: it is the handle every log line and every `am` invocation is keyed
/// by, it is far too long to retype, and it is the thing a reader opens this panel for when
/// something has gone wrong somewhere else.
fn session(record: &WorkAgent, cx: &mut Context<AppState>) -> AnyElement {
    let value = record.session.to_string();
    let copy = value.clone();
    group("Session")
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(label("Session id"))
                .child(div().flex_1().min_w(px(0.)).child(elided(
                    "info-session",
                    value,
                    theme::text(),
                    meta(),
                )))
                .child(icon_button(
                    "info-session-copy",
                    IconName::Copy,
                    false,
                    cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    }),
                )),
        )
        .into_any_element()
}

/// The three folders, in the order a reader reaches for them: where the run lives, where its
/// configuration was written, and where its capture is going.
///
/// **The dump button is there only while there is a dump.** The other two are always drawn, dead
/// where the host has reported no path — a control that comes and goes teaches nothing about why
/// it is missing, and a conversation that has never launched has genuinely no configuration
/// directory to name.
fn footer(
    app: &AppState,
    record: Option<&WorkAgent>,
    harness: &str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let run_dir = record.and_then(|agent| agent.run_dir.clone());
    let config_dir = record.and_then(|agent| agent.config_dir.clone());
    let dump = record.and_then(|agent| agent.debug_dump.clone());

    let mut row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(reveal("info-open-home", "Open home", run_dir, cx))
        .child(reveal(
            "info-open-config",
            "Open config dir",
            config_dir,
            cx,
        ));
    if dump.is_some() {
        row = row.child(reveal("info-open-dump", "Open dump", dump, cx));
    }
    row = row.child(capabilities_button(app, harness, cx));
    row.into_any_element()
}

/// The one control in this panel that asks the host anything: show what this conversation's
/// harness said it can do.
///
/// A toggle inside the same modal rather than a second overlay — it is another reading of the same
/// conversation, and a modal raised over a modal to say one more thing about it is a layer the
/// reader has to dismiss twice. Drawn live for an ACP harness and dead, with the reason under the
/// pointer, for anything else: a harness with its own wire has no such answer, and a control that
/// vanished would read as a feature that is missing.
fn capabilities_button(app: &AppState, harness: &str, cx: &mut Context<AppState>) -> AnyElement {
    let acp = app
        .workbench
        .agent_type_by_label(harness)
        .is_some_and(|info| info.acp);
    if !acp {
        return div()
            .id("info-capabilities-dead")
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .border_1()
            .border_color(theme::border())
            .text_size(meta())
            .text_color(theme::text_faint())
            .child("Capabilities")
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(
                    "This harness speaks its own wire rather than ACP, so it advertises no \
                     capabilities.",
                )
                .build(window, cx)
            })
            .into_any_element();
    }

    let harness = harness.to_string();
    ghost_button(
        "info-capabilities",
        Some(IconName::Info),
        if app.conversation_info_capabilities {
            "Hide capabilities"
        } else {
            "Capabilities"
        },
        cx.listener(move |this, _, _, cx| {
            this.toggle_conversation_info_capabilities(harness.clone(), cx)
        }),
    )
    .into_any_element()
}

/// The capabilities section, drawn inside the body while the foot's toggle is on.
///
/// The panel itself is shared with the harness settings — see
/// [`crate::ui::acp_capabilities::panel`] — so the two surfaces word every answer, including both
/// empty ones, exactly the same way.
fn capabilities(app: &AppState, harness: &str, now_ms: i64) -> AnyElement {
    let info = app.workbench.agent_type_by_label(harness);
    let id = info.map(|info| info.id.as_str()).unwrap_or(harness);
    let acp = info.is_some_and(|info| info.acp);
    group("Capabilities")
        .child(crate::ui::acp_capabilities::panel(
            id,
            app.workbench.settings.acp_capabilities(id),
            acp,
            now_ms,
        ))
        .into_any_element()
}

/// Which harness answers this conversation, as the display label both halves of the panel read.
///
/// The work record's label where the host has minted one, the conversation's own otherwise — the
/// same fallback [`profile`] draws, kept in one place now that the foot resolves it to a harness
/// too.
fn harness_label(conversation: &Conversation, record: Option<&WorkAgent>) -> String {
    record
        .map(|agent| agent.harness.clone())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| conversation.harness.clone())
}

/// One folder button: live where there is a path, dead where there is not.
///
/// The dead form is drawn here rather than taken from the kit because no kit button has one — and
/// a disabled button is a shape the kit should grow only when a second caller wants it, which is
/// the rule this module is deliberately not the first exception to.
fn reveal(
    id: &'static str,
    label: &'static str,
    path: Option<String>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(path) = path else {
        return div()
            .h(px(26.))
            .px_2()
            .flex()
            .items_center()
            .border_1()
            .border_color(theme::border())
            .text_size(meta())
            .text_color(theme::text_faint())
            .child(label)
            .into_any_element();
    };
    ghost_button(
        id,
        Some(IconName::FolderOpen),
        label,
        cx.listener(move |this, _, _, cx| this.reveal_conversation_dir(path.clone(), cx)),
    )
    .into_any_element()
}

/// A titled group of rows. Every section in the panel is one, so the headings sit at one weight
/// and the rows under them at one indent.
fn group(title: &str) -> gpui::Div {
    div().flex().flex_col().gap_1().child(section_label(title))
}

/// One `label: value` line, elided rather than wrapped — a row is one line, and a path or a model
/// id is exactly the kind of string that would otherwise take three.
fn row(id: impl Into<ElementId>, name: &str, value: impl Into<String>) -> AnyElement {
    let value: String = value.into();
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(label(name))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(elided(id, value, theme::text(), meta())),
        )
        .into_any_element()
}

/// The left half of a row, at the one width every section shares.
fn label(name: &str) -> gpui::Div {
    div()
        .w(px(LABEL_WIDTH))
        .flex_none()
        .text_size(meta())
        .text_color(theme::text_muted())
        .child(name.to_string())
}

fn meta() -> gpui::Pixels {
    theme::font(Family::Chrome, Role::Meta)
}

/// What a value reads as when the host has not said one yet. Never a blank: an empty line and an
/// unanswered question look the same, and only one of them is what is happening here.
fn unknown(value: String) -> String {
    if value.is_empty() {
        "\u{2014}".to_string()
    } else {
        value
    }
}

/// A token count, grouped in threes. Six unbroken digits is a number nobody reads at a glance,
/// and every number in this panel is there to be read at a glance.
fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (seen, ch) in digits.chars().rev().enumerate() {
        if seen > 0 && seen % 3 == 0 {
            out.push('\u{202f}');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
