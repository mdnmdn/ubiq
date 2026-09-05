//! One live agent's conversation, drawn once for every surface that shows one.
//!
//! There is a single interface for talking to an agent, and this is it. The agents screen's
//! columns host it today; the chat panel and the style reference are the next two, and neither
//! needs a second renderer to do it — what differs between hosts is [`ConversationView`], which
//! says whether the footer and the composer come with it and which of the window's pooled
//! composers to type into.
//!
//! **Nothing here knows which screen it is inside.** It is handed a [`Conversation`] and draws it:
//! no column, no tab, no slot of the agents screen's own arrangement reaches in. That is the whole
//! constraint, and it is what lets a second host adopt this by passing a different view.
//!
//! Nothing here appends to a transcript either. The composer sends and the line appears when the
//! harness echoes it back — an interface that draws its own half of a conversation is inventing
//! the other half too.

use gpui::{
    AnyElement, Context, ElementId, Focusable, InteractiveElement, IntoElement, ParentElement,
    Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_component::input::Textarea;
use gpui_component::text::TextView;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::conversation::{ConfigChoice, ConfigValue, ToolContent, ToolKind, ToolStatus};
use ubiq_proto::work::{Activity, AgentId};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::conversation::{ConvBlock, Conversation, Pending, QueuedMessage, Run};
use crate::theme;
use crate::ui::kit::menu::MENU_ANCHOR_UP;
use crate::ui::kit::{
    ContextItem, HARNESS_GLYPH, Picker, PickerStyle, confirm_modal, context_menu, field,
    ghost_button, mono, pill, progress_ring, state_chip, status_dot,
};
use crate::ui::work::activity_colour;
use crate::ui::{handler, indexed};

/// What differs between the surfaces that host a conversation.
pub struct ConversationView {
    /// What every element id inside is built from, so two conversations on screen at once do not
    /// collide. A prefix rather than an [`ElementId`], because the ids under it are composed.
    pub id: SharedString,
    /// Which of the window's pooled composer fields this surface types into. An index, and
    /// nothing else: which surface owns which slot is the host's question.
    pub slot: usize,
    pub footer: bool,
    pub composer: bool,
    /// Whether this surface draws the lifecycle strip — the status glyph and the three-dots menu
    /// — itself. The agents column keeps it; the chat panel draws the same two controls (via
    /// [`lifecycle_controls`]) inline in its own toolbar row instead, so it sets this to `false`
    /// rather than showing the strip twice.
    pub header: bool,
}

impl ConversationView {
    fn eid(&self, part: &str) -> ElementId {
        ElementId::Name(format!("{}-{part}", self.id).into())
    }
}

pub fn render(
    app: &AppState,
    conversation: &Conversation,
    view: ConversationView,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;

    let mut root = div().flex().flex_col().flex_1().min_h(px(0.));
    if view.header {
        root = root.child(lifecycle_header(app, conversation, &view, cx));
    }
    root = root.child(transcript(conversation, &view, cx));

    if let Some(pending) = &conversation.pending {
        root = root.child(permission(id, pending, &view, cx));
    }
    if let Some(error) = &conversation.error {
        root = root.child(
            div()
                .px_3()
                .py_1p5()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .bg(theme::danger_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::danger())
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .with_size(Size::XSmall)
                        .text_color(theme::danger()),
                )
                .child(mono(error.clone(), theme::text()).text_size(px(11.5))),
        );
    }
    if view.footer {
        root = root.child(footer(conversation));
    }
    if view.composer {
        root = root.child(composer(app, conversation, &view, window, cx));
    }

    // Delete is destructive and irreversible — the run directory and its seeded credentials go
    // with it — so it is confirmed rather than fired on the click.
    if app.workbench.confirm_end_conversation == Some(id) {
        let entity = cx.entity();
        root = root.child(confirm_modal(
            "conversation-delete-confirm",
            "Delete conversation",
            "Delete this conversation? Its transcript and run directory \u{2014} seeded \
             credentials included \u{2014} go with it. This cannot be undone.",
            "Delete",
            true,
            handler(&entity, move |this, _, cx| {
                this.confirm_end_conversation(cx)
            }),
            handler(&entity, move |this, _, cx| {
                this.dismiss_end_conversation_confirm(cx)
            }),
            window,
        ));
    }

    root.into_any_element()
}

/// The conversation's state, as one glyph reads it — derived rather than stored, so `Conversation`
/// carries no field for it beside `launched`, `run`, `blocks` and `accepts_input`: those are the one
/// source of truth, and [`lifecycle`] is the one place that reads them into a single answer.
///
/// **`Unloaded` and `Starting` are both `!launched`.** What tells them apart is `blocks`, not a flag
/// of its own — a harness that is gone still leaves what it said; a harness never started leaves
/// nothing. That is why [`lifecycle`] tests the transcript rather than adding a second flag next to
/// `launched`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lifecycle {
    /// A harness is being chosen: nothing has run, and its config has not arrived yet.
    Starting,
    /// Config is in hand and nothing blocks the next turn from launching one.
    Ready,
    /// A turn is in flight. Carries which kind, so the glyph reads Thinking, Writing, Tools or
    /// Needs you rather than flattening every turn to one look.
    Working(Activity),
    /// Loaded, and waiting on the next turn.
    Idle,
    /// The harness is gone; the transcript is not.
    Unloaded,
    /// Takes no more turns.
    Ended,
}

impl Lifecycle {
    /// The one or two words the tooltip says. Never a sentence — the whole point of a glyph
    /// standing in for the prose P7 drew above the composer.
    pub fn label(self) -> String {
        match self {
            Lifecycle::Starting => "Starting".to_string(),
            Lifecycle::Ready => "Ready".to_string(),
            Lifecycle::Working(activity) => format!("Working \u{b7} {}", activity.label()),
            Lifecycle::Idle => "Idle".to_string(),
            Lifecycle::Unloaded => "Unloaded".to_string(),
            Lifecycle::Ended => "Ended".to_string(),
        }
    }
}

/// Read the conversation's own fields into the one state the glyph draws.
///
/// Order matters: ended outranks everything (a harness taking no more turns is not "working" just
/// because a race left `run` behind), a turn in flight outranks idle, and only once neither applies
/// does whether it has ever launched — and, if not, whether it has a transcript — decide the rest.
pub fn lifecycle(conversation: &Conversation) -> Lifecycle {
    if conversation.run == Run::Ended || !conversation.accepts_input {
        return Lifecycle::Ended;
    }
    if conversation.run == Run::Working {
        return Lifecycle::Working(conversation.activity());
    }
    if conversation.launched {
        return Lifecycle::Idle;
    }
    if !conversation.blocks.is_empty() {
        return Lifecycle::Unloaded;
    }
    if conversation.config.is_empty() {
        Lifecycle::Starting
    } else {
        Lifecycle::Ready
    }
}

/// Which of the four lifecycle-menu rows apply, in the order the menu draws them — Stop, Unload,
/// Resume, Delete. A pure reading of the conversation's own state, pulled out of [`lifecycle_header`]
/// so the enable/disable rule is testable on its own: Stop only while a turn is running, Unload
/// only while launched, Resume only while not, Delete always (ending applies whatever the state).
pub fn lifecycle_menu_enabled(conversation: &Conversation) -> [bool; 4] {
    [
        conversation.run != Run::Idle,
        conversation.launched,
        !conversation.launched,
        true,
    ]
}

/// The bordered strip the agents column draws above its transcript: [`lifecycle_controls`] inside
/// a header row of its own. The chat panel draws the same controls (see
/// [`crate::ui::chat::sidebar::header`]) inline in its own toolbar instead of this strip, which is
/// why `view.header` gates whether [`render`] calls this at all.
fn lifecycle_header(
    app: &AppState,
    conversation: &Conversation,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    div()
        .h(px(28.))
        .px_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .border_b_1()
        .border_color(theme::border())
        .debug_selector(|| "lifecycle-strip".into())
        .child(lifecycle_controls(app, conversation, view, cx))
        .into_any_element()
}

/// The status glyph and the three-dots lifecycle menu — Stop, Unload, Resume, Delete — together,
/// as a fragment with no strip of its own around them. [`lifecycle_header`] wraps this in the
/// agents column's bordered row; the chat panel's toolbar drops it straight into its one row of
/// controls instead, beside New chat and New tab. One function either way, so the two surfaces can
/// never disagree about which lifecycle state is shown or which menu row is enabled.
///
/// Each menu item disables rather than hides, so the menu's shape never changes under the cursor.
pub fn lifecycle_controls(
    app: &AppState,
    conversation: &Conversation,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    let entity = cx.entity();

    let enabled = lifecycle_menu_enabled(conversation);
    let labels = ["Stop", "Unload", "Resume", "Delete"];
    let items: Vec<ContextItem> = labels
        .into_iter()
        .zip(enabled)
        .map(|(label, enabled)| {
            let item = ContextItem::new(label);
            if enabled { item } else { item.disabled() }
        })
        .collect();

    let button = div()
        .id(view.eid("lifecycle"))
        .h(px(20.))
        .w(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(IconName::EllipsisVertical)
                .with_size(Size::XSmall)
                .text_color(theme::text_muted()),
        )
        .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
            let at = event.position();
            this.open_conversation_menu(id, (at.x.into(), at.y.into()), cx);
        }))
        .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new("Conversation actions").build(window, cx)
        });

    let mut row = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(lifecycle_glyph(conversation, view))
        .child(button);

    if app.workbench.open_menu == Some(MenuId::ConversationLifecycle(id)) {
        let at = app.workbench.conversation_menu.unwrap_or_default();
        row = row.child(context_menu(
            view.eid("lifecycle-menu"),
            point(px(at.0), px(at.1)),
            items,
            indexed(&entity, move |this, index, _window, cx| {
                this.pick_conversation_menu(id, index, cx);
            }),
            handler(&entity, |this, _, cx| this.dismiss_conversation_menu(cx)),
        ));
    }

    row.into_any_element()
}

/// The one glyph that says what P7's muted sentence used to say in prose — beside the three-dots
/// menu rather than above the composer, and the word itself moved into the tooltip.
///
/// No new kit primitive: a `status_dot` is what every other state mark in the window already is —
/// a tab's dot, the sidebar's dot — so this is that same mark, coloured and captioned by
/// [`Lifecycle`].
fn lifecycle_glyph(conversation: &Conversation, view: &ConversationView) -> AnyElement {
    let state = lifecycle(conversation);
    let colour = lifecycle_colour(state);
    let label = state.label();
    div()
        .id(view.eid("lifecycle-glyph"))
        .child(status_dot(colour, theme::pane_bg()))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
        })
        .into_any_element()
}

/// The colour a lifecycle glyph draws. `Working` carries `Activity`'s own reading rather than
/// flattening every turn to one colour; the rest borrow the same tokens the bucket colours already
/// use, so a glyph never invents a fourth meaning for a colour the window already assigns one.
fn lifecycle_colour(state: Lifecycle) -> Rgba {
    match state {
        Lifecycle::Starting | Lifecycle::Ended => theme::text_faint(),
        Lifecycle::Ready | Lifecycle::Idle => theme::info(),
        Lifecycle::Working(activity) => activity_colour(activity),
        Lifecycle::Unloaded => theme::warning(),
    }
}

/// What has been said, oldest first.
fn transcript(
    conversation: &Conversation,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    let blocks: Vec<AnyElement> = conversation
        .blocks
        .iter()
        .enumerate()
        .map(|(ix, block)| match block {
            ConvBlock::User(text) => user_turn(text),
            ConvBlock::Agent(body) => TextView::markdown(
                view.eid(&format!("md-{ix}")),
                SharedString::from(body.clone()),
            )
            .into_any_element(),
            ConvBlock::Thought(body) => thought(body),
            ConvBlock::Tool { call, open } => tool_block(id, ix, call, *open, view, cx),
        })
        .collect();

    div()
        .id(view.eid("transcript"))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .py_2()
        .gap_2()
        .text_size(px(13.5))
        .text_color(theme::text())
        .overflow_y_scroll()
        .children(if blocks.is_empty() {
            vec![
                mono("nothing said yet", theme::text_faint())
                    .text_size(px(11.5))
                    .into_any_element(),
            ]
        } else {
            blocks
        })
        .into_any_element()
}

/// What the user said sits in the accent, the way every other surface in the window draws a turn
/// of theirs.
fn user_turn(text: &str) -> AnyElement {
    div()
        .pl_6()
        .flex_none()
        .child(
            div()
                .p_2()
                .bg(theme::accent_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent())
                .text_size(px(13.))
                .text_color(theme::text())
                .child(SharedString::from(text.to_string())),
        )
        .into_any_element()
}

/// Reasoning, quieter than prose: it is what the agent thought on the way to what it said, and it
/// must not read as the answer.
fn thought(body: &str) -> AnyElement {
    div()
        .p_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::text_faint())
        .child(mono("THINKING", theme::text_faint()).text_size(px(10.5)))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child(SharedString::from(body.to_string())),
        )
        .into_any_element()
}

/// The colour a tool block is filed under, on the four readings the chat panel already uses: a
/// look is informational, a change is a change, a removal is destructive, a command ran. The ten
/// ACP kinds share them rather than growing ten tokens nobody could tell apart.
fn tool_colour(kind: ToolKind) -> Rgba {
    match kind {
        ToolKind::Read | ToolKind::Search | ToolKind::Fetch => theme::accent(),
        ToolKind::Edit | ToolKind::Move => theme::warning(),
        ToolKind::Delete => theme::danger(),
        ToolKind::Execute => theme::success(),
        ToolKind::Think | ToolKind::SwitchMode => theme::info(),
        ToolKind::Other => theme::text_muted(),
    }
}

fn status_colour(status: ToolStatus) -> Rgba {
    match status {
        ToolStatus::Pending => theme::text_faint(),
        ToolStatus::InProgress => theme::accent(),
        ToolStatus::Completed => theme::success(),
        ToolStatus::Failed => theme::danger(),
    }
}

fn status_label(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "pending",
        ToolStatus::InProgress => "running",
        ToolStatus::Completed => "done",
        ToolStatus::Failed => "failed",
    }
}

/// A tool's title: plain for most kinds, a code chip for a command. A command reads the way one
/// does in the app's own markdown — monospace on a raised surface — because the title has no room
/// for the coloured edge every other surface here is identified by.
fn tool_title(kind: ToolKind, title: String) -> gpui::Div {
    let text = mono(title, theme::text()).text_size(px(12.));
    if kind != ToolKind::Execute {
        return text;
    }
    div()
        .bg(theme::surface_raised())
        .px_1()
        .py(px(1.))
        .child(text)
}

/// A tool call: what it did, to what, and how it went — before any of what it produced.
fn tool_block(
    agent: AgentId,
    index: usize,
    call: &ubiq_proto::conversation::ToolCallRecord,
    open: bool,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = tool_colour(call.kind);
    let expandable = !call.content.is_empty();
    let call_id = call.id.clone();

    // A title that wraps — a long command, most often — grows the row rather than being
    // clipped to one line's height: `min_h` is the floor, not the ceiling, and `items_start`
    // keeps the icon and the status pinned to the first line instead of drifting to the
    // paragraph's centre.
    let mut header = div()
        .id(view.eid(&format!("tool-{index}")))
        .min_h(px(30.))
        .px_2()
        .py_1()
        .flex()
        .flex_none()
        .items_start()
        .gap_2()
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::XSmall)
            .text_color(if expandable {
                theme::text_faint()
            } else {
                theme::border()
            })
            .mt(px(2.)),
        )
        .child(
            mono(call.kind.label(), colour)
                .text_size(px(11.5))
                .mt(px(1.)),
        )
        .child(
            tool_title(call.kind, call.title.clone())
                .flex_1()
                .min_w(px(0.)),
        )
        .child(
            mono(status_label(call.status), status_colour(call.status))
                .text_size(px(11.5))
                .mt(px(1.)),
        );

    // A block with nothing behind it does not expand: a chevron that opens on emptiness says the
    // detail is missing rather than absent.
    if expandable {
        header = header
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_conversation_tool(agent, call_id.clone(), cx);
            }));
    }

    let mut card = div()
        .flex()
        .flex_col()
        .flex_none()
        .bg(theme::pane_bg())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(colour)
        .child(header);

    if open && expandable {
        card = card.child(
            div()
                .px_2()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .border_t_1()
                .border_color(theme::border())
                .children(call.content.iter().map(content).collect::<Vec<_>>()),
        );
    }

    card.into_any_element()
}

fn content(item: &ToolContent) -> AnyElement {
    match item {
        ToolContent::Text(text) => div()
            .flex()
            .flex_col()
            .children(
                text.lines()
                    .map(|line| mono(line.to_string(), theme::text_muted()).text_size(px(11.5)))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        ToolContent::Diff {
            path,
            old_text,
            new_text,
        } => diff(path, old_text.as_deref(), new_text),
    }
}

/// An edit, as the block that went and the block that came. Line for line rather than matched:
/// there is no diff engine here, and a bad alignment reads as changes nobody made.
fn diff(path: &str, old_text: Option<&str>, new_text: &str) -> AnyElement {
    let mut rows: Vec<AnyElement> = vec![
        mono(path.to_string(), theme::text_muted())
            .text_size(px(11.))
            .into_any_element(),
    ];
    if let Some(old) = old_text {
        rows.extend(
            old.lines()
                .map(|line| diff_line("-", line, theme::danger(), theme::danger_soft())),
        );
    }
    rows.extend(
        new_text
            .lines()
            .map(|line| diff_line("+", line, theme::success(), theme::success_soft())),
    );

    div().flex().flex_col().children(rows).into_any_element()
}

fn diff_line(marker: &str, text: &str, fg: Rgba, bg: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .gap_2()
        .px_1()
        .bg(bg)
        .child(mono(marker.to_string(), fg).text_size(px(11.5)))
        .child(
            mono(text.to_string(), fg)
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.5)),
        )
        .into_any_element()
}

/// What the agent is asking to be allowed to do, and the answers it offered.
///
/// Nothing emits one of these today — every bridge auto-approves — but the vocabulary carries the
/// request, and a surface that could not draw it would have to grow one the day a bridge stops.
fn permission(
    agent: AgentId,
    pending: &Pending,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let what = pending
        .tool_call
        .title
        .clone()
        .unwrap_or_else(|| "The agent is asking to go ahead".to_string());

    let buttons: Vec<AnyElement> = pending
        .options
        .iter()
        .enumerate()
        .map(|(ix, option)| {
            let request_id = pending.request_id.clone();
            let option_id = option.option_id.clone();
            ghost_button(
                view.eid(&format!("permission-{ix}")),
                None,
                option.name.clone(),
                cx.listener(move |this, _, _, cx| {
                    this.answer_permission(agent, request_id.clone(), option_id.clone(), cx);
                }),
            )
            .into_any_element()
        })
        .collect();

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_2()
        .bg(theme::warning_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::warning())
        .child(mono("NEEDS YOU", theme::warning()).text_size(px(10.5)))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text())
                .child(SharedString::from(what)),
        )
        .child(div().flex().items_center().gap_2().children(buttons))
        .into_any_element()
}

/// What the harness said about itself: which one it is, which model, what the turn has cost, and
/// how much of the context window is gone.
///
/// The ring is drawn only where a window was reported. A percentage of a size nobody named is a
/// wrong ring, and a wrong ring is worse than none.
fn footer(conversation: &Conversation) -> AnyElement {
    let mut row = div()
        .px_3()
        .py_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .border_t_1()
        .border_color(theme::border())
        .child(
            pill(theme::accent())
                .h(px(22.))
                .px_2()
                .child(mono(HARNESS_GLYPH, theme::text()).text_size(px(11.))),
        );

    // Which identity answered. Read-only by design: it is chosen once, in the New agent menu,
    // because a turn already taken was taken as somebody. Drawn only when the run resolved one
    // — an empty pill beside the harness would claim an identity that does not exist.
    if !conversation.account.is_empty() {
        row = row.child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono(conversation.account.clone(), theme::text_muted()).text_size(px(11.))),
        );
    }
    if let Some(model) = &conversation.model {
        row = row.child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono(model.clone(), theme::text()).text_size(px(11.))),
        );
    }
    if let Some(mode) = &conversation.mode {
        row = row.child(state_chip(mode.clone(), theme::info(), 1.0));
    }

    row = row.child(div().flex_1().min_w(px(0.)));

    if let Some(cost) = conversation.cost_usd() {
        row = row.child(mono(format!("${cost:.2}"), theme::text_muted()).text_size(px(11.)));
    }
    if let Some(pct) = conversation.context_pct() {
        row = row.child(progress_ring(pct, 12.)).child(
            mono(
                format!("{:.1}K ctx", conversation.tokens() as f32 / 1000.0),
                theme::text_muted(),
            )
            .text_size(px(11.)),
        );
    }
    if let Some(pct) = conversation.rate_limit_five_hour_pct() {
        row = row.child(mono(format!("5h {pct}%"), theme::text_muted()).text_size(px(11.)));
    }

    row.into_any_element()
}

/// What one composer picker offers, after the caller's own substring filter — the same list
/// `on_pick`'s index resolves against, so a filtered pick can never name the wrong row. `None` is
/// "the harness has not advertised this config id", drawn as its own sentence rather than an
/// empty picker.
pub struct ConfigRow {
    pub label: String,
    pub selected: usize,
    pub values: Vec<String>,
    pub names: Vec<String>,
}

/// Build one for `config_id` from the conversation's advertised config and whatever is typed
/// into the search field. `search` is matched case-insensitively against each choice's name —
/// `to_lowercase()` and `contains`, the one filter every menu in the window uses; an empty query
/// keeps everything. `None` when the harness has not offered this id at all — a harness with no
/// modes never grows a mode picker.
pub fn config_choices(
    conversation: &Conversation,
    config_id: &str,
    search: &str,
) -> Option<ConfigRow> {
    let option = conversation.config.iter().find(|opt| opt.id == config_id)?;
    let current = match &option.value {
        ConfigValue::Select { current, .. } => current.as_str(),
        ConfigValue::Flag { .. } => "",
    };
    let chosen = conversation
        .chosen
        .get(config_id)
        .map(String::as_str)
        .unwrap_or(current);
    let all: &[ConfigChoice] = match &option.value {
        ConfigValue::Select { choices, .. } => choices,
        ConfigValue::Flag { .. } => &[],
    };
    let query = search.to_lowercase();
    let choices: Vec<&ConfigChoice> = all
        .iter()
        .filter(|choice| choice.name.to_lowercase().contains(&query))
        .collect();
    let selected = choices
        .iter()
        .position(|choice| choice.value == chosen)
        .unwrap_or(0);
    let label = choices
        .get(selected)
        .map(|choice| choice.name.clone())
        .unwrap_or_else(|| chosen.to_string());
    let values = choices.iter().map(|choice| choice.value.clone()).collect();
    let names = choices.iter().map(|choice| choice.name.clone()).collect();
    Some(ConfigRow {
        label,
        selected,
        values,
        names,
    })
}

/// The order the composer's pickers appear in. Fixed rather than read off `conversation.config`'s
/// own order: the host may add ids to that list in whatever order it minted them, but the picker
/// row reads left to right as "what to run as, how hard to think, which mode" every time.
const CONFIG_ORDER: [&str; 3] = ["model", "thinking", "mode"];

/// The field that steers this agent, and the Stop that interrupts it.
fn composer(
    app: &AppState,
    conversation: &Conversation,
    view: &ConversationView,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    // A harness that takes no second turn says so where the field would be, rather than leaving
    // one that swallows what is typed.
    if !conversation.accepts_input {
        return div()
            .px_2()
            .py_1p5()
            .flex()
            .flex_none()
            .border_t_1()
            .border_color(theme::border())
            .child(
                mono(
                    "This agent has ended \u{2014} its transcript stays, and it takes no more turns.",
                    theme::text_faint(),
                )
                .text_size(px(11.5)),
            )
            .into_any_element();
    }

    let Some(input) = app.column_inputs.get(view.slot).cloned() else {
        return div().into_any_element();
    };
    let entity = cx.entity();
    let id = conversation.id;
    let slot = view.slot;
    let can_send = !input.read(cx).value().trim().is_empty();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let working = conversation.run == Run::Working;

    // Before the harness has launched, the composer offers a model picker instead of the
    // read-only pill the footer draws once it has. It sits in the controls row *under* the field,
    // beside Send, rather than in a strip above it: what a turn will run as belongs next to the
    // control that starts it, and a row of its own pushed the field down for a chip. It never
    // blocks typing — the user may send before discovery finishes, and the host then launches with
    // the harness's own default.
    let config_row = (!conversation.launched).then(|| {
        let search = app.picker_search.read(cx).value().to_string();
        let search_focused = app
            .picker_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);

        let pickers: Vec<AnyElement> = CONFIG_ORDER
            .iter()
            .filter_map(|&config_id| {
                // Only the model picker filters — thinking and mode are at most six rows, and a
                // filter field over six rows is furniture.
                let query = if config_id == "model" {
                    search.as_str()
                } else {
                    ""
                };
                let row = config_choices(conversation, config_id, query)?;
                let values = row.values;
                let cid = config_id.to_string();
                let mut picker = Picker::new(view.eid(&format!("{config_id}-picker")), row.label)
                    .style(PickerStyle::Chip)
                    .anchor(MENU_ANCHOR_UP)
                    .items(row.names)
                    .selected(row.selected)
                    .open(conversation.open_config.as_deref() == Some(config_id));
                if config_id == "model" {
                    picker = picker.search(&app.picker_search, search_focused);
                }
                let cid_toggle = cid.clone();
                let cid_pick = cid.clone();
                let picker = picker
                    .on_toggle(handler(&entity, move |this, window, cx| {
                        this.toggle_agent_config_menu(id, cid_toggle.clone(), window, cx)
                    }))
                    .on_pick(indexed(&entity, move |this, index, window, cx| {
                        if let Some(value) = values.get(index) {
                            this.pick_agent_config(id, cid_pick.clone(), value.clone(), window, cx);
                        }
                    }))
                    .on_dismiss(handler(&entity, move |this, window, cx| {
                        this.dismiss_agent_config_menu(id, window, cx)
                    }));
                Some(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .child(picker)
                        .into_any_element(),
                )
            })
            .collect();

        if pickers.is_empty() {
            mono("Discovering models\u{2026}", theme::text_faint())
                .text_size(px(11.5))
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .children(pickers)
                .into_any_element()
        }
    });

    // No keyboard hint. Enter, cmd/ctrl+Enter and shift+Enter are what every text field on the
    // machine already does, and a permanent line of shortcut text under every composer is furniture
    // the user reads once. The row carries what changes instead: what this turn will run as.
    let controls = div()
        .px_1p5()
        .pb_1()
        .pt_0p5()
        .flex()
        .items_center()
        .gap_1p5()
        .children(config_row)
        .child(div().flex_1().min_w(px(0.)));

    // One control, and which it is depends on the turn and the draft: idle sends, a running turn
    // with nothing typed offers Stop, and a running turn with something typed queues it instead
    // of writing into a harness mid-turn — the same three states the Enter key answers through
    // `AppState::send_or_enqueue`, so the button and the key never disagree.
    let action = if working && !can_send {
        ghost_button(
            view.eid("stop"),
            Some(IconName::Close),
            "Stop",
            cx.listener(move |this, _, _, cx| this.cancel_turn(id, cx)),
        )
        .text_color(theme::danger())
    } else if working {
        ghost_button(
            view.eid("send"),
            Some(IconName::Inbox),
            "Enqueue",
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        )
        .text_color(theme::accent())
    } else {
        ghost_button(
            view.eid("send"),
            Some(IconName::ArrowUp),
            "Send",
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        )
        .text_color(if can_send {
            theme::accent()
        } else {
            theme::text_faint()
        })
    };

    // "Unloaded" is said once already — the glyph beside the three-dots menu, top left of the
    // view, with the word itself in its tooltip. A composer that also spelled it out in prose
    // would be saying the same fact twice, once as a mark and once as a sentence.
    let field_el = field(theme::accent(), focused)
        .flex_none()
        .flex_col()
        .items_stretch()
        .child(
            div()
                .id(view.eid("composer"))
                .px_2()
                .pt_1p5()
                .cursor_text()
                .child(
                    Textarea::new(&input)
                        .appearance(false)
                        .bordered(false)
                        .w_full()
                        .text_size(px(13.)),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let input = this.column_inputs[slot].clone();
                    input.update(cx, |state, cx| state.focus(window, cx));
                })),
        )
        .child(controls.child(action));

    let mut extras: Vec<AnyElement> = Vec::new();
    if !conversation.queued.is_empty() {
        extras.push(queue_list(id, slot, view, &conversation.queued, cx));
    }

    if extras.is_empty() {
        field_el.into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .children(extras)
            .child(field_el)
            .into_any_element()
    }
}

/// Prompts typed while a turn was running, oldest first — each with an edit that loads it back
/// into the field and a delete that drops it outright. Drawn only when there is one: an empty
/// queue draws nothing, the same discipline every pill in this file follows.
fn queue_list(
    agent_id: AgentId,
    slot: usize,
    view: &ConversationView,
    queued: &[QueuedMessage],
    cx: &mut Context<AppState>,
) -> AnyElement {
    const PREVIEW_CHARS: usize = 80;

    let rows: Vec<AnyElement> = queued
        .iter()
        .map(|message| {
            let queued_id = message.id;
            let mut preview: String = message.text.chars().take(PREVIEW_CHARS).collect();
            if message.text.chars().count() > PREVIEW_CHARS {
                preview.push('\u{2026}');
            }

            div()
                .id(view.eid(&format!("queued-{queued_id}")))
                .px_2()
                .h(px(26.))
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .bg(theme::surface())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::border())
                .child(
                    mono(preview, theme::text_muted())
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(11.5)),
                )
                .child(ghost_button(
                    view.eid(&format!("queued-edit-{queued_id}")),
                    Some(IconName::Replace),
                    "Edit",
                    cx.listener(move |this, _, window, cx| {
                        this.edit_queued_message(agent_id, slot, queued_id, window, cx);
                    }),
                ))
                .child(ghost_button(
                    view.eid(&format!("queued-delete-{queued_id}")),
                    Some(IconName::Delete),
                    "Delete",
                    cx.listener(move |this, _, _, cx| {
                        this.delete_queued_message(agent_id, queued_id, cx);
                    }),
                ))
                .into_any_element()
        })
        .collect();

    div()
        .px_2()
        .pt_1()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1()
        .children(rows)
        .into_any_element()
}
