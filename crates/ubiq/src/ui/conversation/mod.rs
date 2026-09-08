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

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt, AnyElement, App, ClickEvent, ClipboardItem, Context, Div, ElementId,
    Focusable, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, point, pulsating_between, px,
};
use gpui_component::input::Textarea;
use gpui_component::text::TextView;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::conversation::{ConfigChoice, ConfigValue, ToolContent, ToolKind, ToolStatus};
use ubiq_proto::work::{Activity, AgentId};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::conversation::{
    Attachment, ConvBlock, Conversation, Pending, QueuedMessage, Run, SubagentTab,
    TranscriptScroll, short_model_label,
};
use crate::state::file_picker::{SizeReading, size_label, size_reading};
use crate::theme;
use crate::ui::kit::menu::{MENU_ANCHOR_UP, MENU_LAYER};
use crate::ui::kit::{
    ContextItem, HARNESS_GLYPH, Picker, PickerStyle, confirm_modal, context_menu, ghost_button,
    icon_button, mono, pill, primary_button, progress_ring, progress_ring_in, removable_tag,
    status_dot,
};
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
    /// Whether this surface draws the lifecycle strip — the three-dots menu — itself. The agents
    /// column keeps it; the chat panel draws the same menu (via [`lifecycle_menu`]) inline in its
    /// own toolbar row instead, so it sets this to `false` rather than showing the strip twice.
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

    // The composer's resize is tracked on the whole view rather than on the grip: dragging the
    // field's top edge upward means the pointer spends the drag over the transcript, so a handler
    // that only listened on the handle would lose the pointer on the first row. A move arriving
    // with no button down is a release that happened somewhere else, and ends the drag rather than
    // leaving one live for the next pointer that crosses the view.
    let mut root = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            if this.composer_drag.is_none() {
                return;
            }
            if event.dragging() {
                this.drag_composer_resize(f32::from(event.position.y), cx);
            } else {
                this.end_composer_resize(cx);
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.end_composer_resize(cx)),
        );
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

    if let Some(oldest) = conversation.oldest_pending() {
        root = root.child(needs_you_strip(
            conversation,
            oldest,
            conversation.pending.len(),
            &view,
            cx,
        ));
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
            .relative()
            .border_t_1()
            .border_color(theme::border());
        // The block's own top border is the edge a reader would reach for to make the field
        // bigger, so that is what the grip sits on — over the border rather than beside it, so
        // nothing is added to the layout and the row below does not move to make room for a
        // handle. Only where there is a field to resize: a footer on its own has no height to
        // give.
        if view.composer && conversation.accepts_input {
            bottom = bottom.child(composer_grip(&view, cx));
        }
        // Topmost in the block, above the footer as well as the composer: it opens upward over
        // the transcript, so nothing under it moves when it does. Only where a subagent exists —
        // a conversation that spawned none looks exactly as it did before, the same discipline
        // every pill in this file follows.
        if !subagents.is_empty() {
            bottom = bottom.child(agent_switcher(conversation, &subagents, &view, cx));
        }
        if view.footer {
            bottom = bottom.child(footer(
                conversation,
                &subagents,
                app.workbench.settings.ui.show_cache_ring,
                &view,
            ));
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
    /// A turn is in flight and blocked on a human. Outranks [`Self::Working`] because it is the
    /// one state that needs the reader to do something, and the one the title's dot exists to
    /// carry across a window they are not looking at.
    Waiting,
    /// A turn is in flight. Carries which kind, so the tooltip reads Thinking, Writing or Tools
    /// rather than flattening every turn to one word. Never `Activity::NeedsYou` — a blocked turn
    /// is [`Self::Waiting`], which is tested before this.
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
            Lifecycle::Waiting => "Needs you".to_string(),
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
/// because a race left `run` behind), a question outranks the turn it is blocking, a turn in flight
/// outranks idle, and only once none of those applies does whether it has ever launched — and, if
/// not, whether it has a transcript — decide the rest.
pub fn lifecycle(conversation: &Conversation) -> Lifecycle {
    if conversation.run == Run::Ended || !conversation.accepts_input {
        return Lifecycle::Ended;
    }
    if !conversation.pending.is_empty() {
        return Lifecycle::Waiting;
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

/// Which of the five lifecycle-menu rows apply, in the order the menu draws them — Stop, Abort,
/// Unload, Resume, Delete. A pure reading of the conversation's own state, pulled out of
/// [`lifecycle_header`] so the enable/disable rule is testable on its own: Stop only while a turn
/// is running, Abort and Unload only while launched, Resume only while not, Delete always (ending
/// applies whatever the state).
///
/// **Stop and Abort are different verbs.** Stop interrupts the *turn* and leaves the harness to
/// take the next one; Abort kills the *process*, which is what is left when a harness has stopped
/// answering and Stop has nothing to interrupt it with. Abort keeps the conversation, so Resume
/// brings it back — it is Delete that is irreversible, and only Delete is confirmed.
/// The rows the lifecycle menu draws, in order. One list, because there were two and they were a
/// row apart from disagreeing: [`lifecycle_menu_enabled`] answers by position, so a label added to
/// one copy and not the other is a menu whose rows do the wrong thing.
const LIFECYCLE_ROWS: [&str; 5] = ["Stop", "Abort", "Unload", "Resume", "Delete"];

pub fn lifecycle_menu_enabled(conversation: &Conversation) -> [bool; 5] {
    [
        conversation.run != Run::Idle,
        conversation.launched,
        conversation.launched,
        !conversation.launched,
        true,
    ]
}

/// The bordered strip the agents column draws above its transcript: [`lifecycle_menu`] inside a
/// header row of its own. The chat panel draws the same menu (see
/// [`crate::ui::chat::sidebar::header`]) inline in its own toolbar instead of this strip, which is
/// why `view.header` gates whether [`render`] calls this at all.
///
/// **The state dot is not here.** It is on the column's title, beside the agent's name, where a
/// reader scanning a row of columns for the one that wants them is already looking — see
/// [`crate::ui::agents::column`]. A dot in this strip as well would be the same fact twice, a
/// line apart.
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
        .child(lifecycle_menu(app, conversation, view, cx))
        .into_any_element()
}

/// The state mark on its own, for a surface that puts it somewhere other than beside the menu.
///
/// The chat tab's head reads left to right — what the conversation *is*, then what it is *on*,
/// then what can be *done to it* — so it wants the mark and [`lifecycle_menu`] at opposite ends
/// of one row rather than as a pair. Same glyph, same tooltip, same colours as the dot the agents
/// column draws on its title: this is the shared fragment, not a second one.
pub fn lifecycle_mark(conversation: &Conversation, view: &ConversationView) -> AnyElement {
    lifecycle_glyph(conversation, view)
}

/// The three-dots menu on its own, the other half of [`lifecycle_mark`]. The whole of the
/// lifecycle strip the agents column draws, and the far end of the chat panel's toolbar row.
pub fn lifecycle_menu(
    app: &AppState,
    conversation: &Conversation,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = conversation.id;
    let entity = cx.entity();

    let enabled = lifecycle_menu_enabled(conversation);
    let labels = LIFECYCLE_ROWS;
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

/// The colour a lifecycle dot draws — four readings, and only four.
///
/// **Yellow needs you, blue is working, green is idle, grey is stopped.** The dot is read at a
/// glance from across a window full of columns, so what it has to answer is "does this one want
/// me", and four colours is as many as that glance can hold. `Working` is one blue rather than
/// `Activity`'s own palette for the same reason: which *kind* of work is a question the reader is
/// already looking at the transcript to answer, and spending the dot on it costs the one reading
/// nothing else carries.
///
/// Every value is a status token the window already assigns this meaning, so a dot never invents
/// a colour.
pub fn lifecycle_colour(state: Lifecycle) -> Rgba {
    match state {
        Lifecycle::Waiting => theme::warning(),
        Lifecycle::Working(_) => theme::info(),
        Lifecycle::Ready | Lifecycle::Idle => theme::success(),
        Lifecycle::Starting | Lifecycle::Unloaded | Lifecycle::Ended => theme::text_faint(),
    }
}

/// A cheap reading of the transcript's tail: how many blocks there are, and how long the last one
/// is.
///
/// A streaming chunk lengthens the last block, so this moves on every token without hashing the
/// whole transcript once a frame — and it does *not* move when nothing was said, which is what
/// lets [`transcript`] follow the tail without dragging a reader who scrolled up back down.
/// Read over the blocks *on screen* rather than over all of them, because that is the tail being
/// followed: while a delegate's transcript is up, the main agent writing below it is not the tail
/// of anything the reader can see, and following it would scroll a transcript nothing was added
/// to.
fn tail_signature(conversation: &Conversation, visible: &[(usize, &ConvBlock)]) -> u64 {
    let tail = match visible.last().map(|(_, block)| block) {
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
    // The run is part of it: the writing indicator appears and disappears without a block being
    // added, and a tail that did not notice would leave it under the fold.
    let run = conversation.run as u64;
    (visible.len() as u64).wrapping_mul(1_000_003) ^ tail as u64 ^ run.wrapping_mul(31)
}

/// The children the transcript is building, each paired with the block it stands for.
///
/// Two jobs, both of which need the child's *position* and so cannot be done by the caller. It
/// records which block each child is, so a request to be taken to a block can be resolved to a
/// child to scroll to; and above [`TranscriptScroll::windows`] blocks it leaves a child that the
/// last frame painted well outside the viewport unbuilt, standing in the exact height it had.
///
/// **The stand-in is measured, never guessed.** A placeholder of the height the child actually
/// had keeps the content above and below it exactly where it was, so nothing about the scroll
/// position changes — which is the whole reason the reader can be spared the markdown, the diffs
/// and the syntax highlighting of a hundred blocks they are not looking at.
struct Built<'a> {
    children: Vec<AnyElement>,
    /// Which block each child stands for, `None` for a child that is not one — the writing mark,
    /// an unattached prompt, the empty note.
    anchors: Vec<Option<usize>>,
    scroll: Option<&'a TranscriptScroll>,
    windowing: bool,
}

impl<'a> Built<'a> {
    fn new(scroll: Option<&'a TranscriptScroll>, blocks: usize) -> Self {
        Self {
            children: Vec::new(),
            anchors: Vec::new(),
            windowing: scroll.is_some_and(|scroll| scroll.windows(blocks)),
            scroll,
        }
    }

    /// The space this child stood in last frame, where it is far enough off screen to be left
    /// unbuilt. `None` means build it.
    fn gap(&self) -> Option<AnyElement> {
        if !self.windowing {
            return None;
        }
        let scroll = self.scroll?;
        let bounds = scroll.child_bounds(self.children.len())?;
        if scroll.near_viewport(bounds) {
            return None;
        }
        Some(div().flex_none().h(bounds.size.height).into_any_element())
    }

    /// Add the child standing for `block`, building it only if it is worth building.
    fn push(&mut self, block: usize, build: impl FnOnce() -> AnyElement) {
        let child = self.gap().unwrap_or_else(build);
        self.children.push(child);
        self.anchors.push(Some(block));
    }

    /// Add a child that is not a block, and is always built.
    fn extra(&mut self, child: AnyElement) {
        self.children.push(child);
        self.anchors.push(None);
    }

    /// Which child to scroll to, to bring a block into view. The block's own child where it has
    /// one; otherwise the last child before it, which is where a folded-away block is drawn.
    fn child_for(&self, block: usize) -> Option<usize> {
        if let Some(exact) = self
            .anchors
            .iter()
            .position(|anchor| *anchor == Some(block))
        {
            return Some(exact);
        }
        self.anchors
            .iter()
            .enumerate()
            .filter(|(_, anchor)| anchor.is_some_and(|anchor| anchor <= block))
            .map(|(ix, _)| ix)
            .next_back()
    }
}

/// How many same-kind tool cards in a row it takes before the run is folded. Below this the fold
/// would replace a card with a row of the same height, which is not a saving.
const GROUP_MIN: usize = 3;

/// One transcript block, drawn as itself — the arm the grouping loop reuses for the cards it does
/// not fold away, and for the ones it unfolds.
#[allow(clippy::too_many_arguments)]
fn one_block(
    conversation: &Conversation,
    id: AgentId,
    ix: usize,
    block: &ConvBlock,
    attached: &HashMap<usize, Vec<&Pending>>,
    view: &ConversationView,
    root: &gpui::Entity<AppState>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    match block {
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
            let delegate = (call.kind == ToolKind::Delegate && conversation.has_subagent(&call.id))
                .then(|| call.id.clone());
            let waiting = attached.get(&ix);
            let card = tool_block(id, ix, call, *open, delegate, waiting.is_some(), view, cx);
            // The prompt belongs to the call, so it is drawn under it rather than somewhere
            // the reader has to go and find. Several are possible on one call.
            match waiting {
                None => card,
                Some(requests) => div()
                    .flex()
                    .flex_none()
                    .flex_col()
                    .child(card)
                    .children(
                        requests
                            .iter()
                            .copied()
                            .map(|request| permission(id, request, Some(call), view, cx))
                            .collect::<Vec<_>>(),
                    )
                    .into_any_element(),
            }
        }
    }
}

/// The row that stands for the folded part of a run: what kind they were, how many, and a chevron
/// that shows them. It borrows the kind's own colour and the tool card's own shape, so a reader
/// sees a stack of the same thing rather than a new kind of object.
///
/// `trailing` is whether a card is still drawn under the row. It decides one word: calls the row
/// stands for are `earlier` ones only while there is a later one to be earlier *than*. Once the
/// run has finished the row is the whole of it, and it says so.
#[allow(clippy::too_many_arguments)]
fn tool_group(
    agent: AgentId,
    key: &str,
    kind: ToolKind,
    hidden: usize,
    trailing: bool,
    open: bool,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = tool_colour(kind);
    let key = key.to_string();
    let word = if hidden == 1 { "call" } else { "calls" };
    let count = if trailing {
        format!("{hidden} earlier {word}")
    } else {
        format!("{hidden} {word}")
    };
    // Which run the row stands for and what it says about it, in one name — the fold's whole
    // decision is how much of the run the row swallowed, so a selector naming only the run could
    // not tell a finished fold from one still holding its last card out.
    let selector = format!("tool-group-{key}-{}", count.replace(' ', "-"));
    div()
        .id(view.eid(&format!("group-{key}")))
        .debug_selector(move || selector)
        .min_h(px(24.))
        .px_2()
        .py_1()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_l_2()
        .border_color(theme::fade(colour, 0.5))
        .bg(theme::surface())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::XSmall)
            .text_color(theme::text_faint()),
        )
        .child(mono(kind.label(), colour).text_size(px(11.5)))
        .child(mono(count, theme::text_faint()).text_size(px(11.5)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_conversation_tool_group(agent, key.clone(), cx)
        }))
        .into_any_element()
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

    // Every prompt still up, joined onto the call it authorises by that call's id — which is the
    // only field of a request's patch upstream guarantees. What matches nothing in the transcript
    // is drawn on its own at the end rather than dropped: a request nobody can answer deadlocks
    // the turn, so losing one is worse than drawing it out of place.
    let mut attached: HashMap<usize, Vec<&Pending>> = HashMap::new();
    let mut adrift: Vec<&Pending> = Vec::new();
    for request in &conversation.pending {
        match conversation.tool_block_index(&request.tool_call.id) {
            Some(block) => attached.entry(block).or_default().push(request),
            None => adrift.push(request),
        }
    }

    // One agent's turns, never two interleaved — and the indices are the real ones, because the
    // element ids and the tool-toggle listener both key off a block's position in `blocks`.
    //
    // A run of same-kind tool calls is drawn as one row standing for the whole of it: twelve
    // `READ`s in a row are twelve rows of furniture between two sentences. While the run's last
    // call is still going that one card stays out of the fold, because a call in flight is the
    // part a reader is following; once it finishes the row is the entire run. Either way the row
    // says how many it stands for and opens to show them.
    let visible = conversation.visible_blocks();
    let scroll = app.transcript_scrolls.get(view.slot);
    // Where the reader is put before anything is drawn: a transcript switched to is restored to
    // where it was left, and one whose tail moved is followed only for a reader already on it.
    if let Some(scroll) = scroll {
        scroll.sync(
            (id, conversation.viewing_subagent().map(str::to_string)),
            tail_signature(conversation, &visible),
        );
    }
    let mut blocks = Built::new(scroll, visible.len());
    let mut at = 0usize;
    while at < visible.len() {
        let (ix, block) = visible[at];
        // A delegation is never folded away: a spawned agent is a second transcript, not a step.
        // Nor is a call somebody is being asked to authorise — a prompt hidden behind a counter is
        // a turn that deadlocks.
        if let ConvBlock::Tool { call, .. } = block
            && call.kind != ToolKind::Delegate
            && !attached.contains_key(&ix)
        {
            let kind = call.kind;
            let mut end = at + 1;
            while end < visible.len() {
                match visible[end].1 {
                    ConvBlock::Tool { call: next, .. }
                        if next.kind == kind && !attached.contains_key(&visible[end].0) =>
                    {
                        end += 1;
                    }
                    _ => break,
                }
            }
            // Two cards become a row plus a card, which is no shorter and one more thing to learn.
            // Three is where folding starts paying.
            if end - at >= GROUP_MIN {
                let key = call.id.clone();
                let open = conversation.open_groups.contains(&key);
                // The last card is held out of the fold only while it is still going: a call in
                // flight is the one thing in the run a reader is actually following, and folding
                // it away would hide the only part still changing. The moment it finishes there
                // is nothing left to follow, so it joins the ones before it and the whole run
                // becomes the one row — which is what a finished run of twelve READs should cost.
                let (last_ix, last) = visible[end - 1];
                let running = matches!(
                    last,
                    ConvBlock::Tool { call, .. }
                        if matches!(call.status, ToolStatus::Pending | ToolStatus::InProgress)
                );
                let hidden = if running { end - at - 1 } else { end - at };
                blocks.push(ix, || {
                    tool_group(id, &key, kind, hidden, running, open, view, cx)
                });
                if open {
                    for &(hidden_ix, block) in &visible[at..at + hidden] {
                        blocks.push(hidden_ix, || {
                            one_block(
                                conversation,
                                id,
                                hidden_ix,
                                block,
                                &attached,
                                view,
                                &root,
                                cx,
                            )
                        });
                    }
                }
                if running {
                    blocks.push(last_ix, || {
                        one_block(conversation, id, last_ix, last, &attached, view, &root, cx)
                    });
                }
                at = end;
                continue;
            }
        }
        blocks.push(ix, || {
            one_block(conversation, id, ix, block, &attached, view, &root, cx)
        });
        at += 1;
    }

    // A request whose call the transcript does not hold — the patch carried nothing but an id, or
    // the request outran the call announcing it. Self-contained, and still answerable.
    for request in adrift {
        blocks.extra(permission(id, request, None, view, cx));
    }

    // Last, so a transcript holding only an unattached prompt reads as the question it is.
    if blocks.children.is_empty() {
        blocks.extra(
            mono("nothing said yet", theme::text_faint())
                .text_size(px(11.5))
                .into_any_element(),
        );
    }

    // And after everything, while the turn is still running: the tail of a transcript is where a
    // reader waits, so that is where the waiting is drawn. Not while a prompt is up — the question
    // on screen is what is happening, and two marks would compete to say so.
    if conversation.run == Run::Working && conversation.pending.is_empty() {
        blocks.extra(writing_mark(
            conversation.activity(),
            mark_variant(conversation),
            view,
        ));
    }

    let mut body = div()
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

    if let Some(scroll) = scroll {
        // Somewhere to be taken to, asked for by the strip that named the prompt. Resolved here
        // rather than where it was asked for, because only the frame that built the children
        // knows which child a block ended up as.
        match scroll
            .take_request()
            .and_then(|block| blocks.child_for(block))
        {
            Some(child) => scroll.handle.scroll_to_top_of_item(child),
            // The frame that switched transcripts is measuring the one it left, so it cannot
            // answer a jump — it asks for the frame that can. Self-terminating: the next frame
            // is settled, answers, and clears the request.
            None if scroll.request_held() => cx.notify(),
            None => {}
        }
        body = body.track_scroll(&scroll.handle);
    }
    body = body.children(blocks.children);

    // The jump sits over the transcript rather than in the column, so nothing moves when it
    // appears and the last line stays readable under it.
    let mut framed = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .relative()
        .child(body);
    if scroll.is_some_and(TranscriptScroll::away) {
        framed = framed.child(to_tail_button(view, cx));
    }
    framed.into_any_element()
}

/// The overlay that puts a reader who has scrolled up back on the tail.
///
/// Drawn only while there is something below the viewport, because a button that is always there
/// is a button that says nothing. It scrolls; it does not mark anything read and does not resume
/// following on its own — the next thing said does that, which is what the reader asked for by
/// coming back down.
fn to_tail_button(view: &ConversationView, cx: &mut Context<AppState>) -> AnyElement {
    let slot = view.slot;
    div()
        .id(view.eid("to-tail"))
        .absolute()
        .bottom_2()
        .right_3()
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .rounded_md()
        .bg(theme::surface())
        .border_1()
        .border_color(theme::border())
        .shadow_sm()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(IconName::ArrowDown)
                .with_size(Size::XSmall)
                .text_color(theme::text_muted()),
        )
        .child(mono("Go to last message", theme::text_muted()).text_size(px(10.5)))
        .on_click(cx.listener(move |this, _, _, cx| this.scroll_transcript_to_tail(slot, cx)))
        .into_any_element()
}

/// The hit area of the strip that resizes the composer. Wide enough to grab, and drawn as nothing
/// at all: the block's own top border is already the line, so a bar of its own would be a second
/// edge one pixel from the first.
const COMPOSER_GRIP: f32 = 5.0;

/// The strip on the composer block's top edge that makes the field taller.
///
/// Dragging up is more writing space, which is the whole of it. Double-clicking hands the field
/// back to growing with what is typed, because a size dragged by hand needs a way back that is not
/// dragging it to exactly where it was.
fn composer_grip(view: &ConversationView, cx: &mut Context<AppState>) -> AnyElement {
    let slot = view.slot;
    div()
        .id(view.eid("composer-grip"))
        .absolute()
        .top(px(-COMPOSER_GRIP / 2.0))
        .left_0()
        .w_full()
        .h(px(COMPOSER_GRIP))
        .cursor_row_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.start_composer_resize(slot, f32::from(event.position.y), cx);
            }),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if event.click_count() >= 2 {
                this.end_composer_resize(cx);
                this.set_composer_rows(slot, None, cx);
            }
        }))
        .into_any_element()
}

/// How many squares the scanner sweeps across. Five is enough for the sweep to read as travel
/// rather than as a blink, and short enough to sit on one line beside a word.
const SCAN_CELLS: u32 = 5;

/// One square of the waiting mark. Square rather than round, and the accent rather than a colour
/// of its own: this is the same blue and the same corner every chip, ring and pill in the window
/// is drawn with, so the mark reads as part of the application rather than as a widget visiting
/// from somewhere else.
fn mark_cell(colour: Rgba) -> Div {
    div().size(px(5.)).rounded(px(1.)).flex_none().bg(colour)
}

/// Which of the two waiting marks a turn draws.
///
/// Not a random number: the answer has to be the same on every frame of one turn, because a mark
/// that re-rolled per frame would strobe between two animations instead of playing either. So it
/// is a hash of the conversation and of which turn this is — fixed for exactly as long as the turn
/// lasts, and different from one turn to the next, which is all "pick one of two" needs.
fn mark_variant(conversation: &Conversation) -> u64 {
    // Which turn this is, counted from the end: the last thing the user said cannot move while
    // that turn is still running, so this is stable for precisely the mark's lifetime.
    let turn = conversation
        .blocks
        .iter()
        .rposition(|block| matches!(block, ConvBlock::User(_)))
        .map_or(0, |at| at + 1);
    let mut hasher = DefaultHasher::new();
    conversation.id.hash(&mut hasher);
    turn.hash(&mut hasher);
    hasher.finish()
}

/// The mark that says the agent is mid-turn: small accent squares in motion, and the word for what
/// it is doing right now.
///
/// A turn can be a minute of silence between two sentences, and silence in a chat reads as
/// nothing happening. Movement is the only honest thing to draw there — a spinner would claim
/// progress it cannot measure, and a percentage would be a lie with a decimal point.
///
/// **Two animations, one per turn, chosen by [`mark_variant`].** A wave of squares breathing out
/// of phase, or a single light sweeping the row and coming back. Both say the same thing and
/// neither says more than the other; a mark a reader waits minutes in front of is worth not being
/// identical every time. Which one is drawn is never a status — the word beside them is what
/// carries the activity.
fn writing_mark(activity: Activity, variant: u64, view: &ConversationView) -> AnyElement {
    let colour = theme::accent();
    let scanning = variant % 2 == 1;
    let cells: Vec<AnyElement> = if scanning {
        (0..SCAN_CELLS)
            .map(|n| {
                let at = n as f32;
                mark_cell(colour)
                    .with_animation(
                        view.eid(&format!("writing-scan-{n}")),
                        Animation::new(Duration::from_millis(1_400))
                            .repeat()
                            .with_easing(move |delta| {
                                // A triangle wave, so the light runs to the end of the row and
                                // comes back rather than jumping home: one pass out and one pass
                                // back per cycle.
                                let head = if delta < 0.5 {
                                    delta * 2.0
                                } else {
                                    (1.0 - delta) * 2.0
                                } * (SCAN_CELLS - 1) as f32;
                                // How near the head is to this square, softened so the squares
                                // beside it glow too and the row reads as one moving light rather
                                // than five taking turns to blink.
                                let near = 1.0 - ((head - at).abs() / 1.8).min(1.0);
                                0.15 + near * 0.85
                            }),
                        |this, delta| this.opacity(delta),
                    )
                    .into_any_element()
            })
            .collect()
    } else {
        (0..3u32)
            .map(|n| {
                // Each square a third of a cycle behind the one before it, which is what makes
                // the row read as a wave rather than as a blink.
                let phase = n as f32 / 3.0;
                mark_cell(colour)
                    .with_animation(
                        view.eid(&format!("writing-pulse-{n}")),
                        Animation::new(Duration::from_millis(1_100))
                            .repeat()
                            .with_easing(move |delta| {
                                pulsating_between(0.2, 1.0)((delta + phase) % 1.0)
                            }),
                        |this, delta| this.opacity(delta),
                    )
                    .into_any_element()
            })
            .collect()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        // The scanner's squares sit closer together: a light travelling a row has to look like
        // one row, and a wave of three has to look like three things.
        .gap(if scanning { px(3.) } else { px(6.) })
        .px_1()
        .py_0p5()
        .children(cells)
        .child(mono(activity.label(), theme::text_faint()).text_size(px(11.)))
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
///
/// `awaiting` is set where a permission prompt is drawn under this block. The card then reads as
/// blocked on the reader rather than as whatever status the harness last stamped on it — which is
/// `pending`, and `pending` alone cannot tell "input still streaming" from "waiting for you".
#[allow(clippy::too_many_arguments)]
fn tool_block(
    agent: AgentId,
    index: usize,
    call: &ubiq_proto::conversation::ToolCallRecord,
    open: bool,
    delegate: Option<String>,
    awaiting: bool,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = if awaiting {
        theme::warning()
    } else {
        tool_colour(call.kind)
    };
    let expandable = !call.content.is_empty();
    let call_id = call.id.clone();

    // A title that wraps — a long command, most often — grows the row rather than being
    // clipped to one line's height: `min_h` is the floor, not the ceiling, and `items_start`
    // keeps the icon and the status pinned to the first line instead of drifting to the
    // paragraph's centre.
    let mut header = div()
        .id(view.eid(&format!("tool-{index}")))
        // Named by the block it draws, so a fold is legible from outside: which cards a run left
        // on screen is the question, and a card that answers only "some card" cannot settle it.
        .debug_selector(move || format!("tool-card-{index}"))
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
        if awaiting {
            mono("needs you", theme::warning())
        } else {
            mono(status_label(call.status), status_colour(call.status))
        }
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
/// Drawn in the transcript, under the tool call it authorises — `call` is that call, where the
/// transcript holds it. The request's own patch guarantees only a call id, so the title and the
/// detail are read off the call first and the patch second; with neither, the prompt still says
/// what it can and still offers every option, because a request left unanswered blocks the turn.
///
/// The element ids carry the `request_id`, not a position: two prompts on screen at once are two
/// different questions, and an id that moved when the first was answered would hand the second
/// one's clicks to whatever took its place.
fn permission(
    agent: AgentId,
    pending: &Pending,
    call: Option<&ubiq_proto::conversation::ToolCallRecord>,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let request_id = pending.request_id.clone();
    let what = pending
        .tool_call
        .title
        .clone()
        .or_else(|| call.map(|call| call.title.clone()))
        .unwrap_or_else(|| "The agent is asking to go ahead".to_string());

    // The request's own detail, which is what a `switch_mode` prompt's plan and a pre-approval
    // diff arrive as. Only the patch's: the call's own content is the block above this one, and
    // drawing it twice would read as two operations.
    let detail: Vec<AnyElement> = pending
        .tool_call
        .content
        .iter()
        .flatten()
        .map(content)
        .collect();

    let buttons: Vec<AnyElement> = pending
        .options
        .iter()
        .map(|option| {
            let id = view.eid(&format!("permission-{request_id}-{}", option.option_id));
            let request_id = request_id.clone();
            // The option id is echoed back exactly as it arrived: it is the harness's token, and
            // nothing here reads it. `kind` is the only thing this looks at, and only to draw.
            let option_id = option.option_id.clone();
            let answer = cx.listener(move |this, _, _, cx| {
                this.answer_permission(agent, request_id.clone(), option_id.clone(), cx);
            });
            let icon = Some(match (option.kind.allows(), option.kind.remembers()) {
                (true, false) => IconName::Check,
                (true, true) => IconName::CircleCheck,
                (false, false) => IconName::Close,
                (false, true) => IconName::Delete,
            });
            // Filled for going ahead, ghost for refusing; the icon is what separates "this time"
            // from "and remember". Two axes, because `kind` has two.
            if option.kind.allows() {
                primary_button(id, icon, option.name.clone(), answer).into_any_element()
            } else {
                ghost_button(id, icon, option.name.clone(), answer).into_any_element()
            }
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
        .when(!detail.is_empty(), |this| {
            this.child(div().flex().flex_col().gap_1().children(detail))
        })
        // A request that offered no option at all is still worth drawing: it says the turn is
        // blocked, which is the thing the reader has to know. Answering it needs the harness to
        // offer something.
        .when(buttons.is_empty(), |this| {
            this.child(
                mono("the harness offered no answer to this", theme::text_faint())
                    .text_size(px(11.)),
            )
        })
        .child(div().flex().items_center().gap_2().children(buttons))
        .into_any_element()
}

/// That something is waiting, kept in view above the footer while the prompt itself is anywhere in
/// the transcript.
///
/// Answering is blocking, so a prompt scrolled out of sight would read as an agent that had
/// stopped for no reason.
///
/// **The strip is answerable, and it is the way to the prompt.** It carries yes / no / all for the
/// oldest request, because the one control on screen when a turn is blocked should be the one that
/// unblocks it — a strip that only reported the question made a reader hunt for the prompt to
/// answer a question they had already read. Clicking anywhere else on it does that hunt for them:
/// it switches to whoever raised the request and scrolls to the call it authorises, so the two
/// ways of answering lead to the same place rather than competing.
///
/// **It counts, it does not list.** With several up it names the oldest and says how many are
/// behind it; the rest are answered by working through them, one strip at a time, because each
/// one's options are its own.
fn needs_you_strip(
    conversation: &Conversation,
    oldest: &Pending,
    waiting: usize,
    view: &ConversationView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    // The same join the prompt itself does: the patch names the operation only sometimes, and the
    // call in the transcript names it always.
    let what = oldest
        .tool_call
        .title
        .clone()
        .or_else(|| {
            match conversation
                .tool_block_index(&oldest.tool_call.id)
                .and_then(|ix| conversation.blocks.get(ix))
            {
                Some(ConvBlock::Tool { call, .. }) => Some(call.title.clone()),
                _ => None,
            }
        })
        .unwrap_or_else(|| "an operation".to_string());
    // Whose question it is, named where it is not this transcript's own: with a delegate blocked
    // and the main agent on screen, "an operation" is not enough to go on.
    let whose = conversation
        .pending_subagent(oldest)
        .map(|id| conversation.subagent_name(id));

    let agent = conversation.id;
    let slot = view.slot;
    let request = oldest.request_id.clone();

    // One button per reading the request actually offered, and none for a reading it did not.
    let answers = [
        (
            "yes",
            "Yes",
            true,
            oldest
                .option_for(true)
                .map(|option| option.option_id.clone()),
        ),
        (
            "all",
            "All",
            true,
            oldest
                .always_option()
                .map(|option| option.option_id.clone()),
        ),
        (
            "no",
            "No",
            false,
            oldest
                .option_for(false)
                .map(|option| option.option_id.clone()),
        ),
    ];
    let buttons: Vec<AnyElement> = answers
        .into_iter()
        .filter_map(|(part, label, allows, option)| {
            let option = option?;
            let answer_for = request.clone();
            let click = cx.listener(move |this, _, _, cx| {
                this.answer_permission(agent, answer_for.clone(), option.clone(), cx)
            });
            Some(if allows {
                primary_button(view.eid(part), None, label, click).into_any_element()
            } else {
                ghost_button(view.eid(part), None, label, click).into_any_element()
            })
        })
        .collect();

    let mut strip = div()
        .id(view.eid("needs-you"))
        .px_3()
        .py_1p5()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::warning_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::warning())
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(Size::XSmall)
                .text_color(theme::warning()),
        )
        .child(mono("NEEDS YOU", theme::warning()).text_size(px(10.5)));

    // The count is a mark rather than a clause: "and 3 more waiting" read as part of what the
    // operation was, and the number is the part a reader is counting down.
    if waiting > 1 {
        strip = strip.child(waiting_count(waiting, view));
    }

    strip
        .child(
            div()
                .id(view.eid("needs-you-go"))
                .flex()
                .flex_1()
                .min_w(px(0.))
                .items_center()
                .gap_1p5()
                .cursor_pointer()
                .when_some(whose, |this, whose| {
                    this.child(mono(whose, theme::warning()).text_size(px(11.)))
                })
                .child(
                    mono(what, theme::text_muted())
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(11.5)),
                )
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new("Go to what is waiting").build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.reveal_permission(agent, slot, request.clone(), cx)
                })),
        )
        .children(buttons)
        .child(mono("⌘⌥Y / ⌘⌥N", theme::text_faint()).text_size(px(10.5)))
        .into_any_element()
}

/// How many requests are outstanding, drawn as a count and only above one.
fn waiting_count(waiting: usize, view: &ConversationView) -> AnyElement {
    div()
        .id(view.eid("needs-you-count"))
        .h(px(16.))
        .min_w(px(16.))
        .px_1()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::warning())
        .child(mono(waiting.to_string(), theme::pane_bg()).text_size(px(10.)))
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(format!(
                "{waiting} requests waiting — the oldest is named here"
            ))
            .build(window, cx)
        })
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

/// What the `tot` readout stands for while a delegate's transcript is up.
///
/// Says the two things a reader has to know to trust the number: it is this delegate's spend
/// rather than the conversation's, and it is banked per delegate *type*, so where several of a
/// type ran it is their sum. The wire has no finer grain — `UsageRecord::subagent` is a type on
/// purpose — and a footer that quietly presented a shared bucket as one instance's would be
/// drawing a guess as a count.
fn delegate_spend_tip(
    conversation: &Conversation,
    subagents: &[SubagentTab],
    tab: &SubagentTab,
) -> String {
    let Some(kind) = tab.kind.as_deref() else {
        return format!("{} \u{2014} nothing counted for it yet", tab.name);
    };
    let Some((total, cached)) = conversation.subagent_tokens(kind) else {
        return format!("{} \u{2014} nothing counted for it yet", tab.name);
    };
    let mut tip = format!(
        "{total} tokens billed by {kind} delegates \u{b7} {cached} read back from cache \
         \u{b7} a flow, only ever growing"
    );
    // Counted per type, so a reader comparing two rows of the same type is looking at one number
    // twice. Said only where that is actually the case.
    let same = subagents
        .iter()
        .filter(|other| other.kind.as_deref() == Some(kind))
        .count();
    if same > 1 {
        tip.push_str(&format!(
            " \u{b7} shared by all {same} {kind} delegates: the harness banks spend by type, \
             not by instance"
        ));
    }
    tip.push_str(
        " \u{b7} no context level is reported for a delegate, so no ring is drawn beside it",
    );
    tip
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
        "Total \u{2014} \
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
///
/// **The row reports whoever is being read.** With a delegate's transcript up it says what that
/// delegate spent, not what the conversation spent — a reader looking at one agent's turns wants
/// that agent's numbers, and the conversation's total is the one thing that is on screen either
/// way from the switcher's count. Two limits of the wire show through here, and neither is papered
/// over:
///
/// - **A delegate's spend is per *type*, not per instance.** `UsageRecord::subagent` is
///   deliberately a type, so two `general-purpose` delegates share one bucket. The tooltip says
///   so; nothing here divides a shared total between instances to make it look exact.
/// - **A delegate has no context level at all.** A subagent's usage report repeats the *parent's*
///   occupancy, so there is no per-delegate window to draw — and the parent's ring beside a
///   delegate's transcript would be a number about somebody else. So the ring is dropped rather
///   than borrowed, on the same rule as the paragraph above it.
fn footer(
    conversation: &Conversation,
    subagents: &[SubagentTab],
    cache_ring: bool,
    view: &ConversationView,
) -> AnyElement {
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

    // Whose numbers the rest of the row is about. A delegate's transcript reports the delegate;
    // the conversation's own reports the conversation, subagents folded in.
    let delegate = conversation
        .viewing_subagent()
        .and_then(|id| subagents.iter().find(|tab| tab.id == id));
    let (spend, spend_tip) = match delegate {
        Some(tab) => (
            tab.kind
                .as_deref()
                .and_then(|kind| conversation.subagent_tokens(kind)),
            delegate_spend_tip(conversation, subagents, tab),
        ),
        None => (
            conversation
                .total_tokens()
                .map(|total| (total, conversation.cached_tokens().unwrap_or(0))),
            spend_tip(conversation),
        ),
    };

    // Everything that has been spent, which is not what is in the window: a compacted
    // conversation has spent millions and holds thousands. Drawn only where the harness counts it
    // — a pill with nothing behind it is not drawn.
    if let Some((total, _)) = spend {
        row = row.child(tipped(
            view.eid("total-tokens"),
            format!("{:.1}K tot", total as f32 / 1000.0),
            spend_tip,
            theme::text_muted(),
        ));
    }

    // How much of that total was context read back out of the cache rather than paid for again.
    // Off by default and asked for in settings: it is a cost-of-running reading, not a
    // how-is-this-turn-going one, and the row is glanced at. Its own colour, because a second
    // accent ring beside the context one would read as the same fact twice.
    if cache_ring
        && let Some((total, cached)) = spend
        && total > 0
    {
        let pct = ((cached as f64 / total as f64) * 100.0).round() as u8;
        let tip = match delegate {
            Some(tab) => format!("cached {cached} / {total} {pct}% \u{2014} {}", tab.name),
            None => format!("cached {cached} / {total} {pct}%"),
        };
        row = row.child(
            div()
                .id(view.eid("cache-ring"))
                .flex()
                .flex_none()
                .items_center()
                .child(progress_ring_in(pct.min(100), 12., theme::info()))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                }),
        );
    }

    // Only for the conversation's own transcript: no harness reports a delegate's own occupancy,
    // and the parent's would be a reading about somebody else.
    if let Some(pct) = conversation.context_pct().filter(|_| delegate.is_none()) {
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

/// Stop, which is a square and not a cross.
///
/// The distinction is the whole of what the control means: a cross reads as *close this*, and this
/// closes nothing. It ends the turn in flight — the transcript stays, the harness stays, and the
/// next message goes to the same agent — which is what a square over a stream has meant since
/// tape decks. Drawn rather than iconised because the icon set ships no square, and a square is
/// four sides.
fn stop_button(
    id: ElementId,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(26.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(theme::danger())
        .cursor_pointer()
        .hover(|this| this.bg(theme::fade(theme::danger(), 0.8)))
        .child(
            div()
                .size(px(9.))
                .rounded(px(1.5))
                .flex_none()
                .bg(theme::on_accent()),
        )
        .on_click(on_click)
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(
                "Stop this turn \u{2014} the agent stays, and takes the next message",
            )
            .build(window, cx)
        })
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
    // Attachments alone are something to send: the prompt that goes out is what was typed *plus*
    // every attached path as a mention, so a turn that is only files is not an empty one.
    let can_send = !input.read(cx).value().trim().is_empty() || !conversation.attached.is_empty();
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
        // Files for this turn. The picker is the window's own, raised over the explorer's tree and
        // taking as many files as are wanted; what comes back is a tag apiece above the field, and
        // becomes `@path` in the prompt only when it is sent. With no project open there is no
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

    // Stop is there for the whole of a running turn, not only while the field is empty: the moment
    // a message is sent is the moment the reader most wants it back, and a control that appears
    // only when nothing is typed is a control that vanishes as soon as they start writing the
    // follow-up. Beside it, when there is something to send, Enqueue — the turn in flight is not
    // interrupted by typing at it, and what is typed goes out when it ends. Idle keeps one button.
    // All three are what the Enter key answers through `AppState::send_or_enqueue`, so the buttons
    // and the key never disagree.
    let mut actions: Vec<AnyElement> = Vec::new();
    if working {
        actions.push(stop_button(
            view.eid("stop"),
            cx.listener(move |this, _, _, cx| this.cancel_turn(id, cx)),
        ));
        if can_send {
            actions.push(action_button(
                view.eid("send"),
                IconName::Inbox,
                "Enqueue",
                theme::accent(),
                true,
                cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
            ));
        }
    } else {
        actions.push(action_button(
            view.eid("send"),
            IconName::ArrowUp,
            "Send",
            theme::accent(),
            can_send,
            cx.listener(move |this, _, window, cx| this.send_or_enqueue(id, slot, window, cx)),
        ));
    }

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
        .child(controls.children(actions));

    let mut extras: Vec<AnyElement> = Vec::new();
    // Attachments first, so they sit directly under the token and context row and above anything
    // waiting to be sent: they belong to the turn being written, which is the field below them.
    if !conversation.attached.is_empty() {
        extras.push(attachment_tags(id, view, &conversation.attached, cx));
    }
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
    let main_waiting = conversation.pending_count(None);
    let (main_status, main_colour) = if main_waiting > 0 {
        (needs_you_label(main_waiting), theme::warning())
    } else {
        (state.label(), lifecycle_colour(state))
    };
    let mut rows: Vec<AnyElement> = vec![agent_row(
        view.eid("agent-tag-main"),
        "agent-row-main".to_string(),
        "Main agent".to_string(),
        // What the main agent runs as is the footer's chip, right below: saying it twice would be
        // the same fact drawn twice. A delegate has no chip of its own, which is why its row
        // carries one.
        None,
        main_status,
        None,
        main_colour,
        viewing.is_none(),
        cx.listener(move |this, _, _, cx| {
            this.view_conversation_agent(id, None, cx);
            this.toggle_conversation_subagents(id, cx);
        }),
    )];

    rows.extend(subagents.iter().map(|tab| {
        let target = tab.id.clone();
        // A delegate waiting on a human says so in place of what it was doing.
        let (status, colour) = if tab.waiting > 0 {
            (needs_you_label(tab.waiting), theme::warning())
        } else {
            match tab.status {
                Some(status) => (status_label(status).to_string(), status_colour(status)),
                None => ("unknown".to_string(), theme::text_faint()),
            }
        };
        agent_row(
            view.eid(&format!("agent-tag-{}", tab.id)),
            format!("agent-row-{}", tab.id),
            tab.name.clone(),
            tab.model
                .as_deref()
                .map(|model| short_model_label(&conversation.harness, model)),
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
    let active = subagents
        .iter()
        .filter(|tab| {
            matches!(
                tab.status,
                Some(ToolStatus::Pending | ToolStatus::InProgress)
            )
        })
        .count();
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
        .child(mono(subagent_count_label(active, count), theme::text_muted()).text_size(px(11.)))
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

/// What the collapsed header says: how many delegates are still working, and — where some have
/// already finished — out of how many. `3 active subagents of 10` while seven are done; plain
/// `3 active subagents` while all three are still running, because `of 3` is the same number
/// twice. With none running the count alone is the fact worth a line: `10 subagents`.
fn subagent_count_label(active: usize, total: usize) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    if active == 0 {
        return format!("{total} subagent{}", plural(total));
    }
    if active == total {
        return format!("{active} active subagent{}", plural(active));
    }
    format!("{active} active subagent{} of {total}", plural(active))
}

/// What a row says instead of its status while it is blocked on a human. The count only where
/// there is more than one to answer, because `need you 1` is a number nobody needed.
fn needs_you_label(waiting: usize) -> String {
    if waiting > 1 {
        format!("need you \u{d7}{waiting}")
    } else {
        "need you".to_string()
    }
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
///
/// **The model sits beside the name, faint.** A delegate is chiefly identified by what it is
/// answering with, and a row that only had it on hover made the reader hover every row to compare
/// three. Faint rather than muted because it is a qualifier of the name, not a second fact.
///
/// **`need you` takes the status's place rather than sitting beside it.** A delegate blocked on a
/// human is not doing anything, so `running` and `need you` together would be one of them wrong —
/// and the question is the useful reading of the two.
// Nine, each a distinct thing the row draws or answers, and every one of them built inline by
// the one caller — a struct to carry them would be ceremony around a private helper, the same
// reading `kit::menu::menu_panel` makes.
#[allow(clippy::too_many_arguments)]
fn agent_row(
    id: ElementId,
    selector: String,
    name: String,
    model: Option<String>,
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
    // Name and model are one reading of who this is, so they share one flexible box: the model
    // gives way before the name does when the row is narrow.
    .child(
        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .items_center()
            .gap_1p5()
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
                .flex_none(),
            )
            .when_some(model, |this, model| {
                this.child(
                    mono(model, theme::text_faint())
                        .text_size(px(10.))
                        .min_w(px(0.)),
                )
            }),
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

/// The files this turn will carry, one tag each, wrapping onto as many lines as they need.
///
/// **A tag, not a path in the field.** A path spelled into the prompt cannot be clicked to see
/// what it is, and cannot be taken back out without editing text the user did not type; a tag
/// opens the file in the editor and drops itself. The mentions the harness reads are composed
/// from these when the prompt is sent, so nothing new crosses the bus for an attachment.
///
/// **The colour is the size.** A file large enough to cost a noticeable part of the context window
/// says so before the turn is spent on it, and a file large enough to fill it says so louder —
/// [`size_reading`]'s three readings, drawn in the warning and danger tokens. A size no host
/// reported is drawn plainly rather than guessed at. The whole path and the size in figures are
/// the tooltip, because the tag itself has room for a file name and nothing else.
fn attachment_tags(
    agent_id: AgentId,
    view: &ConversationView,
    attached: &[Attachment],
    cx: &mut Context<AppState>,
) -> AnyElement {
    let tags: Vec<AnyElement> = attached
        .iter()
        .map(|file| {
            let attachment = file.id;
            let reading = size_reading(file.size);
            let (fill, edge, colour) = match reading {
                SizeReading::Huge => (theme::danger_soft(), theme::danger(), theme::danger()),
                SizeReading::Large => (theme::warning_soft(), theme::warning(), theme::warning()),
                SizeReading::Plain => (theme::surface(), theme::border(), theme::text_muted()),
            };

            // The row already says the name; what answers "which one is this" is the path from
            // the project root, which is the shape every path in this interface is held in.
            let name = match file.path.rsplit_once('/') {
                Some((_, name)) => name.to_string(),
                None => file.path.clone(),
            };
            let mut tip = file.path.clone();
            let size = size_label(file.size);
            if !size.is_empty() {
                tip.push_str(" \u{00b7} ");
                tip.push_str(&size);
            }
            if let Some(warning) = reading.warning() {
                tip.push_str(" \u{2014} ");
                tip.push_str(warning);
            }

            let open = file.path.clone();
            removable_tag(
                view.eid(&format!("attached-{attachment}")),
                view.eid(&format!("attached-remove-{attachment}")),
                name,
                tip,
                fill,
                edge,
                colour,
                cx.listener(move |this, _, _, cx| this.select_file(open.clone(), cx)),
                cx.listener(move |this, _, _, cx| this.detach_file(agent_id, attachment, cx)),
            )
            .into_any_element()
        })
        .collect();

    div()
        .px_2()
        .pt_1()
        .flex()
        .flex_wrap()
        .flex_none()
        .items_center()
        .gap_1()
        .children(tags)
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
