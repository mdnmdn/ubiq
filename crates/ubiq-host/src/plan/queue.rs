//! The doc queue: what the user said on annotated documents, waiting for the agent bound to them
//! (`D207`).
//!
//! **At most one pending prompt per agent.** A comment queued while the agent is mid-turn joins the
//! entry already waiting rather than becoming a second prompt, so an agent coming out of a turn
//! reads everything the user said in one go instead of a burst of prompts that each interrupt the
//! answer to the last.
//!
//! **The queue holds references, never text.** An entry names a thread and the comments on it;
//! the words are read back from the sidecar only at delivery ([`DocQueue::deliver`]), so a comment
//! edited since is delivered as it now reads, one deleted is left out, and a thread resolved or
//! gone by then is dropped rather than delivered. Composing at delivery is also when it is known
//! how many threads the prompt carries — a few are inlined, many are named and fetched by the
//! agent itself.
//!
//! The coordinator owns the queue and decides *when* to deliver (the agent live, taking input and
//! idle); this module decides only *what* is delivered, which is what keeps it testable with no bus.

use std::collections::HashMap;

use ubiq_proto::ids::{AnnotationId, CommentId, ProjectId};
use ubiq_proto::plan::{DocThreadRef, DocumentHandle};
use ubiq_proto::work::AgentId;

/// Up to this many threads, the prompt carries the comments themselves; beyond it, only the refs
/// and an instruction to fetch them.
pub const INLINE_LIMIT: usize = 3;

/// One thread waiting for an agent: the document, the annotation and the user's comments on it
/// since the last delivery, by id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedRef {
    pub doc: DocumentHandle,
    pub annotation_id: AnnotationId,
    /// In the order they were queued. Empty for a thread the manual ask names with nothing new on
    /// it — the thread itself is the message.
    pub comment_ids: Vec<CommentId>,
}

/// A queued thread as the sidecar reads at delivery — what [`compose`] writes the prompt from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadView {
    pub doc: DocumentHandle,
    pub annotation_id: AnnotationId,
    /// The passage the thread is about: its quote, else its block's text.
    pub quote: Option<String>,
    /// The queued comments' text as it reads now, in thread order.
    pub texts: Vec<String>,
}

/// What [`DocQueue::deliver`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery<E> {
    /// Nothing to send — no entry, or every thread in it resolved or gone (the entry is dropped).
    Nothing,
    /// Sent; the entry is gone. The text is what went out.
    Sent(String),
    /// The send failed; the entry stays, pruned of threads that are gone.
    Failed(E),
}

/// Every agent's pending entry, and the document each was last delivered.
#[derive(Default, Debug)]
pub struct DocQueue {
    pending: HashMap<AgentId, Vec<QueuedRef>>,
    /// The document the agent's last delivery was entirely about. Its next prompt omits the path
    /// when every thread is on this same document. Cleared by a delivery spanning several, by the
    /// document being unbound from the agent, and by the agent's conversation ending.
    last_doc: HashMap<AgentId, DocumentHandle>,
}

impl DocQueue {
    /// Add a thread to the agent's single pending entry. A thread already waiting gains the new
    /// comment ids rather than a second row.
    pub fn push(&mut self, agent: AgentId, queued: QueuedRef) {
        let entry = self.pending.entry(agent).or_default();
        match entry
            .iter_mut()
            .find(|held| held.doc == queued.doc && held.annotation_id == queued.annotation_id)
        {
            Some(held) => {
                for id in queued.comment_ids {
                    if !held.comment_ids.contains(&id) {
                        held.comment_ids.push(id);
                    }
                }
            }
            None => entry.push(queued),
        }
    }

    /// The refs waiting for `agent`, in the order they were queued.
    pub fn pending(&self, agent: AgentId) -> Vec<DocThreadRef> {
        self.pending
            .get(&agent)
            .map(|threads| {
                threads
                    .iter()
                    .map(|thread| DocThreadRef {
                        doc: thread.doc.clone(),
                        annotation_id: thread.annotation_id,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every agent with something waiting.
    pub fn waiting(&self) -> Vec<AgentId> {
        self.pending.keys().copied().collect()
    }

    pub fn has(&self, agent: AgentId) -> bool {
        self.pending.contains_key(&agent)
    }

    /// The project of the agent's first waiting thread.
    pub fn project_of(&self, agent: AgentId) -> Option<ProjectId> {
        self.pending
            .get(&agent)
            .and_then(|threads| threads.first())
            .map(|thread| thread.doc.project_id())
    }

    /// Drop every thread of `doc` from every agent's entry but `keep`'s, and forget it as their
    /// last document — what unbinding or moving a document does. Returns the agents whose entry
    /// changed.
    pub fn drop_doc(&mut self, doc: &DocumentHandle, keep: Option<AgentId>) -> Vec<AgentId> {
        self.last_doc
            .retain(|agent, last| Some(*agent) == keep || last != doc);
        let mut changed = Vec::new();
        self.pending.retain(|agent, threads| {
            if Some(*agent) == keep {
                return true;
            }
            let before = threads.len();
            threads.retain(|thread| &thread.doc != doc);
            if threads.len() != before {
                changed.push(*agent);
            }
            !threads.is_empty()
        });
        changed
    }

    /// Forget which document the agent was last sent — its conversation ended, so the next one
    /// has not been told about any.
    pub fn forget(&mut self, agent: AgentId) {
        self.last_doc.remove(&agent);
    }

    /// Compose the agent's pending entry from `view` — the sidecar read now, `None` for a thread
    /// resolved or gone, which is pruned — and hand it to `send`. **The entry leaves the queue only
    /// if `send` succeeds**: a failed send leaves it where it was, so anything queued after merges
    /// into it and the next delivery carries the lot.
    pub fn deliver<E>(
        &mut self,
        agent: AgentId,
        mut view: impl FnMut(&QueuedRef) -> Option<ThreadView>,
        send: impl FnOnce(&str) -> Result<(), E>,
    ) -> Delivery<E> {
        let Some(refs) = self.pending.get_mut(&agent) else {
            return Delivery::Nothing;
        };
        let mut views = Vec::new();
        refs.retain(|queued| match view(queued) {
            Some(seen) => {
                views.push(seen);
                true
            }
            None => false,
        });
        if views.is_empty() {
            self.pending.remove(&agent);
            return Delivery::Nothing;
        }
        let text = compose(&views, self.last_doc.get(&agent));
        if let Err(error) = send(&text) {
            return Delivery::Failed(error);
        }
        self.pending.remove(&agent);
        match single_doc(&views) {
            Some(doc) => {
                self.last_doc.insert(agent, doc.clone());
            }
            None => {
                self.last_doc.remove(&agent);
            }
        }
        Delivery::Sent(text)
    }
}

/// The one document every thread is on, if there is one.
fn single_doc(threads: &[ThreadView]) -> Option<&DocumentHandle> {
    let first = &threads.first()?.doc;
    threads
        .iter()
        .all(|thread| &thread.doc == first)
        .then_some(first)
}

/// How the prompt names a document.
fn label(doc: &DocumentHandle) -> String {
    match doc {
        DocumentHandle::File { rel_path, .. } => format!("`{rel_path}`"),
        DocumentHandle::Plan { task_id, .. } => format!("the plan of task {task_id}"),
        DocumentHandle::MissionDoc { task_id, name, .. } => {
            format!("the mission document `{name}` of task {task_id}")
        }
    }
}

/// Which tools answer a document's threads, and with which argument.
fn tools(doc: &DocumentHandle) -> &'static str {
    match doc {
        DocumentHandle::File { .. } => "the ubiq-doc MCP (by `path`)",
        DocumentHandle::Plan { .. } | DocumentHandle::MissionDoc { .. } => {
            "the ubiq-plan MCP (by `task_id`)"
        }
    }
}

/// The words delivered for a pending entry.
///
/// Three things vary: whether the comments are inlined (at most [`INLINE_LIMIT`] threads) or
/// only named, whether the document is named at all (not when every thread is on `last_doc`, the
/// document the agent was last told about), and whether each thread names its own document
/// (when the entry spans several).
pub fn compose(threads: &[ThreadView], last_doc: Option<&DocumentHandle>) -> String {
    let single = single_doc(threads);
    let same_as_last = single.is_some() && single == last_doc;
    let inline = threads.len() <= INLINE_LIMIT;
    let count = threads.len();
    let plural = if count == 1 { "" } else { "s" };

    let mut text = match single {
        Some(_) if same_as_last => format!(
            "The user left {count} new comment thread{plural} on the document you are working on.\n"
        ),
        Some(doc) => format!(
            "The user left {count} new comment thread{plural} on {} for you.\n",
            label(doc)
        ),
        None => format!(
            "The user left {count} new comment thread{plural} across several documents for you.\n"
        ),
    };

    if inline {
        for thread in threads {
            text.push_str(&format!("\n- annotation {}", thread.annotation_id));
            if single.is_none() {
                text.push_str(&format!(" on {}", label(&thread.doc)));
            }
            text.push('\n');
            if let Some(quote) = thread
                .quote
                .as_deref()
                .map(str::trim)
                .filter(|q| !q.is_empty())
            {
                let clipped: String = quote.chars().take(200).collect();
                let ellipsis = if clipped.len() < quote.len() {
                    "…"
                } else {
                    ""
                };
                text.push_str(&format!("  Quoted: {clipped}{ellipsis}\n"));
            }
            for comment in &thread.texts {
                text.push_str(&format!("  Comment: {}\n", comment.trim()));
            }
        }
    } else {
        // Grouped by document, in the order each document first appears.
        let mut docs: Vec<&DocumentHandle> = Vec::new();
        for thread in threads {
            if !docs.contains(&&thread.doc) {
                docs.push(&thread.doc);
            }
        }
        text.push('\n');
        for doc in docs {
            let ids: Vec<String> = threads
                .iter()
                .filter(|thread| &thread.doc == doc)
                .map(|thread| thread.annotation_id.to_string())
                .collect();
            if single.is_some() {
                text.push_str(&format!("- annotations {}\n", ids.join(", ")));
            } else {
                text.push_str(&format!(
                    "- {}: annotations {}\n",
                    label(doc),
                    ids.join(", ")
                ));
            }
        }
        text.push_str(
            "\nRead them with `list_annotations` (and `read_doc` or `read_plan` for the text).\n",
        );
    }

    let hint = match single {
        Some(doc) => tools(doc).to_string(),
        None => "the ubiq-doc MCP (files, by `path`) or the ubiq-plan MCP (plans, by `task_id`)"
            .to_string(),
    };
    text.push_str(&format!(
        "\nAnswer each thread with `reply_annotation` of {hint}, and call `resolve_annotation` once \
         it is dealt with."
    ));
    text
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn file(project: ProjectId, path: &str) -> DocumentHandle {
        DocumentHandle::File {
            project_id: project,
            rel_path: path.to_string(),
        }
    }

    /// A stand-in sidecar: comment text by id, and which annotations are still open.
    #[derive(Default)]
    struct Sidecar {
        texts: HashMap<CommentId, String>,
        closed: Vec<AnnotationId>,
    }

    impl Sidecar {
        fn comment(&mut self, doc: &DocumentHandle, text: &str) -> QueuedRef {
            let id = CommentId::generate();
            self.texts.insert(id, text.to_string());
            QueuedRef {
                doc: doc.clone(),
                annotation_id: AnnotationId::generate(),
                comment_ids: vec![id],
            }
        }

        fn view(&self, queued: &QueuedRef) -> Option<ThreadView> {
            if self.closed.contains(&queued.annotation_id) {
                return None;
            }
            Some(ThreadView {
                doc: queued.doc.clone(),
                annotation_id: queued.annotation_id,
                quote: None,
                texts: queued
                    .comment_ids
                    .iter()
                    .filter_map(|id| self.texts.get(id).cloned())
                    .collect(),
            })
        }
    }

    fn deliver(queue: &mut DocQueue, sidecar: &Sidecar, agent: AgentId) -> Option<String> {
        match queue.deliver(agent, |q| sidecar.view(q), |_| Ok::<(), ()>(())) {
            Delivery::Sent(text) => Some(text),
            _ => None,
        }
    }

    #[test]
    fn two_sends_coalesce_into_one_pending_entry_with_refs_merged() {
        let doc = file(ProjectId::generate(), "spec.md");
        let agent = AgentId::generate();
        let mut sidecar = Sidecar::default();
        let mut queue = DocQueue::default();
        let first = sidecar.comment(&doc, "same words");
        // A second, identical comment on the same thread is its own comment, not a duplicate.
        let mut again = sidecar.comment(&doc, "same words");
        again.annotation_id = first.annotation_id;
        let other = sidecar.comment(&doc, "what about that?");

        queue.push(agent, first.clone());
        queue.push(agent, again);
        queue.push(agent, other.clone());

        assert_eq!(queue.waiting(), vec![agent]);
        let refs = queue.pending(agent);
        assert_eq!(refs.len(), 2, "one ref per thread, the second send merged");
        assert_eq!(refs[0].annotation_id, first.annotation_id);
        assert_eq!(refs[1].annotation_id, other.annotation_id);

        let prompt = deliver(&mut queue, &sidecar, agent).expect("one pending prompt");
        assert_eq!(prompt.matches("same words").count(), 2);
        assert!(prompt.contains("what about that?"));
        assert!(!queue.has(agent), "delivered means nothing left waiting");
        assert_eq!(deliver(&mut queue, &sidecar, agent), None);
    }

    #[test]
    fn delivery_reads_the_comments_as_they_are_now_and_skips_closed_threads() {
        let doc = file(ProjectId::generate(), "spec.md");
        let agent = AgentId::generate();
        let mut sidecar = Sidecar::default();
        let mut queue = DocQueue::default();
        let edited = sidecar.comment(&doc, "draft");
        let closed = sidecar.comment(&doc, "never mind");
        queue.push(agent, edited.clone());
        queue.push(agent, closed.clone());

        sidecar
            .texts
            .insert(edited.comment_ids[0], "final".to_string());
        sidecar.closed.push(closed.annotation_id);
        let prompt = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(prompt.contains("final") && !prompt.contains("draft"));
        assert!(!prompt.contains(&closed.annotation_id.to_string()));

        // Every thread closed by delivery time: nothing is sent and the entry goes.
        let only = sidecar.comment(&doc, "x");
        sidecar.closed.push(only.annotation_id);
        queue.push(agent, only);
        assert_eq!(
            queue.deliver(agent, |q| sidecar.view(q), |_| Ok::<(), ()>(())),
            Delivery::Nothing
        );
        assert!(!queue.has(agent));
    }

    #[test]
    fn few_threads_are_inlined_and_many_are_only_named() {
        let doc = file(ProjectId::generate(), "spec.md");
        let view = |i: usize| ThreadView {
            doc: doc.clone(),
            annotation_id: AnnotationId::generate(),
            quote: None,
            texts: vec![format!("note {i}")],
        };
        let few: Vec<_> = (0..INLINE_LIMIT).map(view).collect();
        let prompt = compose(&few, None);
        assert!(prompt.contains("note 0") && prompt.contains("note 2"));
        assert!(!prompt.contains("list_annotations"));

        let many: Vec<_> = (0..=INLINE_LIMIT).map(view).collect();
        let prompt = compose(&many, None);
        assert!(!prompt.contains("note 0"), "refs only beyond the limit");
        for queued in &many {
            assert!(prompt.contains(&queued.annotation_id.to_string()));
        }
        assert!(prompt.contains("list_annotations"));
        assert!(prompt.contains("`spec.md`"));
    }

    #[test]
    fn the_path_is_omitted_only_for_the_document_last_delivered() {
        let project = ProjectId::generate();
        let spec = file(project, "spec.md");
        let notes = file(project, "notes.md");
        let agent = AgentId::generate();
        let mut sidecar = Sidecar::default();
        let mut queue = DocQueue::default();

        queue.push(agent, sidecar.comment(&spec, "one"));
        let first = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            first.contains("`spec.md`"),
            "first delivery names the document"
        );

        queue.push(agent, sidecar.comment(&spec, "two"));
        let second = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            !second.contains("spec.md"),
            "same document as last time: no path"
        );
        assert!(second.contains("the document you are working on"));

        queue.push(agent, sidecar.comment(&spec, "three"));
        queue.push(agent, sidecar.comment(&notes, "four"));
        let mixed = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            mixed.contains("`spec.md`") && mixed.contains("`notes.md`"),
            "per-ref paths"
        );

        queue.push(agent, sidecar.comment(&spec, "five"));
        let after = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            after.contains("`spec.md`"),
            "a mixed delivery forgets the last document"
        );

        queue.push(agent, sidecar.comment(&spec, "six"));
        deliver(&mut queue, &sidecar, agent).unwrap();
        queue.forget(agent);
        queue.push(agent, sidecar.comment(&spec, "seven"));
        let fresh = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            fresh.contains("`spec.md`"),
            "a new conversation is told the path"
        );
    }

    #[test]
    fn a_refused_send_keeps_the_entry_and_later_threads_merge_into_it() {
        let spec = file(ProjectId::generate(), "spec.md");
        let agent = AgentId::generate();
        let mut sidecar = Sidecar::default();
        let mut queue = DocQueue::default();
        queue.push(agent, sidecar.comment(&spec, "one"));

        let refused = queue.deliver(agent, |q| sidecar.view(q), |_| Err("the harness is gone"));
        assert_eq!(refused, Delivery::Failed("the harness is gone"));
        assert_eq!(queue.pending(agent).len(), 1, "nothing was lost");

        queue.push(agent, sidecar.comment(&spec, "two"));
        let seen = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            seen.contains("one") && seen.contains("two"),
            "one prompt carries both"
        );
        assert!(
            seen.contains("`spec.md`"),
            "the refused send did not count as last delivered"
        );
        assert!(!queue.has(agent));
    }

    #[test]
    fn unbinding_drops_the_documents_threads_and_its_memory() {
        let project = ProjectId::generate();
        let spec = file(project, "spec.md");
        let notes = file(project, "notes.md");
        let agent = AgentId::generate();
        let mut sidecar = Sidecar::default();
        let mut queue = DocQueue::default();
        queue.push(agent, sidecar.comment(&spec, "one"));
        deliver(&mut queue, &sidecar, agent).unwrap();
        queue.push(agent, sidecar.comment(&spec, "two"));
        queue.push(agent, sidecar.comment(&notes, "three"));

        assert_eq!(queue.drop_doc(&spec, None), vec![agent]);
        assert_eq!(queue.pending(agent).len(), 1);
        assert_eq!(queue.drop_doc(&spec, Some(agent)), Vec::<AgentId>::new());
        queue.drop_doc(&notes, None);
        assert!(!queue.has(agent));

        queue.push(agent, sidecar.comment(&spec, "four"));
        let prompt = deliver(&mut queue, &sidecar, agent).unwrap();
        assert!(
            prompt.contains("`spec.md`"),
            "unbinding forgot it as the last document"
        );
    }
}
