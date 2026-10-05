//! **The block-to-editor swap:** double-click a rendered block and edit *that block*, in place.
//!
//! One `list()` row stops being a rendered block and becomes a text editor seeded from the block's
//! own source. Every other row keeps rendering through `blocks::row` exactly as before.
//!
//! **The commit writes the buffer and nothing else.** Committing splices the edited text into the
//! document source at the block's byte range and hands the result to [`MdView::buffer`] with
//! `replace_all` — which emits `InputEvent::Change`, so the view's own debounce reparses and splices
//! the list, and the file tab's subscription marks the file dirty, exactly as for a keystroke in
//! the source editor. Nothing here reparses or touches the block table: two places that splice the
//! list are two places for the views to disagree.
//!
//! **A byte range is only valid against the parse it came from.** The editor remembers each
//! block's content hash when it opens, and [`commit_block`] refuses to produce a source unless
//! every hash is still the hash of the block at that index; [`note_stale`] says so in the row the
//! moment a reparse breaks it.
//!
//! Ported from the markdown-viewer spike's `ui/blockedit.rs` (mission T-298).

use std::ops::Range;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement, KeyBinding, MouseButton, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, actions, div,
    px,
};
use gpui_component::input::{InputEvent, Textarea, TextareaState};
use ubiq_md::{BlockKind, Document};

use super::blocks::{self, Metrics};
use super::events::MdViewEvent;
use super::view::MdView;
use crate::theme::{self, Family, Role};
use crate::ui::kit;

actions!(ubiq_mdview, [CommitBlock, CancelBlock]);

/// The key context the editor row publishes.
pub const KEY_CONTEXT: &str = "MdBlockEdit";

/// The measure the editor's height is estimated against, in monospace characters.
const WRAP_COLUMNS: usize = 100;

/// How short an editor may be, in rows.
const MIN_ROWS: usize = 3;

/// The Textarea's line height in rems — `gpui_component::Input` sets `line_height(rems(1.25))`.
const LINE_REMS: f32 = 1.25;

/// The editor's chrome — its border plus a little slack — on top of `rows × line height`.
const EDITOR_PAD: f32 = 10.0;

/// The most of the viewport an editor may take, as a fraction.
const MAX_VIEWPORT: f32 = 0.7;

/// ⌘⏎ / ⌃⏎ commit and Escape cancels — bound twice, for the row's context and for the field inside
/// it: the component library binds both keys in `Input`, the deepest node, and `MdBlockEdit > Input`
/// matches at the same depth and wins by being registered later (`app::install_key_bindings` runs
/// after `gpui_component::init`). Plain Enter is left to the field: a newline.
pub fn key_bindings() -> Vec<KeyBinding> {
    let field = "MdBlockEdit > Input";
    vec![
        KeyBinding::new("cmd-enter", CommitBlock, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-enter", CommitBlock, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CancelBlock, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-enter", CommitBlock, Some(field)),
        KeyBinding::new("ctrl-enter", CommitBlock, Some(field)),
        KeyBinding::new("escape", CancelBlock, Some(field)),
    ]
}

/// **One span of blocks, being edited.** Lives on the [`MdView`] whose `ListState` row it replaces.
pub struct BlockEdit {
    /// The root blocks this editor stands in for — one, or a heading plus the paragraph below it
    /// ([`span_for`]). The editor renders in the first row; the rest render empty.
    pub blocks: Range<usize>,
    /// Each block's content hash as of the parse the editor opened against — the staleness guard.
    pub hashes: Vec<u64>,
    /// The editor's own text: one block of markdown, not a document.
    pub state: Entity<TextareaState>,
    /// Why the last commit attempt wrote nothing, shown in the row. The editor stays open and keeps
    /// the reader's text either way.
    pub refused: Option<SharedString>,
    /// The editor's height in pixels, fixed when it opens ([`rows_for`]): a `list()` row has no
    /// height for a Textarea to fill.
    pub height: f32,
    /// Blur commits. Dropped with the editor, so a cancel cannot commit what it just cancelled.
    _blur: Subscription,
}

impl BlockEdit {
    fn refuse(mut self, refused: Refused) -> Self {
        self.refused = Some(refused.message().into());
        self
    }
}

// ── The pure part ───────────────────────────────────────────────────

/// Why a commit produced no source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// There is no block at that index any more.
    Gone,
    /// The block at that index is not the one the editor opened on.
    Stale,
}

impl Refused {
    pub fn message(self) -> &'static str {
        match self {
            Refused::Gone => "that block no longer exists — the document changed under the editor",
            Refused::Stale => "the block changed under the editor — commit refused",
        }
    }
}

/// What [`commit_block`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Commit {
    /// The whole document source, with the edited span spliced in.
    Write(String),
    /// The editor's text is the span's text; writing it back would arm a reparse for nothing.
    Unchanged,
    /// The span's range cannot be trusted. Nothing is written.
    Refused(Refused),
}

/// A block's source as the editor shows it: its bytes, less the blank line that separates it from
/// the next block — that belongs to the document's structure, and [`splice_source`] puts it back.
pub fn editable_text(source: &str, range: Range<usize>) -> &str {
    source[range].trim_end_matches('\n')
}

/// A document source with `range` replaced by `edited`, the block's own trailing newlines kept
/// exactly — so a commit can neither run two blocks together nor push them apart. Emptying a block
/// is allowed: the separator survives and the next parse has one fewer block.
pub fn splice_source(source: &str, range: Range<usize>, edited: &str) -> String {
    let original = &source[range.clone()];
    let separator = &original[original.trim_end_matches('\n').len()..];
    let body = edited.trim_end_matches('\n');

    let mut out =
        String::with_capacity(source.len() - original.len() + body.len() + separator.len());
    out.push_str(&source[..range.start]);
    out.push_str(body);
    out.push_str(separator);
    out.push_str(&source[range.end..]);
    out
}

/// **The guarded commit.** The one function that decides whether a byte range may be written at.
pub fn commit_block(doc: &Document, span: Range<usize>, hashes: &[u64], edited: &str) -> Commit {
    let range = match span_range(doc, span, hashes) {
        Ok(range) => range,
        Err(refused) => return Commit::Refused(refused),
    };
    if editable_text(&doc.source, range.clone()) == edited.trim_end_matches('\n') {
        return Commit::Unchanged;
    }
    Commit::Write(splice_source(&doc.source, range, edited))
}

/// Which root blocks one editor covers when opened on `at`: a paragraph directly under a heading
/// is edited together with it; everything else is a span of one.
pub fn span_for(doc: &Document, at: usize) -> Range<usize> {
    let widen = at > 0
        && matches!(
            doc.blocks.get(at).map(|b| &b.kind),
            Some(BlockKind::Paragraph)
        )
        && matches!(
            doc.blocks.get(at - 1).map(|b| &b.kind),
            Some(BlockKind::Heading { .. })
        );
    if widen { at - 1..at + 1 } else { at..at + 1 }
}

/// The span's byte range, if every block still has its remembered hash. The one place the
/// staleness guard lives.
fn span_range(doc: &Document, span: Range<usize>, hashes: &[u64]) -> Result<Range<usize>, Refused> {
    let blocks = doc.blocks.get(span).ok_or(Refused::Gone)?;
    if blocks.len() != hashes.len() {
        return Err(Refused::Gone);
    }
    if blocks
        .iter()
        .zip(hashes)
        .any(|(block, hash)| block.hash != *hash)
    {
        return Err(Refused::Stale);
    }
    match (blocks.first(), blocks.last()) {
        (Some(first), Some(last)) => Ok(first.range.start..last.range.end),
        _ => Err(Refused::Gone),
    }
}

/// How many rows tall the editor for this text opens, soft wrap included: at least the rendered
/// block's height in editor lines (`rendered_rows`) and the source's own line count, capped at
/// `max_rows`, never under [`MIN_ROWS`]. Computed, not grown — a `list()` row whose content grows
/// after the measure is a clipped row.
pub fn rows_for(text: &str, rendered_rows: usize, max_rows: usize) -> usize {
    let rows: usize = text
        .lines()
        .map(|line| 1 + line.chars().count() / WRAP_COLUMNS)
        .sum();
    rows.max(rendered_rows).min(max_rows).max(MIN_ROWS)
}

// ── Opening, committing, cancelling ─────────────────────────────────

/// Void the span's measured heights with a same-count splice — `remeasure` would drop every other
/// row's height too.
fn remeasure(view: &MdView, span: Range<usize>) {
    if span.end <= view.doc.blocks.len() {
        let count = span.len();
        view.list.splice(span, count);
    }
}

/// **Open an editor on root block `at`**, seeded from its source. An editor open elsewhere commits
/// first; if that is refused the new one does not open.
pub fn open(view: &mut MdView, at: usize, window: &mut Window, cx: &mut Context<MdView>) {
    if !view.editable {
        return;
    }
    let open_on = view.edit.as_ref().map(|edit| edit.blocks.clone());
    if open_on.as_ref().is_some_and(|span| span.contains(&at)) {
        return;
    }
    if open_on.is_some() && !commit(view, window, cx) {
        return;
    }
    if view.doc.blocks.get(at).is_none() {
        return;
    }
    let span = span_for(&view.doc, at);
    let hashes: Vec<u64> = view.doc.blocks[span.clone()]
        .iter()
        .map(|b| b.hash)
        .collect();
    let Ok(range) = span_range(&view.doc, span.clone(), &hashes) else {
        return;
    };
    let seed = editable_text(&view.doc.source, range).to_string();
    let measured: f32 = span
        .clone()
        .map(|ix| {
            view.list
                .bounds_for_item(ix)
                .map_or(0.0, |b| f32::from(b.size.height))
        })
        .sum();
    let viewport = f32::from(view.list.viewport_bounds().size.height);
    let line = f32::from(window.rem_size()) * LINE_REMS;
    let rows = rows_for(
        &seed,
        (measured / line).round() as usize,
        (viewport * MAX_VIEWPORT / line) as usize,
    );
    let height = rows as f32 * line + EDITOR_PAD;

    let state = cx.new(|cx| {
        TextareaState::new(window, cx)
            .soft_wrap(true)
            .default_value(seed)
    });
    state.update(cx, |state, cx| state.focus(window, cx));

    // Blur commits: clicking away from a block finishes editing it.
    let blur = cx.subscribe_in(
        &state,
        window,
        |this, _state, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Blur) {
                commit(this, window, cx);
            }
        },
    );

    view.edit = Some(BlockEdit {
        blocks: span.clone(),
        hashes,
        state,
        refused: None,
        height,
        _blur: blur,
    });
    remeasure(view, span);
    cx.notify();
}

/// The read-only / editable switch. Going read-only commits an open editor first; a refused commit
/// keeps the editor — and the switch — where they are.
pub fn set_editable(view: &mut MdView, on: bool, window: &mut Window, cx: &mut Context<MdView>) {
    if !on {
        commit(view, window, cx);
        if view.edit.is_some() {
            return;
        }
    }
    view.editable = on;
    cx.notify();
}

/// **Commit the open editor.** `true` when the row is no longer an editor — including nothing
/// open, and a commit that wrote nothing because nothing changed.
pub fn commit(view: &mut MdView, window: &mut Window, cx: &mut Context<MdView>) -> bool {
    let Some(edit) = view.edit.take() else {
        return true;
    };
    let span = edit.blocks.clone();
    let edited = edit.state.read(cx).value().to_string();

    let outcome = write(view, span.clone(), &edit.hashes, &edited, window, cx);
    if let Err(refused) = outcome {
        // Refusing to write is not a licence to throw away what the reader typed.
        view.edit = Some(edit.refuse(refused));
    }
    remeasure(view, span);

    // Focus leaves a row that is about to stop being an editor, or it lands on a dropped entity.
    if outcome.is_ok() {
        window.focus(&view.focus, cx);
    }
    cx.notify();
    outcome.is_ok()
}

/// **The guarded write:** run [`commit_block`] for the span, and on `Write` put the new source into
/// the buffer and emit [`MdViewEvent::BlockEdited`]. `Ok(true)` written, `Ok(false)` unchanged,
/// `Err` refused — nothing written.
///
/// `replace_all` rather than `set_value`: `set_value` suppresses `InputEvent::Change`, and that
/// event is what reparses the view and dirties the file. It also keeps the buffer's undo history,
/// so ⌘Z in the source editor undoes a block commit.
pub fn write(
    view: &mut MdView,
    span: Range<usize>,
    hashes: &[u64],
    edited: &str,
    window: &mut Window,
    cx: &mut Context<MdView>,
) -> Result<bool, Refused> {
    let source = match commit_block(&view.doc, span.clone(), hashes, edited) {
        Commit::Write(source) => source,
        Commit::Unchanged => return Ok(false),
        Commit::Refused(refused) => return Err(refused),
    };
    let old_range = span_range(&view.doc, span.clone(), hashes)?;
    view.buffer
        .update(cx, |state, cx| state.replace_all(source, window, cx));
    cx.emit(MdViewEvent::BlockEdited {
        blocks: span,
        old_range,
        new_text: edited.trim_end_matches('\n').to_string(),
    });
    Ok(true)
}

/// **Escape: close the editor and write nothing.** The buffer was never touched.
pub fn cancel(view: &mut MdView, window: &mut Window, cx: &mut Context<MdView>) {
    let Some(edit) = view.edit.take() else {
        return;
    };
    remeasure(view, edit.blocks.clone());
    window.focus(&view.focus, cx);
    cx.emit(MdViewEvent::EditCancelled {
        blocks: edit.blocks,
    });
    cx.notify();
}

/// **A reparse happened while an editor was open.** Check the guard now rather than at ⌘⏎, and
/// return whether this call is what broke it.
pub fn note_stale(view: &mut MdView) -> bool {
    let Some(edit) = view.edit.as_ref() else {
        return false;
    };
    if edit.refused.is_some() {
        return false;
    }
    let Err(refused) = span_range(&view.doc, edit.blocks.clone(), &edit.hashes) else {
        return false;
    };
    if let Some(edit) = view.edit.take() {
        view.edit = Some(edit.refuse(refused));
    }
    true
}

// ── The row ─────────────────────────────────────────────────────────

/// **One `list()` row: the block, or the editor standing in for it.** A click selects the block; a
/// double-click opens the editor on it when the view is editable.
pub fn row(view: &WeakEntity<MdView>, this: &MdView, ix: usize, m: &Metrics) -> AnyElement {
    if let Some(edit) = this.edit.as_ref().filter(|edit| edit.blocks.contains(&ix)) {
        return if edit.blocks.start == ix {
            editor_row(view, edit, m)
        } else {
            div().into_any_element()
        };
    }

    let toggle = view.clone();
    let front = matches!(this.doc.blocks[ix].kind, BlockKind::FrontMatter { .. }).then(|| {
        blocks::FrontToggle {
            open: this.front_open,
            on_click: Box::new(move |_, _, cx: &mut App| {
                toggle.update(cx, |this, cx| this.toggle_front(ix, cx)).ok();
            }),
        }
    });
    let rendered = blocks::row(&this.doc, ix, m, Some(&this.find), front);
    let click = view.clone();
    let editable = this.editable;

    div()
        .id(("md-block", ix))
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .on_click(move |event: &ClickEvent, window, cx: &mut App| {
            click
                .update(cx, |this, cx| {
                    if editable && event.click_count() >= 2 {
                        open(this, ix, window, cx);
                    } else {
                        this.select_block(ix, cx);
                    }
                })
                .ok();
        })
        .child(rendered)
        // Absolute, so the decor cannot change the row's measured height (`annotation.rs`).
        .children(super::annotation::overlays(view, this, ix, m))
        .into_any_element()
}

/// The editor in the page's own inset, so the text does not jump sideways when the row changes.
fn editor_row(view: &WeakEntity<MdView>, edit: &BlockEdit, m: &Metrics) -> AnyElement {
    let commit_with = view.clone();
    let cancel_with = view.clone();

    div()
        .key_context(KEY_CONTEXT)
        .flex()
        .flex_col()
        .w_full()
        .pl(px(m.inset))
        .pr(px(m.margin))
        .pt(px(m.body))
        .gap_1()
        .on_action(move |_: &CommitBlock, window, cx: &mut App| {
            commit_with
                .update(cx, |this, cx| {
                    commit(this, window, cx);
                })
                .ok();
        })
        .on_action(move |_: &CancelBlock, window, cx: &mut App| {
            cancel_with
                .update(cx, |this, cx| cancel(this, window, cx))
                .ok();
        })
        .child(
            Textarea::new(&edit.state)
                .h(px(edit.height))
                .appearance(true)
                .bordered(true)
                .font_family(theme::MONO_FONT)
                .text_size(theme::font(Family::Content, Role::Body)),
        )
        .child(hint(view, edit))
        .into_any_element()
}

/// Under the editor: Save, Cancel, the keys, and the refusal when there is one.
fn hint(view: &WeakEntity<MdView>, edit: &BlockEdit) -> AnyElement {
    let save = view.clone();
    let discard = view.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .text_size(theme::font(Family::Chrome, Role::Label))
        .child(kit::primary_button(
            "md-block-save",
            None,
            "Save",
            move |_: &ClickEvent, window, cx: &mut App| {
                save.update(cx, |this, cx| {
                    commit(this, window, cx);
                })
                .ok();
            },
        ))
        .child(
            kit::ghost_button(
                "md-block-cancel",
                None,
                "Cancel",
                move |_: &ClickEvent, window, cx: &mut App| {
                    discard.update(cx, |this, cx| cancel(this, window, cx)).ok();
                },
            )
            // The view tracks a focus handle, so a mouse-down that bubbles to it takes focus from
            // the editor — and blur commits, which would make Cancel a Save.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default()),
        )
        .child(
            div()
                .text_color(theme::text_faint())
                .child("⌘⏎ save · esc cancel"),
        )
        .when_some(edit.refused.clone(), |this, refused| {
            this.child(div().text_color(theme::warning()).child(refused))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(source: &str) -> Document {
        ubiq_md::parse(source)
    }

    fn range_of(doc: &Document, at: usize) -> Range<usize> {
        doc.blocks[at].range.clone()
    }

    fn commit_one(doc: &Document, at: usize, hash: u64, edited: &str) -> Commit {
        commit_block(doc, at..at + 1, &[hash], edited)
    }

    fn hashes_of(doc: &Document, span: Range<usize>) -> Vec<u64> {
        doc.blocks[span].iter().map(|b| b.hash).collect()
    }

    // ── The span ────────────────────────────────────────────────────

    #[test]
    fn a_paragraph_under_a_heading_widens_the_span() {
        let document = doc("# Title\n\nalpha\n\nbeta\n");
        assert_eq!(span_for(&document, 1), 0..2);
    }

    #[test]
    fn the_span_stays_single_everywhere_else() {
        let document = doc("# Title\n\n- a\n- b\n\nalpha\n\nbeta\n");
        assert_eq!(span_for(&document, 0), 0..1, "the heading itself");
        assert_eq!(span_for(&document, 1), 1..2, "a list under a heading");
        assert_eq!(span_for(&document, 2), 2..3, "a paragraph under a list");
        assert_eq!(
            span_for(&document, 3),
            3..4,
            "a paragraph under a paragraph"
        );
        assert_eq!(
            span_for(&doc("alpha\n\nbeta\n"), 0),
            0..1,
            "the first block"
        );
    }

    #[test]
    fn the_combined_editor_is_seeded_with_the_separator() {
        let document = doc("# Title\n\nalpha\n\nbeta\n");
        let range = span_range(&document, 0..2, &hashes_of(&document, 0..2)).unwrap();
        assert_eq!(editable_text(&document.source, range), "# Title\n\nalpha");
    }

    #[test]
    fn a_combined_commit_splices_the_whole_span_and_reparses_apart() {
        let before = doc("# Title\n\nalpha\n\nbeta\n");
        let Commit::Write(source) = commit_block(
            &before,
            0..2,
            &hashes_of(&before, 0..2),
            "# New title\n\nALPHA",
        ) else {
            panic!("a fresh span commits");
        };
        assert_eq!(source, "# New title\n\nALPHA\n\nbeta\n");
        let after = doc(&source);
        assert_eq!(after.blocks.len(), 3);
        assert!(matches!(after.blocks[0].kind, BlockKind::Heading { .. }));
        assert!(matches!(after.blocks[1].kind, BlockKind::Paragraph));
        assert_eq!(after.blocks[2].hash, before.blocks[2].hash);
    }

    #[test]
    fn a_combined_commit_refuses_when_either_block_is_stale() {
        let opened = doc("# Title\n\nalpha\n\nbeta\n");
        let hashes = hashes_of(&opened, 0..2);
        assert_eq!(
            commit_block(&doc("# Other\n\nalpha\n\nbeta\n"), 0..2, &hashes, "x"),
            Commit::Refused(Refused::Stale)
        );
        assert_eq!(
            commit_block(
                &doc("# Title\n\nalpha changed\n\nbeta\n"),
                0..2,
                &hashes,
                "x"
            ),
            Commit::Refused(Refused::Stale)
        );
        assert_eq!(
            commit_block(&doc("# Title\n"), 0..2, &hashes, "x"),
            Commit::Refused(Refused::Gone)
        );
    }

    #[test]
    fn an_unchanged_combined_editor_writes_nothing() {
        let document = doc("# Title\n\nalpha\n\nbeta\n");
        assert_eq!(
            commit_block(
                &document,
                0..2,
                &hashes_of(&document, 0..2),
                "# Title\n\nalpha\n"
            ),
            Commit::Unchanged
        );
    }

    // ── The splice ──────────────────────────────────────────────────

    #[test]
    fn edits_first_middle_and_last_blocks() {
        let document = doc("alpha\n\nbeta\n\ngamma\n");
        let at = |ix, text| splice_source(&document.source, range_of(&document, ix), text);
        assert_eq!(at(0, "ALPHA"), "ALPHA\n\nbeta\n\ngamma\n");
        assert_eq!(at(1, "BETA"), "alpha\n\nBETA\n\ngamma\n");
        assert_eq!(at(2, "GAMMA"), "alpha\n\nbeta\n\nGAMMA\n");
    }

    #[test]
    fn a_block_replaced_by_empty_text() {
        let document = doc("alpha\n\nbeta\n\ngamma\n");
        let spliced = splice_source(&document.source, range_of(&document, 1), "");
        assert_eq!(spliced, "alpha\n\n\n\ngamma\n");
        let after = doc(&spliced);
        assert_eq!(after.blocks.len(), 2);
        assert_eq!(after.block_source(1).map(str::trim), Some("gamma"));
    }

    #[test]
    fn the_separator_is_the_blocks_own_whatever_was_typed() {
        let document = doc("alpha\n\nbeta\n\ngamma\n");
        let range = range_of(&document, 1);
        assert_eq!(
            splice_source(&document.source, range.clone(), "BETA\n\n\n"),
            splice_source(&document.source, range, "BETA")
        );
    }

    #[test]
    fn a_multi_line_block_keeps_its_own_newlines() {
        let document = doc("alpha\n\n```rust\nfn main() {}\n```\n\ngamma\n");
        assert_eq!(
            editable_text(&document.source, range_of(&document, 1)),
            "```rust\nfn main() {}\n```"
        );
        let spliced = splice_source(
            &document.source,
            range_of(&document, 1),
            "```rust\nfn main() { todo!() }\n```",
        );
        assert_eq!(
            spliced,
            "alpha\n\n```rust\nfn main() { todo!() }\n```\n\ngamma\n"
        );
    }

    // ── The guard ───────────────────────────────────────────────────

    #[test]
    fn commits_against_the_parse_it_opened_on() {
        let document = doc("alpha\n\nbeta\n\ngamma\n");
        assert_eq!(
            commit_one(&document, 1, document.blocks[1].hash, "BETA"),
            Commit::Write("alpha\n\nBETA\n\ngamma\n".to_string())
        );
    }

    #[test]
    fn refuses_a_stale_hash_and_a_block_that_is_gone() {
        let opened = doc("alpha\n\nbeta\n\ngamma\n");
        let moved = doc("alpha\n\nbeta rewritten by somebody else\n\ngamma\n");
        assert_eq!(
            commit_one(&moved, 1, opened.blocks[1].hash, "BETA"),
            Commit::Refused(Refused::Stale)
        );
        assert_eq!(
            commit_one(&doc("alpha\n"), 2, opened.blocks[2].hash, "GAMMA"),
            Commit::Refused(Refused::Gone)
        );
    }

    #[test]
    fn nothing_typed_writes_nothing() {
        let document = doc("alpha\n\nbeta\n\ngamma\n");
        let hash = document.blocks[1].hash;
        let seed = editable_text(&document.source, range_of(&document, 1));
        assert_eq!(commit_one(&document, 1, hash, seed), Commit::Unchanged);
        assert_eq!(
            commit_one(&document, 1, hash, &format!("{seed}\n")),
            Commit::Unchanged
        );
    }

    // ── What the reparse sees ───────────────────────────────────────

    #[test]
    fn a_commit_splices_exactly_one_row() {
        // Committing one block is an edit to one block, so the reparse's delta is one row and every
        // other row keeps the height the layout measured.
        let mut document = doc("# Title\n\nalpha\n\nbeta\n\ngamma\n\n## End\n");
        let before = document.blocks.len();
        let Commit::Write(source) = commit_one(
            &document,
            2,
            document.blocks[2].hash,
            "beta, rewritten at length",
        ) else {
            panic!("a fresh hash commits");
        };
        let delta = document.reparse(&source);
        let (rows, count) = super::super::sync::splice_range(delta);
        assert_eq!(rows, 2..3);
        assert_eq!(count, 1);
        assert_eq!(document.blocks.len(), before);
    }

    #[test]
    fn a_commit_that_splits_a_block_leaves_the_blocks_above_alone() {
        let before = doc("alpha\n\nbeta\n\ngamma\n");
        let Commit::Write(source) =
            commit_one(&before, 1, before.blocks[1].hash, "beta one\n\nbeta two")
        else {
            panic!("a fresh hash commits");
        };
        let after = doc(&source);
        assert_eq!(after.blocks.len(), 4);
        assert_eq!(after.blocks[0].hash, before.blocks[0].hash);
        assert_eq!(after.blocks[3].hash, before.blocks[2].hash);
    }

    // ── The row's height ────────────────────────────────────────────

    #[test]
    fn rows_are_estimated_from_the_text_and_clamped() {
        const CAP: usize = 30;
        assert_eq!(rows_for("", 0, CAP), MIN_ROWS);
        assert_eq!(rows_for("alpha", 1, CAP), MIN_ROWS);
        assert_eq!(rows_for("alpha", 7, CAP), 7);
        assert_eq!(rows_for(&"x".repeat(450), 0, CAP), 5);
        assert_eq!(rows_for(&"line\n".repeat(20), 4, CAP), 20);
        assert_eq!(rows_for(&"line\n".repeat(200), 0, CAP), CAP);
        assert_eq!(rows_for(&"line\n".repeat(20), 0, 2), MIN_ROWS);
    }
}
