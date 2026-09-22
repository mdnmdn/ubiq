//! An agent's question to the user: where it is filed, when it interrupts, and what goes back.
//!
//! **The ask arrives unasked for.** Every other modal in this window is raised by a gesture; this
//! one is raised by a harness that has parked a tool call and is waiting. So the first thing this
//! decides is whether to interrupt at all: the dialog opens only where the user is already looking
//! at that conversation *and* nothing else is up, because a dialog that lands on top of a
//! half-filled form is a dialog that took the form away. Anything else raises the bell instead.
//!
//! **The transcript entry is written either way.** The notification is a flash and the bell's list
//! is a history; what makes an ask findable an hour later is the entry in the conversation that
//! raised it, and it is the same entry whether the dialog opened at once or not.
//!
//! **Closing is not answering.** Escape, the ×, an outside click — all three take the view away
//! and touch nothing that was typed, so the entry's button reopens the same drafts. Only Confirm
//! and `Chat about this` send, and each of them sends exactly once: the record leaves
//! [`AskStage::Waiting`], which is what makes every later opening read-only.

use ubiq_proto::ask::{AskClosed, AskOutcome, AskQuestion};

use crate::state::ask::{AskDialog, AskRecord, AskStage};

use super::*;

impl AppState {
    // ── Reading ─────────────────────────────────────────────────────

    /// The record the dialog is drawing, where one is up.
    pub fn open_ask(&self, cx: &App) -> Option<(&AskDialog, &AskRecord)> {
        let dialog = self.workbench.ask.as_ref()?;
        let record = self.ask_record(dialog.agent_id, dialog.ask_id, cx)?;
        Some((dialog, record))
    }

    /// One ask by the two ids that name it. Across every project the window holds, the way a pane
    /// is found: an agent belongs to a project, and nothing that reaches here knows which.
    pub fn ask_record(&self, agent_id: AgentId, ask_id: AskId, _cx: &App) -> Option<&AskRecord> {
        self.projects
            .values()
            .find_map(|open| open.conversations.get(&agent_id))
            .and_then(|conversation| conversation.ask(ask_id))
    }

    fn ask_record_mut(&mut self, agent_id: AgentId, ask_id: AskId) -> Option<&mut AskRecord> {
        self.projects
            .values_mut()
            .find_map(|open| open.conversations.get_mut(&agent_id))
            .and_then(|conversation| conversation.ask_mut(ask_id))
    }

    /// The record the dialog is on, for a mutator that is about to change it.
    fn dialog_record_mut(&mut self) -> Option<(usize, &mut AskRecord)> {
        let dialog = self.workbench.ask?;
        let record = self.ask_record_mut(dialog.agent_id, dialog.ask_id)?;
        Some((dialog.tab, record))
    }

    // ── Raising and dismissing ──────────────────────────────────────

    /// Show one ask — what the transcript entry's button does. Refuses while anything else is up,
    /// for the reason [`Self::asked`] refuses: a modal over a modal is a modal that took the one
    /// underneath away.
    pub fn show_ask(
        &mut self,
        agent_id: AgentId,
        ask_id: AskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.top_layer().is_some() {
            return;
        }
        if self.ask_record(agent_id, ask_id, cx).is_none() {
            return;
        }
        self.workbench.ask = Some(AskDialog {
            agent_id,
            ask_id,
            tab: 0,
            cursor: 0,
        });
        self.set_ask_fields(window, cx);
        self.pending_ask_focus = true;
        cx.notify();
    }

    /// [`Self::show_ask`] for a question that has just arrived, which is the one path with no
    /// window on the stack: the host's messages are routed by a task, not by a gesture.
    ///
    /// Sound because the two fields are empty whenever no dialog is up — every close clears them,
    /// which is the same discipline the feedback modal's fields keep — and a record raised here
    /// has never been typed into. Reopening one that *has* been goes through `show_ask`.
    fn raise_fresh_ask(&mut self, agent_id: AgentId, ask_id: AskId, cx: &mut Context<Self>) {
        self.workbench.ask = Some(AskDialog {
            agent_id,
            ask_id,
            tab: 0,
            cursor: 0,
        });
        // No `Window` on this path — see `fill_ask_fields`'s own reason — so the grab is queued
        // and drained the next time the window renders.
        self.pending_ask_focus = true;
        cx.notify();
    }

    /// Put the dialog away without answering. Nothing is sent and nothing typed is lost — the
    /// drafts are the record's, and the record stays on the conversation.
    pub fn close_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.ask = None;
        self.set_ask_fields(window, cx);
        cx.notify();
    }

    /// Put the dialog away when the record it names has gone — the conversation it was filed on
    /// was deleted under it. Nothing is sent: the delete already told the host, and there is no
    /// record left to answer.
    ///
    /// Without this the dialog draws an empty `div` while `workbench.ask` stays `Some`, so
    /// [`Layer::Ask`](crate::state::Layer::Ask) holds the stack: every later ask is downgraded to
    /// a notification, [`Self::show_ask`] refuses, and nothing on screen can be dismissed.
    pub(super) fn settle_ask_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.workbench.ask else {
            return;
        };
        if self
            .ask_record(dialog.agent_id, dialog.ask_id, cx)
            .is_some()
        {
            return;
        }
        self.workbench.ask = None;
        // The two fields still hold what was typed and a message carries no window to clear them
        // with, so the next frame does it — `fill_ask_fields`'s reason.
        self.refill_ask_fields = true;
        cx.notify();
    }

    /// Clear the dialog's two fields after a close that had no window to clear them with. Drained
    /// in `render` for `fill_project_form`'s reason: `set_value` needs a window, and the message
    /// that took the conversation away does not carry one.
    pub(super) fn fill_ask_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.refill_ask_fields {
            self.refill_ask_fields = false;
            self.set_ask_fields(window, cx);
        }
        // Queued by `show_ask` and `raise_fresh_ask`, neither of which is guaranteed a `Window` —
        // the second arrives on the host's own task. Drained here, the one place both paths are
        // sure to reach a frame with one.
        if self.pending_ask_focus {
            self.pending_ask_focus = false;
            self.ask_focus.clone().focus(window, cx);
        }
    }

    /// Switch question. The drafts are already current — every keystroke is mirrored as it is
    /// typed — so this only has to point the two fields at the question now on screen, and put
    /// the keyboard cursor back on the first option: a cursor left where the last question's list
    /// ended would read as a pick on a question the reader never visited.
    pub fn set_ask_tab(&mut self, tab: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.workbench.ask.as_mut() {
            dialog.tab = tab;
            dialog.cursor = 0;
        }
        self.set_ask_fields(window, cx);
        cx.notify();
    }

    /// Mirror the question on screen into the two text fields, on `set_feedback_fields`'s
    /// discipline: one direction per event, so the field the user is typing in is never written
    /// back over from the state it is writing to.
    fn set_ask_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (other, notes) = self
            .open_ask(cx)
            .and_then(|(dialog, record)| record.drafts.get(dialog.tab))
            .map(|draft| (draft.other.clone(), draft.notes.clone()))
            .unwrap_or_default();
        self.ask_other_input
            .clone()
            .update(cx, |input, cx| input.set_value(&other, window, cx));
        self.ask_notes_input
            .clone()
            .update(cx, |input, cx| input.set_value(&notes, window, cx));
    }

    // ── The controls ────────────────────────────────────────────────

    /// Pick or unpick one option of the question on screen. `option` is a position, and the one
    /// past the agent's own options is "Other".
    pub fn toggle_ask_option(&mut self, option: usize, cx: &mut Context<Self>) {
        if let Some((tab, record)) = self.dialog_record_mut()
            && record.live()
        {
            record.toggle(tab, option);
        }
        cx.notify();
    }

    // ── The keyboard ────────────────────────────────────────────────
    //
    // `AskMoveUp` / `AskMoveDown` walk the cursor over the question on screen; `AskToggle` (space)
    // picks or unpicks wherever it sits, the same thing a click on that card does. Plain `enter`
    // (`DialogConfirm`) does that and moves on to the next question, and `⌘⏎` (`SubmitSearch`)
    // does the moving on its own — confirming the dialog outright on the last question, since
    // there is nowhere left to move to. None of the four fires while a field has the keyboard:
    // `DialogConfirm` and `AskMoveUp`/`AskMoveDown`/`AskToggle` are bound only outside `Input`, on
    // `new_agent.rs`'s reasoning, so typing a space or an arrow key in "Other" or "Notes" still
    // does what typing does. `⌘⏎` is the one exception, bound inside a field too, because it is
    // the platform's usual "send this" chord and a note half-written is still worth submitting
    // from.

    /// Move the keyboard cursor over the question on screen, clamped to its own option list
    /// — `options.len()` is "Other", the last stop either direction can reach.
    pub fn move_ask_cursor(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some((tab, record)) = self.dialog_record_mut() else {
            return;
        };
        let last = record.other_at(tab);
        let Some(dialog) = self.workbench.ask.as_mut() else {
            return;
        };
        dialog.cursor = dialog
            .cursor
            .saturating_add_signed(delta as isize)
            .min(last);
        cx.notify();
    }

    /// Space: pick or unpick whatever the cursor is on, exactly as clicking that card would.
    pub fn toggle_ask_cursor(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.workbench.ask else {
            return;
        };
        self.toggle_ask_option(dialog.cursor, cx);
    }

    /// Plain `enter`: pick the cursor's option, then move to the next question — the one thing
    /// left for a reader who never touches the mouse. Does nothing further on the last question;
    /// `⌘⏎` is what confirms from there.
    pub fn advance_ask_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.workbench.ask else {
            return;
        };
        self.toggle_ask_option(dialog.cursor, cx);
        let Some((tab, record)) = self.dialog_record_mut() else {
            return;
        };
        if tab + 1 < record.questions.len() {
            self.set_ask_tab(tab + 1, window, cx);
        }
    }

    /// `⌘⏎`: move to the next question, or confirm the dialog outright from the last one — the
    /// keyboard's way through the whole ask without a click.
    pub fn confirm_ask_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tab, record)) = self.dialog_record_mut() else {
            return;
        };
        if tab + 1 < record.questions.len() {
            self.set_ask_tab(tab + 1, window, cx);
        } else {
            self.confirm_ask(window, cx);
        }
    }

    /// `tab`, while "Other" or "Notes" has the keyboard: hand it to the other one. The question's
    /// only two fields, so this is the whole of "next" — nothing to do where neither is focused.
    pub fn ask_next_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let other_focused = self
            .ask_other_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let notes_focused = self
            .ask_notes_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if other_focused {
            self.ask_notes_input
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        if notes_focused {
            let Some(dialog) = self.workbench.ask else {
                return;
            };
            let picked_other = self
                .ask_record(dialog.agent_id, dialog.ask_id, cx)
                .is_some_and(|record| record.picked(dialog.tab, record.other_at(dialog.tab)));
            if picked_other {
                self.ask_other_input
                    .clone()
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
    }

    pub fn retype_ask_other(&mut self, typed: String, cx: &mut Context<Self>) {
        if let Some((tab, record)) = self.dialog_record_mut()
            && record.live()
            && let Some(draft) = record.drafts.get_mut(tab)
        {
            draft.other = typed;
            cx.notify();
        }
    }

    pub fn renote_ask(&mut self, typed: String, cx: &mut Context<Self>) {
        if let Some((tab, record)) = self.dialog_record_mut()
            && record.live()
            && let Some(draft) = record.drafts.get_mut(tab)
        {
            draft.notes = typed;
            cx.notify();
        }
    }

    /// Send the answers. Refuses in place while a question is unanswered, on `send_feedback`'s
    /// discipline: the button is drawn dim rather than unwired, so the rule lives in one place.
    pub fn confirm_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.workbench.ask else {
            return;
        };
        let Some(record) = self.ask_record_mut(dialog.agent_id, dialog.ask_id) else {
            return;
        };
        if !record.ready() {
            return;
        }
        let answers = record.answer();
        record.stage = AskStage::Answered(answers.clone());
        self.bus.send(Message::AnswerAsk {
            agent_id: dialog.agent_id,
            ask_id: dialog.ask_id,
            outcome: AskOutcome::Answered(answers),
        });
        self.close_ask(window, cx);
    }

    /// "Chat about this": end the ask with no answer to any question and let the user type. A
    /// real outcome — the agent is told so — which is why it sends rather than merely closing.
    pub fn chat_about_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.workbench.ask else {
            return;
        };
        let Some(record) = self.ask_record_mut(dialog.agent_id, dialog.ask_id) else {
            return;
        };
        if !record.live() {
            return;
        }
        record.stage = AskStage::Chatted;
        self.bus.send(Message::AnswerAsk {
            agent_id: dialog.agent_id,
            ask_id: dialog.ask_id,
            outcome: AskOutcome::Chat,
        });
        self.close_ask(window, cx);
    }

    // ── What the host says ──────────────────────────────────────────

    /// The ask family: a question raised out of nothing, and the word that one stopped waiting.
    ///
    /// Answers with the message when it belongs to another family, on the convention every other
    /// `receive_*` in `wire.rs` follows.
    pub(super) fn receive_ask(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::AskUser {
                agent_id,
                ask_id,
                questions,
            } => self.asked(agent_id, ask_id, questions, cx),
            Message::AskEnded {
                agent_id,
                ask_id,
                why,
            } => self.ask_ended(agent_id, ask_id, why, cx),
            other => return Some(other),
        }
        None
    }

    /// File a question and decide whether it interrupts.
    fn asked(
        &mut self,
        agent_id: AgentId,
        ask_id: AskId,
        questions: Vec<AskQuestion>,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self
            .projects
            .values_mut()
            .find(|open| open.conversations.contains_key(&agent_id))
        else {
            // Nothing to file it against and nothing to draw it in — the project was closed
            // between the tool call and the push. Inventing a conversation to hang it on would be
            // worse, so say so instead: `AskEnded` travels this way too, and the host frees the
            // parked call at once rather than leaving the harness to wait out its timeout.
            tracing::warn!("ask {ask_id}: no conversation for agent {agent_id}");
            self.bus.send(Message::AskEnded {
                agent_id,
                ask_id,
                why: AskClosed::Gone,
            });
            return;
        };
        // Read before the borrow ends: the bell needs the agent's name, per `G198`.
        let actor = open.work.agent(agent_id).map(|agent| agent.name.clone());
        let shown = conversation_shown(open, agent_id);
        let headers: Vec<String> = questions
            .iter()
            .map(|question| question.header.clone())
            .collect();
        if let Some(conversation) = open.conversations.get_mut(&agent_id) {
            // How many blocks the transcript held the instant this ask landed, which is where its
            // row is drawn — see `AskRecord::at_block`.
            let at_block = conversation.blocks.len();
            conversation.file_ask(AskRecord::new(ask_id, questions, at_block));
        }

        // A dialog already up counts as "the agent is not visible": whatever the user is doing,
        // they are not looking at this transcript, and taking their modal away to ask them
        // something else is the interruption this rule exists to prevent.
        if shown && self.top_layer().is_none() {
            self.raise_fresh_ask(agent_id, ask_id, cx);
            return;
        }
        let mut request = NotificationRequest::new(
            Level::Warning,
            Family::Agents,
            match headers.is_empty() {
                true => "Is asking for your feedback.".to_string(),
                false => format!("Is asking for your feedback \u{b7} {}", headers.join(", ")),
            },
        )
        .with_category("ask")
        .with_link(UbiqLink::Agent(agent_id))
        .with_os();
        if let Some(actor) = actor {
            request = request.with_actor(actor);
        }
        self.raise_notification(request);
        cx.notify();
    }

    /// The host stopped waiting. The entry stays and becomes read-only — an ask that timed out is
    /// still what the agent asked, and a reader who comes back to it deserves to see so rather
    /// than a control that would send into nothing.
    fn ask_ended(
        &mut self,
        agent_id: AgentId,
        ask_id: AskId,
        why: AskClosed,
        cx: &mut Context<Self>,
    ) {
        if let Some(record) = self.ask_record_mut(agent_id, ask_id)
            && record.live()
        {
            record.stage = AskStage::Ended(why);
        }
        // The dialog stays up if it was up, now read-only: it is the same question, and closing it
        // under the user would take away the only thing on screen that says what happened. Its
        // two fields stop being drawn, so nothing has to be written back into them.
        cx.notify();
    }
}
