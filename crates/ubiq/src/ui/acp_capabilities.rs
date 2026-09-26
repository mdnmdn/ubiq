//! What one ACP harness said it can do, drawn once for the two places that ask.
//!
//! **A reading, not a form.** Every line is something the agent itself stated in its `initialize`
//! answer and the host kept — see [`ubiq_proto::acp`] — so nothing here sends a message and no
//! control in it would change the answer. That is why every negative case is a sentence rather
//! than a disabled button: a harness that speaks its own wire will never advertise capabilities,
//! and one that has never been conversed with gains a record the moment an agent connects, with
//! nothing for the reader to do in between.
//!
//! **Its own dialog, raised from two places** (`T-207`): a small icon button beside the other
//! controls on a login in the harnesses settings section, and a button in a conversation's info
//! modal. It used to be drawn inline in both, which made a long reading the thing a reader had to
//! scroll a settings page past to reach the next login. One render for both callers: the panel is
//! a reading of a harness fact, and two readings of one fact are two vocabularies for it.

use gpui::{
    AnyElement, Context, ElementId, IntoElement, ParentElement, SharedString, Styled, Window, div,
    px,
};

use ubiq_proto::acp::AcpCapabilitiesRecord;

use crate::app::AppState;
use crate::state::Layer;
use crate::state::settings::magnitude;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{elided, ghost_button, modal, section_label};

/// The dialog itself: the panel for whichever harness [`crate::state::workbench::WorkbenchState`]
/// says is being read, over whatever raised it.
///
/// A reading, so the footer holds one button and it closes. Nothing in the body sends anything.
pub fn dialog(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(agent_type) = app.workbench.capabilities.clone() else {
        return div().into_any_element();
    };
    let info = app.workbench.agent_type(&agent_type);
    let acp = info.is_some_and(|info| info.acp);
    let title = info
        .map(|info| info.label.clone())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| agent_type.clone());
    // One clock read for the frame, threaded into the panel: a reading inside a render is a
    // reading that differs between two lines of the same surface.
    let now_ms = chrono::Utc::now().timestamp_millis();
    let view = cx.entity();

    modal(
        "harness-capabilities",
        theme::accent(),
        &format!("{title} \u{2014} capabilities"),
        panel(
            &agent_type,
            app.workbench.settings.acp_capabilities(&agent_type),
            acp,
            now_ms,
        ),
        ghost_button(
            "harness-capabilities-close",
            None,
            "Close",
            cx.listener(|this, _, _, cx| this.close_capabilities(cx)),
        )
        .into_any_element(),
        crate::ui::dismiss(&view, Layer::Capabilities, |this, _, cx| {
            this.close_capabilities(cx)
        }),
        window,
    )
}

/// The capabilities panel for one harness.
///
/// `agent_type` is the library's harness id — it keys every element in the panel, so the same
/// render can stand in two surfaces at once without their rows colliding. `acp` is
/// [`ubiq_proto::messages::AgentTypeInfo::acp`], which is what tells a missing record apart from a
/// harness that has no such answer to give; `record` is what the host last kept, and `now_ms` is
/// what the age of that record is measured against.
pub fn panel(
    agent_type: &str,
    record: Option<&AcpCapabilitiesRecord>,
    acp: bool,
    now_ms: i64,
) -> AnyElement {
    if !acp {
        return block()
            .child(note(
                "This harness speaks its own wire rather than ACP, so it advertises no \
                 capabilities.",
            ))
            .into_any_element();
    }

    let Some(record) = record else {
        return block()
            .child(note(
                "Discovered when an agent on this harness first connects \u{2014} none has yet.",
            ))
            .into_any_element();
    };

    let mut panel = block();
    panel = panel.child(identity(record));

    for (index, group) in record.groups.iter().enumerate() {
        let mut section = div().flex().flex_col().gap_1().pt_1();
        section = section.child(section_label(&group.label));
        section = section.children(group.entries.iter().enumerate().map(|(row, entry)| {
            capability_row(
                ElementId::Name(format!("acp-caps-{agent_type}-{index}-{row}").into()),
                &entry.label,
                &entry.id,
                entry.supported,
                &entry.description,
            )
        }));
        panel = panel.child(section);
    }

    // Omitted entirely where the agent named none: an empty heading says the agent answered the
    // question and it did not.
    if !record.auth_methods.is_empty() {
        let mut section = div().flex().flex_col().gap_1().pt_1();
        section = section.child(section_label("Authentication"));
        section = section.children(record.auth_methods.iter().enumerate().map(|(row, method)| {
            auth_row(
                ElementId::Name(format!("acp-caps-{agent_type}-auth-{row}").into()),
                method,
            )
        }));
        panel = panel.child(section);
    }

    panel
        .child(
            div()
                .pt_1()
                .text_size(meta())
                .text_color(theme::text_faint())
                .child(SharedString::from(format!(
                    "discovered {} ago",
                    magnitude(now_ms - record.discovered_ms)
                ))),
        )
        .into_any_element()
}

/// Who answered, and on which protocol. The agent's own words: `display_name` is its `title` where
/// it sent one, and the version is the build it named rather than anything this side inferred.
fn identity(record: &AcpCapabilitiesRecord) -> AnyElement {
    let who = match record.agent.as_ref() {
        Some(agent) => match agent.version.as_deref() {
            Some(version) => format!("{} {version}", agent.display_name()),
            None => agent.display_name().to_string(),
        },
        None => "\u{2014}".to_string(),
    };

    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(SharedString::from(who)),
        )
        .child(
            div()
                .flex_none()
                .text_size(meta())
                .text_color(theme::text_muted())
                .child(SharedString::from(format!(
                    "ACP protocol v{}",
                    record.protocol_version
                ))),
        )
        .into_any_element()
}

/// One capability: what it is called, whether the agent has it, and what having it means.
///
/// The wire key is drawn faint and monospaced beside the label rather than as the label itself —
/// it is the one part of the row that is not prose, and it is what a reader matches against a
/// protocol document when the prose is not enough.
fn capability_row(
    id: ElementId,
    label: &str,
    key: &str,
    supported: bool,
    description: &str,
) -> AnyElement {
    let (mark, colour) = if supported {
        ("\u{2713}", theme::success())
    } else {
        ("\u{2014}", theme::text_faint())
    };

    let mut lines = div().flex().flex_col().flex_1().min_w(px(0.)).child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(meta())
                    .text_color(if supported {
                        theme::text()
                    } else {
                        theme::text_muted()
                    })
                    .child(SharedString::from(label.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .font_family(theme::MONO_FONT)
                    .child(elided(id, key.to_string(), theme::text_faint(), micro())),
            ),
    );
    if !description.is_empty() {
        lines = lines.child(
            div()
                .text_size(micro())
                .text_color(theme::text_faint())
                .child(SharedString::from(description.to_string())),
        );
    }

    div()
        .flex()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(12.))
                .flex_none()
                .text_size(meta())
                .text_color(colour)
                .child(SharedString::from(mark)),
        )
        .child(lines)
        .into_any_element()
}

/// One authentication method the agent advertised — its name, its own line about it, and a mark on
/// the one it nominated as needing no interaction. No credential material passes through here.
fn auth_row(id: ElementId, method: &ubiq_proto::acp::AcpAuthMethodRecord) -> AnyElement {
    let mut lines = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(meta())
                        .text_color(theme::text())
                        .child(SharedString::from(method.name.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .font_family(theme::MONO_FONT)
                        .child(elided(id, method.id.clone(), theme::text_faint(), micro())),
                ),
        )
        .children(method.description.as_ref().map(|description| {
            div()
                .text_size(micro())
                .text_color(theme::text_faint())
                .child(SharedString::from(description.clone()))
        }));
    if method.default {
        lines = lines.child(
            div()
                .text_size(micro())
                .text_color(theme::text_muted())
                .child(SharedString::from("default")),
        );
    }

    div()
        .flex()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(12.))
                .flex_none()
                .text_size(meta())
                .text_color(if method.default {
                    theme::success()
                } else {
                    theme::text_faint()
                })
                .child(SharedString::from(if method.default {
                    "\u{2713}"
                } else {
                    "\u{2014}"
                })),
        )
        .child(lines)
        .into_any_element()
}

/// The column every case of the panel is laid out in, so the two empty states sit where the rows
/// they stand in for would.
fn block() -> gpui::Div {
    div().flex().flex_col().gap_1()
}

/// A sentence in place of a list. Faint, because it is an answer about the harness rather than one
/// of the harness's own answers.
fn note(text: &str) -> AnyElement {
    div()
        .text_size(meta())
        .text_color(theme::text_faint())
        .child(SharedString::from(text.to_string()))
        .into_any_element()
}

fn meta() -> gpui::Pixels {
    theme::font(Family::Chrome, Role::Meta)
}

fn micro() -> gpui::Pixels {
    theme::font(Family::Chrome, Role::Micro)
}
