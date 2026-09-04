use super::*;

impl AppState {
    /// Point the screen at a session or at one agent. Both are selections, and everything else on
    /// the screen — the graph's session, the inspector, the tasks drawer — is a function of this
    /// one field.
    pub fn select_in_graph(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.selection = Some(selection);
        }
        cx.notify();
    }

    /// Draw one session's agents, or every session's. It does not move the selection: what the
    /// inspector and the drawer report on is a separate question from what the canvas draws.
    pub fn show_graph_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.show_session(session);
        }
        cx.notify();
    }

    /// Put every filter on the orchestration screen back. The one control for "show me all of it".
    pub fn clear_graph_filters(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.clear_filters();
        }
        cx.notify();
    }

    pub fn toggle_agent_bucket(&mut self, bucket: Bucket, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.toggle_bucket(bucket);
        }
        cx.notify();
    }

    pub fn zoom_graph(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.zoom_by(delta);
        }
        cx.notify();
    }

    pub fn reset_graph_zoom(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.zoom = 1.0;
        }
        cx.notify();
    }

    /// Throw the arrangement away and lay the graph out again from what the agents and tasks say.
    ///
    /// Every hand-placed card is lost, which is the point: it is the way back from a canvas the
    /// user has pulled apart, and there is nothing else on the screen that undoes a drag. The full
    /// `relayout` rather than `place_new`, which is the one that leaves placed cards alone.
    pub fn tidy_graph(&mut self, cx: &mut Context<Self>) {
        if let Some((graph, work)) = self.graph_over_work(cx) {
            graph.relayout(work);
        }
        cx.notify();
    }

    pub fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.show_inspector = !graph.show_inspector;
        }
        cx.notify();
    }

    pub fn toggle_tasks_drawer(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.tasks_open = !graph.tasks_open;
        }
        cx.notify();
    }

    /// Select one agent and put the inspector on its thread — what the `chat` affordance on a card
    /// does, and the one place the screen changes two things at once, because a card asking for a
    /// conversation with the panel shut has asked for nothing.
    pub fn open_agent_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.selection = Some(Selection::Agent(agent));
            graph.tab = InspectorTab::Chat;
            graph.show_inspector = true;
        }
        cx.notify();
    }

    pub fn select_inspector_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.tab = if index == 0 {
                InspectorTab::Chat
            } else {
                InspectorTab::Tasks
            };
        }
        cx.notify();
    }

    /// Pick a card or a container up.
    ///
    /// A card selects itself on the way up, because what is being moved is what the user is
    /// looking at, and a drag that left the inspector on something else would be reporting on the
    /// wrong agent. A container does not: dragging a box to make room is not a claim about what
    /// the user wants to read.
    pub fn start_graph_carry(&mut self, held: Held, grab: (f32, f32), cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.start_carry(held, grab);
            if let Held::Agent(agent) = held {
                graph.selection = Some(Selection::Agent(agent));
            }
        }
        cx.notify();
    }

    /// Move whatever is being carried, and lay a grain of sand where the pointer passed.
    ///
    /// The trail is skipped when the system asks for reduced motion — it is the only motion on
    /// this screen, and what is held still follows the pointer without it.
    pub fn move_graph_carry(
        &mut self,
        at: (f32, f32),
        pointer: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let trail = (!cx.reduce_motion()).then_some(pointer);
        if let Some((graph, work)) = self.graph_over_work(cx) {
            graph.carry_to(work, at, trail, std::time::Instant::now());
        }
        cx.notify();
    }

    /// Put it down, and ask for the card to be moved into whatever container it landed in.
    ///
    /// **Position is the interface's own fact, membership is the host's.** The drop writes the
    /// card's new offset and nothing else; which task it serves is written down, so the answer is
    /// an `AssignAgent` and the card only changes hands when the host says it has.
    pub fn end_graph_carry(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let landed = self
            .graph_over_work(cx)
            .and_then(|(graph, work)| graph.end_carry(work));
        if let Some((agent_id, task_id)) = landed {
            self.bus.send(Message::AssignAgent {
                project_id,
                agent_id,
                task_id: Some(task_id),
            });
        }
        cx.notify();
    }

    /// What the composer sends, to the selected agent.
    ///
    /// Nothing is appended here. The line lands in the thread when the host answers with the agent
    /// carrying it — an interface that writes its own message into a transcript is inventing half
    /// of a conversation.
    pub fn send_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(graph) = self.graph(cx) else {
            return;
        };
        let text = graph.draft.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(Selection::Agent(agent_id)) = graph.selection else {
            return;
        };
        self.bus.send(Message::SendToAgent {
            project_id,
            agent_id,
            text,
        });
        if let Some(graph) = self.graph_mut(cx) {
            graph.draft.clear();
        }
        let input = self.agent_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Age the drag trail by one frame, and answer whether it still owes the window another.
    ///
    /// A drag that ended outside the graph — on the inspector, or off the window — never reaches
    /// the canvas's drop handler, so a carry with no live drag behind it is put down here. That is
    /// what stops a card sticking to the pointer after the button came up somewhere else.
    pub(super) fn settle_graph(&mut self, cx: &mut Context<Self>) {
        let stranded = self
            .graph(cx)
            .is_some_and(|graph| graph.carry.is_some() && !cx.has_active_drag());
        if stranded {
            self.end_graph_carry(cx);
        }
        if let Some(graph) = self.graph_mut(cx) {
            graph.settle_sand(std::time::Instant::now());
        }
    }

    // ── The tasks board ─────────────────────────────────────────────

    // ── the task panel's own edits ──────────────────────────────────
    // Every one of these asks and waits. The panel goes on reporting the task the host last
    // confirmed, so a refusal leaves nothing to unwind — which is the same reason a pane is drawn
    // when the coordinator answers rather than when the interface asked.
}
