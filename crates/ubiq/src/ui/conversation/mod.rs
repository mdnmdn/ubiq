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

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, ElementId, Focusable, InteractiveElement,
    IntoElement, ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window,
    anchored, deferred, div, point, px,
};
use gpui_component::input::Textarea;
use gpui_component::text::TextView;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::conversation::{ConfigChoice, ConfigValue, ToolContent, ToolKind, ToolStatus};
use ubiq_proto::work::{Activity, AgentId};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::conversation::{
    ConvBlock, Conversation, Pending, QueuedMessage, Run, short_model_label,
};
use crate::theme;
use crate::ui::kit::menu::{MENU_ANCHOR_UP, MENU_LAYER};
use crate::ui::kit::{
    ContextItem, HARNESS_GLYPH, Picker, PickerStyle, confirm_modal, context_menu, ghost_button,
    icon_button, mono, pill, progress_ring, status_dot,
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

    let subagents = conversation.subagents();

    let mut root = div().flex().flex_col().flex_1().min_h(px(0.));
    if view.header {
        root = root.child(lifecycle_header(app, conversation, &view, cx));
    }
    if let Some(subagent) = conversation.viewing_subagent() {
        let tab = subagents.iter().find(|tab| tab.id == subagent);
        root = root.child(reading_strip(
            &conversation.subagent_name(subagent),
            tab.and_then(|tab| tab.model.as_deref())
                .map(|model| short_model_label(&conversation.harness, model)),
        ));
    }
    root = root.child(transcript(app, conversation, &view, cx));

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
    // One rule, above both. The footer and the composer are one block at the bottom of the view
    // — not two boxes — so the separator belongs to the block rather than to either half.
    if view.footer || view.composer {
        let mut bottom = div()
            .flex()
            .flex_col()
            .flex_none()
            .border_t_1()
            .border_color(theme::border());
        // Topmost in the block, above the footer as well as the composer: it opens upward over
        // the transcript, so nothing under it moves when it does. Only where a subagent exists —
        // a conversation that spawned none looks exactly as it did before, the same discipline
        // every pill in this file follows.
        if !subagents.is_empty() {
            bottom = bottom.child(agent_switcher(conversation, &subagents, &view, cx));
        }
        if view.footer {
            bottom = bottom.child(footer(conversation, &view));
        }
        if view.composer {
            bottom = bottom.child(composer(app, conversation, &view, window, cx));
        }
        root = root.child(bottom);
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

/// The state mark on its own, for a surface that puts it somewhere other than beside the menu.
///
/// The chat tab's head reads left to right — what the conversation *is*, then what it is *on*,
/// then what can be *done to it* — so it wants the two halves of [`lifecycle_controls`] at
/// opposite ends of one row rather than as a pair. Same glyph, same tooltip, same colours: this
/// is the shared fragment, not a second one.
pub fn lifecycle_mark(conversation: &Conversation, view: &ConversationView) -> AnyElement {
    lifecycle_glyph(conversation, view)
}

/// The three-dots menu on its own, the other half of [`lifecycle_controls`].
pub fn lifecycle_menu(
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

    let mut row = div().flex().flex_none().items_center().child(
        div()
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
            }),
    );

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

/// A cheap reading of the transcript's tail: how many blocks there are, and how long the last one
/// is.
///
/// A streaming chunk lengthens the last block, so this moves on every token without hashing the
/// whole transcript once a frame — and it does *not* move when nothing was said, which is what
/// lets [`transcript`] follow the tail without dragging a reader who scrolled up back down.
fn tail_signature(conversation: &Conversation) -> u64 {
    let tail = match conversation.blocks.last() {
        Some(
            ConvBlock::User(text)
            | ConvBlock::Agent { body: text, .. }
            | ConvBlock::Thought { body: text, .. },
        ) => text.len(),
        Some(ConvBlock::Tool { call, open }) => {
            call.title.len() + call.content.len() + usize::from(*open)
        }
        None => 0,
    };
    (conversation.blocks.len() as u64).wrapping_mul(1_000_003) ^ tail as u64
}

/// What has been said, oldest first — and, when anything new has landed, scrolled to.
fn transcript(
    app: &AppState,
    conversation: &Conversation,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    let root = cx.entity();
    // One agent's turns, never two interleaved — and the indices are the real ones, because the
    // element ids and the tool-toggle listener both key off a block's position in `blocks`.
    let blocks: Vec<AnyElement> = conversation
        .visible_blocks()
        .into_iter()
        .map(|(ix, block)| match block {
            ConvBlock::User(text) => copyable(view, ix, text, user_turn(text)),
            ConvBlock::Agent { body, .. } => copyable(
                view,
                ix,
                body,
                TextView::markdown(
                    view.eid(&format!("md-{ix}")),
                    SharedString::from(body.clone()),
                )
                .on_link_click(crate::ui::on_link(root.clone(), None))
                .into_any_element(),
            ),
            ConvBlock::Thought { body, .. } => copyable(view, ix, body, thought(body)),
            ConvBlock::Tool { call, open } => {
                // A delegation is a way in to the agent it spawned, and the way in exists only
                // once that agent has said something: the instance id *is* this call's id.
                let delegate = (call.kind == ToolKind::Delegate
                    && conversation.has_subagent(&call.id))
                .then(|| call.id.clone());
                tool_block(id, ix, call, *open, delegate, view, cx)
            }
        })
        .collect();

    let mut root = div()
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
        .overflow_y_scroll();

    // Follow the tail, and only the tail: the handle is scrolled down when the signature it last
    // followed has changed, so a quiet conversation the reader has scrolled up in stays where they
    // put it.
    if let Some((handle, followed)) = app.transcript_scrolls.get(view.slot) {
        let signature = tail_signature(conversation);
        if followed.get() != signature {
            followed.set(signature);
            handle.scroll_to_bottom();
        }
        root = root.track_scroll(handle);
    }

    root.children(if blocks.is_empty() {
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

/// One message, with its own copy control in the lower right — hidden until the pointer is over
/// the message, because a control on every line of a transcript reads as a toolbar rather than a
/// conversation. The clipboard gets the block's own text, which is what the harness said or
/// received rather than anything rendered over it.
fn copyable(view: &ConversationView, ix: usize, text: &str, body: AnyElement) -> AnyElement {
    let group = SharedString::from(format!("{}-msg-{ix}", view.id));
    let text = text.to_string();
    div()
        .relative()
        .flex()
        .flex_none()
        .flex_col()
        .group(group.clone())
        .child(body)
        .child(
            div()
                .absolute()
                .bottom_0()
                .right_0()
                .invisible()
                .group_hover(group, |this| this.visible())
                .child(
                    icon_button(
                        view.eid(&format!("copy-{ix}")),
                        IconName::Copy,
                        false,
                        move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        },
                    )
                    .size(px(22.))
                    .bg(theme::surface()),
                ),
        )
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
/// must not read as the answer. Whose thought it was is not marked here any more \u{2014} the
/// transcript shows one agent at a time, and [`reading_strip`] names it once above the whole of it.
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

/// The one line above a subagent's transcript that says whose turns are on screen, and what they
/// are being answered with. Drawn only while a subagent is being read: the main agent's own
/// transcript is the default and needs no caption to say so.
///
/// The model is the delegate's own, shortened by [`short_model_label`] — the composer's chip
/// shortens the parent's the same way, so one conversation never spells a model two ways — and a
/// delegate the harness named no model for draws nothing rather than a placeholder, exactly as the
/// footer's pills do.
fn reading_strip(name: &str, model: Option<String>) -> AnyElement {
    div()
        .px_3()
        .py_1()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .bg(theme::surface())
        .border_b_1()
        .border_color(theme::border())
        .debug_selector(|| "reading-strip".into())
        .child(mono("\u{21b3}", theme::info()).text_size(px(10.5)))
        .child(mono(name.to_string(), theme::info()).text_size(px(11.5)))
        .children(model.map(|model| mono(model, theme::text_faint()).text_size(px(11.))))
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
        // A delegation is where the work went, so it reads as its own thing rather than borrowing
        // the colour of a thought.
        ToolKind::Delegate => theme::accent_muted(),
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
/// `delegate` is the subagent this block is the entry point to, where it is one: a `Delegate` call
/// whose spawned agent has spoken. Then the block switches the transcript instead of unfolding its
/// own detail — what a reader wants from a delegation is the other transcript, not the summary of
/// it. A delegation with no agent behind it yet stays inert.
fn tool_block(
    agent: AgentId,
    index: usize,
    call: &ubiq_proto::conversation::ToolCallRecord,
    open: bool,
    delegate: Option<String>,
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
        );

    header = header.child(
        mono(status_label(call.status), status_colour(call.status))
            .text_size(px(11.5))
            .mt(px(1.)),
    );

    if let Some(target) = delegate {
        header = header
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.view_conversation_agent(agent, Some(target.clone()), cx);
            }))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Read this agent's transcript")
                    .build(window, cx)
            });
    // A block with nothing behind it does not expand: a chevron that opens on emptiness says the
    // detail is missing rather than absent.
    } else if expandable {
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

/// A short label with the whole of itself on hover — the shape every readout in the footer and
/// the composer's picker row takes, because the row has space for a mark and none for a number.
fn tipped(id: ElementId, label: String, tip: String, colour: Rgba) -> AnyElement {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .child(mono(label, colour).text_size(px(11.)))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
        .into_any_element()
}

/// The whole of what the `tot` readout stands for: what the three letters mean, then the five ways
/// a token is billed, then what each spawned subagent spent of it.
///
/// The first clause is the distinction the footer lives or dies by. `tot` is a **flow** — summed
/// over every turn, only ever growing — where the ring beside it is a **level** that falls the
/// moment the conversation is compacted. A three-letter label is only honest if hovering it says
/// which of the two it is.
///
/// One line, `·`-separated, like every other tooltip in the row — and the subagent half is drawn
/// only where a subagent spent something, so a conversation that spawned none reads exactly as it
/// did before.
fn spend_tip(conversation: &Conversation) -> String {
    let Some(spend) = conversation.spend else {
        return String::new();
    };
    let mut tip = format!(
        "Total spent \u{2014} every token this conversation has billed, subagents included. \
         It only grows; the ring beside it is what is in the window now, and that can fall. \
         \u{b7} {} tokens \u{b7} in {} \u{b7} out {} \u{b7} thinking {} \u{b7} cache read {} \
         \u{b7} cache creation {}",
        spend.total(),
        spend.input,
        spend.output,
        spend.thinking,
        spend.cache_read,
        spend.cache_creation,
    );
    for (name, total) in conversation.subagent_spend() {
        tip.push_str(&format!(" \u{b7} {name} {total}"));
    }
    tip
}

/// What the harness said about itself: which one it is and as whom, what it has spent, and how much
/// of the context window is gone.
///
/// **No numbers in the row that a mark can carry.** The money and the rate-limit windows are gone
/// outright, and what is left says the least it can on screen and the whole of it on hover — the
/// footer is glanced at, not read. What a turn *runs as* is not here at all any more: the model,
/// the thinking level and the mode are the composer's own pickers, live for the conversation's
/// whole life, and a read-only pill beside them would be the same fact drawn twice.
///
/// The ring is drawn only where a window was reported. A percentage of a size nobody named is a
/// wrong ring, and a wrong ring is worse than none.
fn footer(conversation: &Conversation, view: &ConversationView) -> AnyElement {
    // Which harness, and which identity answered — one chip, because they are one answer: this
    // conversation is *that* harness signed in as *that* person. Read-only by design: it is chosen
    // once, in the New agent menu, because a turn already taken was taken as somebody.
    let (identity, mut identity_tip) = if conversation.account.is_empty() {
        (
            HARNESS_GLYPH.to_string(),
            format!(
                "{} \u{2014} no account, running as you",
                conversation.harness
            ),
        )
    } else {
        (
            format!("{HARNESS_GLYPH} {}", conversation.account),
            format!(
                "{} \u{b7} signed in as {} \u{2014} chosen once, when the agent was started",
                conversation.harness, conversation.account
            ),
        )
    };
    // Whether this identity may spend past its plan is a fact about the account, so it hangs off
    // the account chip rather than off a banner of its own: the `5h N%` readout was removed
    // deliberately and this does not bring it back. Said only when the answer is no — an account
    // that can still spill over has nothing to warn about.
    if let Some(rate) = &conversation.rate_limit
        && rate
            .overage_status
            .as_deref()
            .is_some_and(|status| status != "allowed")
    {
        identity_tip.push_str(" \u{b7} overage ");
        identity_tip.push_str(rate.overage_status.as_deref().unwrap_or_default());
        if let Some(reason) = &rate.overage_reason {
            identity_tip.push_str(&format!(" ({reason})"));
        }
    }

    let mut row = div()
        .px_3()
        .py_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(
            pill(theme::accent())
                .h(px(22.))
                .px_2()
                .id(view.eid("identity"))
                .child(mono(identity, theme::text()).text_size(px(11.)))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(identity_tip.clone()).build(window, cx)
                }),
        )
        .child(div().flex_1().min_w(px(0.)));

    // Everything the conversation has spent, which is not what is in the window: a compacted
    // conversation has spent millions and holds thousands. Drawn only where the harness counts it
    // — a pill with nothing behind it is not drawn.
    if let Some(total) = conversation.total_tokens() {
        row = row.child(tipped(
            view.eid("total-tokens"),
            format!("{:.1}K tot", total as f32 / 1000.0),
            spend_tip(conversation),
            theme::text_muted(),
        ));
    }

    if let Some(pct) = conversation.context_pct() {
        let used = conversation.tokens();
        let size = conversation.usage.as_ref().map_or(0, |usage| usage.size);
        // The ring and the count are one fact drawn twice — how full the window is right now — so
        // they say the same sentence on hover. A level, not a total: it falls when the harness
        // compacts, which is exactly what tells it apart from `tot` beside it.
        let tip = format!(
            "Context window \u{2014} {used} of {size} tokens in it right now, {pct}% full. \
             A level, not a total: it falls when the conversation is compacted."
        );
        let ring_tip = tip.clone();
        row = row
            .child(
                div()
                    .id(view.eid("context-ring"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(progress_ring(pct, 12.))
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(ring_tip.clone()).build(window, cx)
                    }),
            )
            .child(tipped(
                view.eid("context-tokens"),
                format!("{:.1}K ctx", used as f32 / 1000.0),
                tip,
                theme::text_muted(),
            ));
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

/// The composer's one action, drawn the way [`crate::ui::kit::primary_button`] draws a screen's
/// one obvious action: filled, square, and the only solid block in the view.
///
/// Icon only. Send, Enqueue and Stop are one control in three states, and a word that changes
/// width would move the button out from under the pointer as a turn starts; the word is the
/// tooltip instead. Nothing to send drains the fill rather than removing the button, so the row
/// never changes shape.
fn action_button(
    id: ElementId,
    icon: IconName,
    label: &'static str,
    fill: Rgba,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(26.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(if enabled {
            fill
        } else {
            theme::fade(fill, 0.35)
        })
        .child(
            Icon::new(icon)
                .with_size(Size::XSmall)
                .text_color(theme::on_accent()),
        )
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(theme::fade(fill, 0.8)))
                .on_click(on_click)
        })
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(label).build(window, cx))
        .into_any_element()
}

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
    let working = conversation.run == Run::Working;

    // What this turn runs as, for the conversation's whole life rather than only before it
    // launched: `SetAgentConfig` is answered by a running harness as readily as by a pending one,
    // and a level nobody can see or change is a level nobody knows they are paying for. The row
    // sits *under* the field, beside Send — what a turn will run as belongs next to the control
    // that starts it — and never blocks typing: the user may send before discovery finishes, and
    // the host then launches with the harness's own default.
    let config_row = {
        let search = app.picker_search.read(cx).value().to_string();
        let search_focused = app
            .picker_search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);

        // What the thinking picker currently says, read once: the model chip wears its first
        // letter, so the two are never out of step.
        let thinking = config_choices(conversation, "thinking", "").map(|row| row.label);
        let letter = thinking.as_deref().and_then(|level| {
            level
                .chars()
                .next()
                .map(|first| first.to_uppercase().to_string())
        });

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

                // The chip says the least that identifies the choice; the tooltip says the whole
                // of it. A model id is `vendor-family-version-date` and a thinking level is a
                // word, and neither fits a row that also has to hold a field.
                let (label, tip) = match config_id {
                    "model" => {
                        let short = short_model_label(&conversation.harness, &row.label);
                        match (&letter, &thinking) {
                            (Some(letter), Some(level)) => (
                                format!("{short} \u{b7} {letter}"),
                                format!("{} \u{b7} {level} thinking", row.label),
                            ),
                            _ => (short, row.label.clone()),
                        }
                    }
                    "thinking" => (
                        letter.clone().unwrap_or_else(|| row.label.clone()),
                        format!("{} thinking", row.label),
                    ),
                    _ => (
                        row.label.clone(),
                        format!("{} \u{b7} permission mode", row.label),
                    ),
                };

                let mut picker = Picker::new(view.eid(&format!("{config_id}-picker")), label)
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
                        .id(view.eid(&format!("{config_id}-chip")))
                        .flex()
                        .flex_none()
                        .items_center()
                        .child(picker)
                        .tooltip(move |window, cx| {
                            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                        })
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
    };

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
        .child(config_row)
        // Files, as the harness reads a reference to one: `@path`, project-relative. The picker
        // is the window's own, raised over the explorer's tree; with no project open there is no
        // tree and the button does nothing.
        .child(
            icon_button(
                view.eid("attach"),
                IconName::Plus,
                false,
                cx.listener(move |this, _, window, cx| {
                    this.raise_composer_picker(id, slot, window, cx)
                }),
            )
            .size(px(26.))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Attach files").build(window, cx)
            }),
        )
        .child(div().flex_1().min_w(px(0.)));

    // One control, and which it is depends on the turn and the draft: idle sends, a running turn
    // with nothing typed offers Stop, and a running turn with something typed queues it instead
    // of writing into a harness mid-turn — the same three states the Enter key answers through
    // `AppState::send_or_enqueue`, so the button and the key never disagree.
    let action = if working && !can_send {
        action_button(
            view.eid("stop"),
            IconName::Close,
            "Stop",
            theme::danger(),
            true,
            cx.listener(move |this, _, _, cx| this.cancel_turn(id, cx)),
        )
    } else if working {
        action_button(
            view.eid("send"),
            IconName::Inbox,
            "Enqueue",
            theme::accent(),
            can_send,
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        )
    } else {
        action_button(
            view.eid("send"),
            IconName::ArrowUp,
            "Send",
            theme::accent(),
            can_send,
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        )
    };

    // "Unloaded" is said once already — the glyph beside the three-dots menu, top left of the
    // view, with the word itself in its tooltip. A composer that also spelled it out in prose
    // would be saying the same fact twice, once as a mark and once as a sentence.
    let field_el = div()
        .flex()
        .flex_none()
        .flex_col()
        .items_stretch()
        // Up in an empty field brings the last thing sent back, the way a shell does. Captured
        // above the field, because the field's own `up` moves the cursor and stops there — and it
        // is handed back untouched when there is nothing to recall or something is typed, so a
        // draft of more than one line still navigates.
        .capture_action(
            cx.listener(move |this, _: &gpui_component::input::MoveUp, window, cx| {
                if this.recall_last_message(slot, window, cx) {
                    cx.stop_propagation();
                }
            }),
        )
        .child(
            div()
                .id(view.eid("composer"))
                .debug_selector(|| "composer-field".into())
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

/// How many delegates there are, and — once asked — who they are.
///
/// Collapsed is the resting state: one row saying `3 subagents`, at the top of the bottom block,
/// because a conversation's delegates are a fact worth a line and a list worth asking for. Opening
/// it draws the list *upward*, over the transcript, through the same `anchored` + `deferred` pair
/// every menu in the window uses — the composer must not move when the panel opens, because a
/// control that walks away from the cursor is a control you have to chase.
///
/// One row per agent: the conversation's own turns first, then every subagent it has spawned. Each
/// says who on the left and what it is doing on the right, and clicking one switches the transcript
/// to it. The main agent's row never leaves the list: it is how the reader gets back. Neither half
/// invents vocabulary — the main agent's word is [`Lifecycle::label`]'s and its colour
/// [`lifecycle_colour`]'s, a subagent's are [`status_label`]'s and [`status_colour`]'s, read off
/// the `Task` call that spawned it. A subagent whose spawning call is not in the transcript has no
/// status to read, and says that rather than being claimed to be running.
fn agent_switcher(
    conversation: &Conversation,
    subagents: &[crate::state::conversation::SubagentTab],
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    let viewing = conversation.viewing_subagent();
    let open = conversation.subagents_open;

    let state = lifecycle(conversation);
    let mut rows: Vec<AnyElement> = vec![agent_row(
        view.eid("agent-tag-main"),
        "agent-row-main".to_string(),
        "Main agent".to_string(),
        state.label(),
        // What the main agent runs as is the footer's chip, right below: saying it twice would be
        // the same fact drawn twice.
        None,
        lifecycle_colour(state),
        viewing.is_none(),
        cx.listener(move |this, _, _, cx| {
            this.view_conversation_agent(id, None, cx);
            this.toggle_conversation_subagents(id, cx);
        }),
    )];

    rows.extend(subagents.iter().map(|tab| {
        let target = tab.id.clone();
        let (status, colour) = match tab.status {
            Some(status) => (status_label(status).to_string(), status_colour(status)),
            None => ("unknown".to_string(), theme::text_faint()),
        };
        agent_row(
            view.eid(&format!("agent-tag-{}", tab.id)),
            format!("agent-row-{}", tab.id),
            tab.name.clone(),
            status,
            Some(subagent_tip(conversation, tab)),
            colour,
            viewing == Some(tab.id.as_str()),
            cx.listener(move |this, _, _, cx| {
                this.view_conversation_agent(id, Some(target.clone()), cx);
                this.toggle_conversation_subagents(id, cx);
            }),
        )
    }));

    let count = subagents.len();
    let mut header = div()
        .id(view.eid("agent-switcher-header"))
        .relative()
        .px_2()
        .py_1()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .debug_selector(|| "agent-switcher".into())
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronUp
            })
            .with_size(Size::XSmall)
            .text_color(theme::text_faint()),
        )
        .child(
            mono(
                format!("{count} subagent{}", if count == 1 { "" } else { "s" }),
                theme::text_muted(),
            )
            .text_size(px(11.)),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_conversation_subagents(id, cx);
        }));

    if open {
        header = header.child(
            deferred(
                anchored()
                    .anchor(MENU_ANCHOR_UP)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id(view.eid("agent-switcher-panel"))
                            .min_w(px(240.))
                            .p_1()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .gap_1()
                            .bg(theme::surface_raised())
                            .border_l(px(theme::ACCENT_EDGE))
                            .border_color(theme::accent())
                            .shadow_lg()
                            .debug_selector(|| "agent-switcher-panel".into())
                            .children(rows),
                    ),
            )
            .priority(MENU_LAYER),
        );
    }

    header.into_any_element()
}

/// What a delegate *is*, for the row's hover: the type the harness named, what it is answering
/// with, and what effort it runs at.
///
/// Each clause is drawn only where the harness said it. No fallback to the parent conversation's
/// own model or thinking level: a delegate launched on a smaller model is exactly what this
/// tooltip exists to show, and borrowing the parent's answer would hide the one case worth
/// hovering for. Nothing said at all leaves the delegate's name, which the row already carries.
///
/// Public because it is what the panel's row *says*, and a test that the row and [`reading_strip`]
/// agree about a delegate's model has to be able to read one of the two.
pub fn subagent_tip(
    conversation: &Conversation,
    tab: &crate::state::conversation::SubagentTab,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(kind) = &tab.kind {
        parts.push(kind.clone());
    }
    if let Some(model) = &tab.model {
        parts.push(short_model_label(&conversation.harness, model));
    }
    if let Some(thinking) = &tab.thinking {
        parts.push(format!("{thinking} thinking"));
    }
    if parts.is_empty() {
        return tab.name.clone();
    }
    format!("{} \u{2014} {}", tab.name, parts.join(" \u{b7} "))
}

/// One row of that list: a `pill`, because a pill is what this window already draws a small named
/// fact as. Selected takes the accent edge and the full-strength text; the rest stay muted, so the
/// list reads as one selected agent rather than as several equal buttons. The status sits at the
/// far end, so a column of rows reads down either side.
// Eight, each a distinct thing the row draws or answers, and every one of them built inline by
// the one caller — a struct to carry them would be ceremony around a private helper, the same
// reading `kit::menu::menu_panel` makes.
#[allow(clippy::too_many_arguments)]
fn agent_row(
    id: ElementId,
    selector: String,
    name: String,
    status: String,
    tip: Option<String>,
    status_colour: Rgba,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    pill(if selected {
        theme::accent()
    } else {
        theme::border()
    })
    .id(id)
    .debug_selector(move || selector.clone())
    .h(px(22.))
    .w_full()
    .px_2()
    .gap_1p5()
    .cursor_pointer()
    .when(selected, |this| this.bg(theme::surface_raised()))
    .hover(|this| this.bg(theme::hover()))
    .child(
        mono(
            name,
            if selected {
                theme::text()
            } else {
                theme::text_muted()
            },
        )
        .text_size(px(11.))
        .flex_1()
        .min_w(px(0.)),
    )
    .child(mono(status, status_colour).text_size(px(10.5)))
    .on_click(on_click)
    .when_some(tip, |this, tip| {
        this.tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        })
    })
    .into_any_element()
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
