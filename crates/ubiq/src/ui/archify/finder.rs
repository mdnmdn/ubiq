//! The node finder (`V/node-finder.js`, `00` §7: essential, "Ubiq picker"): `/` or `f` (or `⋯` >
//! Find node) opens the kit picker over the picture, the base's shared filter field takes the
//! keys, and a row (or Enter, for the first) focuses that node and glides the camera to it and its
//! neighbours. The list is the nodes of the scene matching every word typed, by label, id, kind,
//! sublabel, context and tag.
//!
//! The list is drawn by the viewer's root, a sibling of the picture and not inside it: the
//! picture's single-letter keys (`Archify`) would otherwise take the letters typed into the
//! field.

use std::sync::Arc;

use ubiq_archify::scene::{GroupKind, Scene};
use ubiq_archify::tokens::{Kind, Token};
use gpui::{
    Anchor, AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement, Styled,
    Window, div, px,
};
use gpui_component::input::InputEvent;
use crate::app::{AppState, DialogCancel};
use crate::ui::kit::{Picker, PickerStyle};
use crate::ui::{handler, indexed};

use crate::state::archify::focus::Reach;
use crate::state::archify::motion::Framing;
use crate::state::archify::{tab_of, ui};
use crate::ui::archify::theme;

/// The most rows the list shows.
pub const MAX_RESULTS: usize = 40;

/// Where the list hangs below the viewer's top: under the status strip.
const TOP_PX: f32 = 36.0;

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub node: String,
    pub label: String,
    /// The kind and where the node sits, for the faint text after the label.
    pub detail: String,
    pub kind: Option<Kind>,
}

/// The nodes matching `query` (every whitespace-separated word must occur in the node's id,
/// label, kind, sublabel, context or tags, ignoring case), labels that start with the first word
/// first, then those that contain it, then the rest, each in scene order. An empty query lists
/// every node. At most [`MAX_RESULTS`].
pub fn entries(scene: &Scene, query: &str) -> Vec<Entry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found: Vec<(u8, Entry)> = Vec::new();
    for g in scene.groups.iter().filter(|g| g.kind == GroupKind::Node) {
        let Some(node) = g.node_id.clone() else { continue };
        let kind = g.node_kind;
        let hay = [
            Some(node.as_str()),
            Some(g.label.as_str()),
            kind.map(Kind::as_str),
            g.sublabel.as_deref(),
            g.context.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(g.tags.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
        if !words.iter().all(|w| hay.contains(w.as_str())) {
            continue;
        }
        let label = g.label.to_lowercase();
        let rank = match words.first() {
            None => 0,
            Some(w) if label.starts_with(w.as_str()) => 0,
            Some(w) if label.contains(w.as_str()) => 1,
            Some(_) => 2,
        };
        let detail = [
            kind.map(|k| k.as_str().to_string()),
            g.context.clone().or_else(|| g.sublabel.clone()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
        let label = if g.label.is_empty() { node.clone() } else { g.label.clone() };
        found.push((rank, Entry { node, label, detail, kind }));
    }
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().take(MAX_RESULTS).map(|(_, e)| e).collect()
}

// --------------------------------------------------------------------------------------------- //
// Open, close, choose
// --------------------------------------------------------------------------------------------- //

/// Open the finder over picture `key`: the filter is emptied and takes the keys. Enter in it is
/// wired the first time.
pub fn open(app: &mut AppState, key: &str, window: &mut Window, cx: &mut Context<AppState>) {
    let search = app.picker_search.clone();
    search.update(cx, |state, cx| {
        state.set_value("", window, cx);
        state.focus(window, cx);
    });
    ui(cx).view(key).finder = true;
    if !std::mem::replace(&mut ui(cx).finder_bound, true) {
        cx.subscribe_in(&search, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                enter(this, window, cx);
            }
        })
        .detach();
    }
    cx.notify();
}

/// Close the finder and give the keys back to the picture.
pub fn close(app: &mut AppState, key: &str, window: &mut Window, cx: &mut Context<AppState>) {
    let _ = app;
    ui(cx).view(key).finder = false;
    if let Some(handle) = ui(cx).picture_focus.get(key).cloned() {
        window.focus(&handle, cx);
    }
    cx.notify();
}

fn scene_of(key: &str, cx: &mut Context<AppState>) -> Option<Arc<Scene>> {
    ui(cx).tabs.get(tab_of(key)).and_then(|t| t.scene.clone())
}

/// Focus `node`, end whatever lens, relationship or route was on, and frame the node with its
/// neighbours (a glide, or a jump under reduced motion).
pub fn choose(
    app: &mut AppState,
    key: &str,
    node: String,
    window: &mut Window,
    cx: &mut Context<AppState>,
) {
    let now = ui(cx).cameras.now_ms();
    ui(cx).motions.end_interaction(key, now);
    let view = ui(cx).view(key);
    view.focus = Some(node.clone());
    view.reach = Reach::Near;
    ui(cx).motions.reveal(key, vec![node], Framing::Neighbours);
    close(app, key, window, cx);
}

/// Enter in the filter field: the first row.
fn enter(app: &mut AppState, window: &mut Window, cx: &mut Context<AppState>) {
    let Some(key) = ui(cx).open_finder() else { return };
    let Some(scene) = scene_of(&key, cx) else { return };
    let query = app.picker_search.read(cx).value().to_string();
    if let Some(first) = entries(&scene, &query).into_iter().next() {
        choose(app, &key, first.node, window, cx);
    }
}

// --------------------------------------------------------------------------------------------- //
// The view
// --------------------------------------------------------------------------------------------- //

/// The list for picture `key`, when its finder is open: the kit picker with the shared filter
/// field, a coloured dot per kind, and Escape (the window's `DialogCancel`) closing it.
pub fn view(
    app: &AppState,
    key: &str,
    scene: &Scene,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    if !ui(cx).peek_view(key).finder {
        return None;
    }
    let query = app.picker_search.read(cx).value().to_string();
    let found = entries(scene, &query);
    let focused = app.picker_search.read(cx).focus_handle(cx).is_focused(window);
    let entity = cx.entity();
    let nodes: Vec<String> = found.iter().map(|e| e.node.clone()).collect();
    let (pick_key, dismiss_key, cancel_key) = (key.to_string(), key.to_string(), key.to_string());
    let picker = Picker::new("archify-finder", "Find node")
        .items(found.iter().map(|e| e.label.clone()))
        .details(found.iter().map(|e| Some(e.detail.clone())))
        .dots(
            found
                .iter()
                .map(|e| e.kind.map(|k| theme::rgba(Token::KindStroke(k)))),
        )
        .open(true)
        .anchor(Anchor::TopLeft)
        .style(PickerStyle::Chip)
        .search(&app.picker_search, focused)
        .on_pick(indexed(&entity, move |this, ix, window, cx| {
            if let Some(node) = nodes.get(ix) {
                choose(this, &pick_key, node.clone(), window, cx);
            }
        }))
        .on_dismiss(handler(&entity, move |this, window, cx| {
            close(this, &dismiss_key, window, cx)
        }));
    Some(
        div()
            .absolute()
            .top(px(TOP_PX))
            .left_2()
            .on_action(cx.listener(move |this, _: &DialogCancel, window, cx| {
                close(this, &cancel_key, window, cx)
            }))
            .child(picker)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_archify::scene::{Bounds, Group};

    fn scene() -> Scene {
        let mut s = Scene::default();
        let b = Bounds::new(0.0, 0.0, 10.0, 10.0);
        let mut node = |id: &str, kind: Kind, label: &str, context: Option<&str>, tags: &[&str]| {
            let mut g = Group::node(id, kind, label, b);
            g.context = context.map(str::to_string);
            g.tags = tags.iter().map(|t| t.to_string()).collect();
            s.groups.push(g);
        };
        node("web", Kind::Frontend, "Web app", None, &["public"]);
        node("api", Kind::Backend, "Orders API", Some("Platform \u{203a} Core"), &[]);
        node("orders-db", Kind::Database, "Orders store", Some("Platform"), &["pii"]);
        node("proxy", Kind::Backend, "Billing auth proxy", None, &[]);
        node("auth", Kind::Security, "Auth", None, &["public"]);
        s.groups.push(Group::new(GroupKind::Frame, "frame-x", b));
        s
    }

    fn ids(found: &[Entry]) -> Vec<&str> {
        found.iter().map(|e| e.node.as_str()).collect()
    }

    #[test]
    fn an_empty_query_lists_every_node_and_no_frame() {
        let s = scene();
        assert_eq!(ids(&entries(&s, "")), ["web", "api", "orders-db", "proxy", "auth"]);
        assert_eq!(entries(&s, "   ").len(), 5);
    }

    #[test]
    fn every_word_must_match_the_label_id_kind_context_or_a_tag() {
        let s = scene();
        assert_eq!(ids(&entries(&s, "orders")), ["api", "orders-db"], "by label or id");
        assert_eq!(ids(&entries(&s, "orders api")), ["api"]);
        assert_eq!(ids(&entries(&s, "DATABASE")), ["orders-db"], "by kind, ignoring case");
        assert_eq!(ids(&entries(&s, "core")), ["api"], "by context");
        assert_eq!(ids(&entries(&s, "public")), ["web", "auth"], "by tag");
        assert!(entries(&s, "nothing here").is_empty());
    }

    #[test]
    fn a_label_that_starts_with_the_word_comes_before_one_that_only_contains_it() {
        let s = scene();
        // "Billing auth proxy" is earlier in the scene but only contains the word; "Auth" starts
        // with it.
        assert_eq!(ids(&entries(&s, "auth")), ["auth", "proxy"]);
        // Both start with "orders": scene order.
        assert_eq!(ids(&entries(&s, "orders")), ["api", "orders-db"]);
        // A hit by context only ranks last of all.
        assert_eq!(ids(&entries(&s, "platform")), ["api", "orders-db"]);
    }

    #[test]
    fn an_entry_carries_the_kind_and_where_it_sits() {
        let s = scene();
        let api = entries(&s, "api").remove(0);
        assert_eq!(api.label, "Orders API");
        assert_eq!(api.kind, Some(Kind::Backend));
        assert_eq!(api.detail, "backend \u{b7} Platform \u{203a} Core");
        assert_eq!(entries(&s, "web")[0].detail, "frontend");
    }

    #[test]
    fn the_list_is_capped() {
        let mut s = Scene::default();
        for i in 0..100 {
            s.groups.push(Group::node(&format!("n{i}"), Kind::Backend, "n", Bounds::new(0.0, 0.0, 1.0, 1.0)));
        }
        assert_eq!(entries(&s, "").len(), MAX_RESULTS);
    }
}
