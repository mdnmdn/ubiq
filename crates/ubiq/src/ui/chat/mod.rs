//! The chat panel.
//!
//! It is **editor-like tabs, not a chat of its own**: many instances may be open at once, each
//! attached to a conversation the host owns — or to none — and each free to move to any dockable
//! region. The set of conversations a tab may attach to is the host's, the same set the agents
//! screen draws as columns, so a conversation started here is visible there and the other way
//! round, and closing a tab ends nothing: the conversation, if it had one, keeps running.
//!
//! What a tab draws for its attachment is [`crate::ui::conversation`], the one conversation view
//! every surface shares. This module supplies only the frame: which tab, what it is attached to,
//! and the furniture around it.
//!
//! **The agents screen's columns draw this same panel** ([`ChatHost::Column`]). A column keeps its
//! own chrome — the agent tabs, the `+`, the identity line — and below it is this panel, header,
//! conversation, composer and focus mode included. What a column lacks is the attach chevron: a
//! column's tabs are what say which agent it shows.

pub mod sidebar;

use gpui::{
    App, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::AppState;
use crate::state::ChatHost;
use crate::state::conversation::Conversation;
use crate::theme;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::kit::panel;

pub fn render(
    app: &AppState,
    host: ChatHost,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let attached = attached(app, host, cx);
    let mut root = panel();
    // A column carries its own activity-coloured edge, and stacks the panel under its chrome; a
    // dock tab draws the hairline and fills its leaf.
    root = match host {
        ChatHost::Tab(_) => root.border_l_1().border_color(theme::border()),
        ChatHost::Column(_) => root.flex_1(),
    };
    root.child(sidebar::header(app, host, attached, window, cx))
        // No status strip: the run pill, the context ring and the cost are the shared view's
        // footer, computed from what the harness actually reported. A second strip over it was a
        // fixture, and two answers about one conversation is one answer too many.
        .child(body(app, host, attached, window, cx))
}

/// The element id prefix one panel's conversation view draws under — distinct per host, so two
/// panels never share hover groups or scroll state.
pub(crate) fn view_id(host: ChatHost) -> SharedString {
    match host {
        ChatHost::Tab(id) => SharedString::from(format!("chat-{id}")),
        ChatHost::Column(slot) => SharedString::from(format!("agents-column-{slot}")),
    }
}

/// The conversation one chat panel is attached to, and the composer slot it types into — or
/// `None` if it is attached to nothing. Read once by [`render`] and handed to both the toolbar and
/// the body, so the two never disagree about which conversation (and which glyph, which menu) the
/// panel is showing.
///
/// **The tab is the project on screen's; the conversation need not be** (`T-149`). A card picked
/// on the Teams canvas under [`crate::state::TeamsSpan::Window`] attaches the active project's tab
/// to an agent another held project owns, so the record is read through
/// [`AppState::teams_conversation`], which resolves the agent's own project. Under the project
/// span that is the same answer [`AppState::conversation`] gave. A column is always the project
/// on screen's, and reads its active tab's conversation there.
fn attached<'a>(app: &'a AppState, host: ChatHost, cx: &App) -> Option<(&'a Conversation, usize)> {
    let open = app.open_project(cx)?;
    match host {
        ChatHost::Tab(id) => {
            let tab = open.chats.iter().find(|tab| tab.id == id)?;
            app.teams_conversation(tab.attached?, cx)
                .map(|conv| (conv, tab.slot))
        }
        ChatHost::Column(slot) => {
            let column = open.agents.columns.iter().find(|c| c.slot == slot)?;
            app.conversation(column.active_agent()?, cx)
                .map(|conv| (conv, slot))
        }
    }
}

/// The attached conversation, or a note saying there is nothing to draw.
///
/// The transcript and the composer are the shared view's, not the tab's: a second transcript
/// would be a second answer about the same conversation, and the record is the host's alone.
fn body(
    app: &AppState,
    host: ChatHost,
    attached: Option<(&Conversation, usize)>,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    match (attached, host) {
        // In focus mode the modal draws this conversation over the same composer slot, so the
        // panel holds a placeholder rather than a second live composer.
        (Some(_), _) if app.workbench.chat_focus == Some(host) => div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .bg(theme::app_bg())
            .text_size(theme::font(theme::Family::Chrome, theme::Role::Label))
            .text_color(theme::text_faint())
            .child("Focused")
            .into_any_element(),
        (Some((conversation, slot)), _) => conversation::render(
            app,
            conversation,
            ConversationView {
                id: view_id(host),
                slot,
                footer: true,
                composer: true,
                header: false,
            },
            window,
            cx,
        ),
        // A column whose agent has no conversation is a frame out of date, not a state: the tab
        // is pruned on the next `WorkList`. So it says that, rather than drawing a transcript.
        (None, ChatHost::Column(_)) => crate::ui::empty::empty_page(
            "No conversation",
            "This window is not holding a conversation for this agent any more.",
            IconName::CircleX,
            None,
        )
        .into_any_element(),
        // Nothing attached, and nothing to explain: a tab with no conversation in it is the
        // ordinary state of a fresh tab, not a fault. So the page is the one thing there is to do
        // about it — no title and no note, which said in two lines what the icon says. Not
        // `empty::empty_page`, whose whole shape is a title and a note.
        (None, ChatHost::Tab(id)) => div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .bg(theme::app_bg())
            // Drawn at its own size rather than through `kit::icon_button`: that is the window's
            // chrome control, a fixed small square, and this is a page's whole empty state — the
            // one thing there is to do here, so it is sized as the page's subject.
            .child(
                div()
                    .id("chat-empty-start")
                    .size(px(theme::empty_start_size()))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|this| this.bg(theme::hover()))
                    .child(
                        Icon::new(IconName::Play)
                            .with_size(theme::empty_start_icon())
                            .text_color(theme::text_muted()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_new_agent_in_chat(id, window, cx)
                    }))
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("start a new agent").build(window, cx)
                    }),
            )
            .into_any_element(),
    }
}

/// Focus mode: one chat tab's conversation view near full-window, painted at the window root by
/// `ui::shell`.
///
/// It is the same [`conversation::render`] over the same composer slot as the panel's, so the
/// draft, the attachments and the scroll state are the panel's own — nothing is copied. The panel
/// draws a placeholder while this is up. Escape is the window's (`Layer::ChatFocus`); the outside
/// click and the buttons close it through the same guard.
pub fn focus_modal(
    app: &AppState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> Option<gpui::AnyElement> {
    use gpui::{ElementId, FontWeight, anchored, deferred, point};
    use std::rc::Rc;

    let host = app.workbench.chat_focus?;
    let (conversation, slot) = attached(app, host, cx)?;
    let title = app
        .teams_agent(conversation.id, cx)
        .map(|agent| app.agent_label(agent).title.to_string())
        .unwrap_or_default();

    let viewport = window.viewport_size();
    let entity = cx.entity();
    let dismiss = Rc::new(crate::ui::dismiss(
        &entity,
        crate::state::Layer::ChatFocus,
        |this, _, cx| this.close_chat_focus(cx),
    ));
    let outside = dismiss.clone();

    let body = conversation::render(
        app,
        conversation,
        ConversationView {
            id: SharedString::from(format!("{}-focus", view_id(host))),
            slot,
            footer: true,
            composer: true,
            header: false,
        },
        window,
        cx,
    );

    let panel = div()
        .id("chat-focus-panel")
        .w(viewport.width * theme::CHAT_FOCUS_RATIO)
        .h(viewport.height * theme::CHAT_FOCUS_RATIO)
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        .child(
            div()
                .h(px(38.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::font(theme::Family::Chrome, theme::Role::Micro))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text_faint())
                        .child(SharedString::from(title.to_uppercase())),
                )
                .child(crate::ui::kit::icon_button_tip(
                    ElementId::Name("chat-focus-exit".into()),
                    IconName::Minimize,
                    "Leave focus mode (Esc)",
                    true,
                    move |_, window, cx| dismiss(window, cx),
                )),
        )
        .child(body)
        .on_mouse_down_out(move |_, window, cx| outside(window, cx));

    Some(
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id("chat-focus")
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::scrim())
                    .occlude()
                    .child(panel),
            ),
        )
        // Above the kit's dropdowns, like every modal.
        .priority(2)
        .into_any_element(),
    )
}
