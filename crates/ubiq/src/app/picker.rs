use super::*;

/// What the preview says instead of drawing a picture it only has the front of.
///
/// Both read paths stop at `MAX_FILE_BYTES` and report `truncated`; a prefix of an image is not
/// an image, and handing one to the renderer draws an empty box that explains nothing.
const TOO_BIG: &str = "larger than the read ceiling";

impl AppState {
    /// Raise a picker over the sink's fixture tree, in the shape the page's controls describe.
    ///
    /// The previous answer goes with it: a readout left standing over a dialog that is being asked
    /// again reads as this dialog's answer, which it is not.
    pub fn raise_sink_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.picker.result = None;
        self.sink.picker.dismissed = false;
        let request = self.sink.picker.request();
        let view = self.sink.picker.view;
        self.open_file_picker(request, crate::state::sink::picker_tree(), view, window, cx);
    }

    // ── The file picker ─────────────────────────────────────────
    //
    // One dialog, raised by whichever screen needs a path and answered back to whoever asked. The
    // window holds it because exactly one may be up, and because the field above its rows is one
    // of the window's fields like every other.

    /// Raise a picker over `forest`, in the arrangement it should open in.
    ///
    /// The field is emptied first: a filter left over from the last dialog would hide rows the new
    /// one was raised to show.
    pub fn open_file_picker(
        &mut self,
        request: crate::state::file_picker::PickerRequest,
        forest: Vec<crate::state::file_picker::PickerNode>,
        view: PickerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A dialog takes the window's attention, so it closes whatever menu was down: two things
        // claiming an outside click is how a dismissal races itself.
        self.workbench.open_menu = None;
        self.picker_filter
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.file_picker = Some(FilePickerState::open(request, forest, view));
        // The field takes the keyboard, because the first thing a picker is for is typing a name
        // into it. Every other key the dialog answers to is bound against the field as well as
        // against the dialog, so the arrows still drive the rows — see `ui::file_picker`.
        let field = self.picker_filter.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    /// A key the dialog answered, or did not.
    ///
    /// Answering `false` is what hands the key back: `left` and `right` mean nothing in the flat
    /// list, and the caller propagates so the filter field gets its caret keys back.
    pub fn press_picker_key(
        &mut self,
        key: PickerKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = self.file_picker.as_mut() else {
            return false;
        };
        let pressed = picker.press(key);
        let at = picker.cursor_index();

        match pressed {
            Pressed::Ignored => false,
            Pressed::Moved => {
                // Follow the cursor: an arrow past the last drawn row has to bring it into view.
                if let Some(at) = at {
                    self.picker_scroll.scroll_to_item(at);
                }
                // A right-arrow or `enter` on a shut folder may just have expanded one a host
                // owns — the same folder a twisty click would have asked about.
                self.sync_host_browse(cx);
                cx.notify();
                true
            }
            Pressed::Commit => {
                self.commit_file_picker(window, cx);
                true
            }
            Pressed::Dismiss => {
                self.cancel_file_picker(window, cx);
                true
            }
        }
    }

    pub fn set_picker_view(&mut self, view: PickerView, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.set_view(view);
        }
        cx.notify();
    }

    /// Show or hide dotfiles. A toggle rather than a one-way filter — see
    /// `state::file_picker::FilePickerState::show_hidden`'s own doc on why one has to exist at
    /// all: a folder a user needs, like `.config`, must stay reachable.
    pub fn toggle_picker_hidden(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            let show = !picker.show_hidden();
            picker.set_show_hidden(show);
        }
        cx.notify();
    }

    pub fn toggle_picker_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.toggle_folder(&path);
        }
        // The twisty may just have opened a folder a host still owes a listing to.
        self.sync_host_browse(cx);
        cx.notify();
    }

    /// What a click on a row does, which the picker itself decides: a folder that cannot be picked
    /// opens, and a pick that was asked to be final closes the dialog on the spot.
    pub fn click_picker_row(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.as_mut() else {
            return;
        };
        if picker.click(&path) {
            self.commit_file_picker(window, cx);
            return;
        }
        // An unpickable click on a folder in tree view opens it — the same event a twisty click
        // asks the host about.
        self.sync_host_browse(cx);
        cx.notify();
    }

    /// Hand what was chosen to whoever asked for it, and take the dialog down.
    pub fn commit_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        let picked = picker.picked().to_vec();
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = Some(picked);
                self.sink.picker.dismissed = false;
            }
            // What a composer asked for arrives as tags under the token row, not as text in the
            // field: the prompt is still being written, and a path spelled into it cannot be
            // taken back out by clicking it. The sizes come off the picker's own nodes — the
            // interface reads no disk — which is why they are taken before it is dropped.
            PickerOwner::Composer { agent, slot } => {
                let sized = picker.picked_with_sizes();
                self.attach_files(agent, slot, &sized, window, cx);
            }
            // The host this folder came from lives in `host_browse`, not in the owner itself — see
            // `state::file_picker::PickerOwner::HostProject`'s own doc.
            // The folder a knowledge base source reads from. Written into the "Add source" form
            // rather than sent anywhere: the source is committed when that form is confirmed, as
            // one whole-list `SetKbSources`, not one folder at a time.
            // The one answer that leaves the window: a task's attachments are stored, so what was
            // picked becomes a `SetTaskField` rather than interface state.
            PickerOwner::TaskAttachment { task } => {
                self.add_task_attachments(task, picked, cx);
            }
            // The same answer, one step earlier: there is no task yet, so it lands on the draft.
            PickerOwner::NewMissionAttachment => {
                self.add_new_mission_attachments(picked, cx);
            }
            PickerOwner::KbFolder => {
                if let Some(path) = picked.into_iter().next() {
                    self.accept_kb_source_folder(path, window, cx);
                }
            }
            // A tool's starting folder, folded into the open editor rather than sent: the row
            // itself is only written back on Save, same as every other field the form holds.
            PickerOwner::ToolFolder => {
                if let Some(path) = picked.into_iter().next() {
                    self.accept_tool_starting_folder(path, cx);
                }
            }
            PickerOwner::HostProject => {
                if let (Some(path), Some(browse)) =
                    (picked.into_iter().next(), self.host_browse.take())
                {
                    self.adding = true;
                    self.bus.send_to(
                        browse.host,
                        Message::AddProject {
                            path,
                            name: None,
                            colour: None,
                            custom_colour: None,
                            temporary: !browse.persistent,
                            // A folder taken straight from the host browser never passed the
                            // creation panel, which is the one place the mode is chosen.
                            storage: StorageMode::default(),
                        },
                    );
                }
            }
        }
        self.close_host_browse();
        cx.notify();
    }

    /// Raise the picker over the open project's explorer tree, to be answered into one composer.
    ///
    /// Nothing happens with no project open: there is no tree to choose from, and an empty dialog
    /// would say the project has no files rather than that none have been listed.
    pub fn raise_composer_picker(
        &mut self,
        agent: AgentId,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forest = self
            .explorer(cx)
            .map(|explorer| crate::state::file_picker::forest_from_explorer(&explorer.root))
            .unwrap_or_default();
        if forest.is_empty() {
            return;
        }
        let request = crate::state::file_picker::PickerRequest::new(
            PickerOwner::Composer { agent, slot },
            "Attach files or folders to the prompt",
        )
        .kind(PickKind::Either);
        self.open_file_picker(request, forest, PickerView::Tree, window, cx);
    }

    /// Raise the picker over the project **and the knowledge base**, to be answered onto a task.
    ///
    /// **This is the one picker that offers two key spaces at once.** A task attachment is either
    /// a project-relative path or a `kb:{source}:{path}` address
    /// (`ubiq_proto::work::Attachment`), and both have to be reachable from one dialog or
    /// "attach from the knowledge base" becomes a second control that does nearly the same thing.
    /// So the explorer's forest gets one more top-level folder —
    /// `state::file_picker::forest_from_kb` — whose every row already carries the address the
    /// record stores. Nothing in the picker knows: a path is a path.
    ///
    /// Files only, unlike the composer's `+`: a folder here is a way to the documents under it,
    /// and picking one would let the knowledge base's own container rows — which name no
    /// document — reach a record.
    ///
    /// Nothing happens with neither a project tree nor a source: an empty dialog would say the
    /// project has no files rather than that none have been listed.
    pub fn raise_task_attachment_picker(
        &mut self,
        task: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut forest = self
            .explorer(cx)
            .map(|explorer| crate::state::file_picker::forest_from_explorer(&explorer.root))
            .unwrap_or_default();
        forest.extend(
            self.kb(cx)
                .and_then(|kb| crate::state::file_picker::forest_from_kb(&kb.sources)),
        );
        if forest.is_empty() {
            return;
        }
        let request = crate::state::file_picker::PickerRequest::new(
            PickerOwner::TaskAttachment { task },
            "Attach files or knowledge-base documents to this task",
        )
        .kind(PickKind::Files);
        self.open_file_picker(request, forest, PickerView::Tree, window, cx);
    }

    /// The task panel's paste: put what is on the board onto the task, or do nothing.
    ///
    /// The same two cases the composer's paste has — a copied file attaches by its path, a copied
    /// picture has none and is written into `.ubiq/pasted/` first — with one difference that
    /// follows from a task attachment being *stored*: the picture's attachment goes up when the
    /// write is **answered**, not when it is sent. The composer bets the other way because its
    /// chip sits in a draft the user is still writing and a round trip would land after the turn;
    /// a record that outlives the window has no such hurry, and a dangling path written into
    /// `tasks.toml` would outlive the mistake.
    pub fn paste_into_task(&mut self, task: TaskId, cx: &mut Context<Self>) {
        let Some(found) = clipboard::clipboard_attachment(cx) else {
            return;
        };
        match found {
            clipboard::PastedAttachment::Path(path) => {
                let named = match self.project_relative(&path, cx) {
                    Some((holder, rel)) if Some(holder) == self.project(cx) => rel,
                    _ => path.to_string_lossy().into_owned(),
                };
                self.add_task_attachments(task, vec![named], cx);
            }
            clipboard::PastedAttachment::Image { bytes, format } => {
                self.paste_image_into_task(task, bytes, format, cx)
            }
        }
        cx.notify();
    }

    /// Raise the same picker for the mission being written, which has no task to hang the answer
    /// on yet — so the answer lands on the draft, and `settle_new_mission` writes it onto the
    /// anchor as a `SetTaskField` once there is one.
    pub fn raise_new_mission_attachment_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut forest = self
            .explorer(cx)
            .map(|explorer| crate::state::file_picker::forest_from_explorer(&explorer.root))
            .unwrap_or_default();
        forest.extend(
            self.kb(cx)
                .and_then(|kb| crate::state::file_picker::forest_from_kb(&kb.sources)),
        );
        if forest.is_empty() {
            return;
        }
        let request = crate::state::file_picker::PickerRequest::new(
            PickerOwner::NewMissionAttachment,
            "Attach files or knowledge-base documents to this mission",
        )
        .kind(PickKind::Files);
        self.open_file_picker(request, forest, PickerView::Tree, window, cx);
    }

    /// The new-mission dialog's paste, [`Self::paste_into_task`]'s path exactly: a copied **file**
    /// attaches by its path, a copied **picture** has none and is written into `.ubiq/pasted/`
    /// first, joining the chip row only when that write is answered.
    ///
    /// Pasted **text** never reaches here — it is typed into whichever field holds the keyboard by
    /// the component library's own paste, which is what the requirements editor wants.
    pub fn paste_into_new_mission(&mut self, cx: &mut Context<Self>) {
        let Some(found) = clipboard::clipboard_attachment(cx) else {
            return;
        };
        match found {
            clipboard::PastedAttachment::Path(path) => {
                let named = match self.project_relative(&path, cx) {
                    Some((holder, rel)) if Some(holder) == self.project(cx) => rel,
                    _ => path.to_string_lossy().into_owned(),
                };
                self.add_new_mission_attachments(vec![named], cx);
            }
            clipboard::PastedAttachment::Image { bytes, format } => {
                // The draft's picture waits for the write the way a task's does, and for the same
                // reason: what is on the chip row here becomes a stored attachment on the anchor.
                if let Some(project) = self.project(cx) {
                    let (rel_path, _) = self.write_pasted_image(project, bytes, format);
                    self.pasted_writes.push(PastedWrite {
                        project,
                        rel_path,
                        into: PastedInto::NewMission,
                    });
                }
            }
        }
        cx.notify();
    }

    /// Hang what was picked on the conversation as attachments, one tag each.
    ///
    /// Nothing is written into the field: the paths become `@path` mentions only when the prompt
    /// is actually sent, which is `Conversation::compose_prompt`'s job. A path already attached
    /// is not attached twice. The keyboard goes back to the composer either way — the dialog took
    /// it, and the user was in the middle of typing.
    fn attach_files(
        &mut self,
        agent: AgentId,
        slot: usize,
        picked: &[(String, Option<u64>)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent)
        {
            for (path, size) in picked {
                conversation.attach(path.clone(), *size);
            }
        }
        if let Some(input) = self.column_inputs.get(slot).cloned() {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Take one attachment back off — a tag's own dismiss control.
    pub fn detach_file(&mut self, agent: AgentId, attachment: u64, cx: &mut Context<Self>) {
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent)
        {
            conversation.detach(attachment);
        }
        cx.notify();
    }

    // ── Pasting a file into a composer ──────────────────────────
    //
    // The `+` picker is one way to attach a file; the platform's own clipboard is the other, and
    // this is it. What comes out the far end is identical — a `Conversation::attach`, one tag,
    // one `@path` in the prompt — so the chip row, the dedupe, the size warning and
    // `compose_prompt` all work here without knowing a paste happened.

    /// The composer's paste: attach what is on the board, or hand the keystroke back.
    ///
    /// **A board carrying only text is not this gesture.** `cx.propagate()` returns the chord to
    /// the field, where `InputState::paste` types the text in exactly as it always has — see
    /// `install_key_bindings` for why both bindings exist and which wins.
    ///
    /// A copied *file* is attached by its path and nothing is written. A copied *picture* has no
    /// path, so one is made for it: see [`Self::attach_pasted_image`].
    pub fn paste_into_composer(
        &mut self,
        agent: AgentId,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(found) = clipboard::clipboard_attachment(cx) else {
            cx.propagate();
            return;
        };
        match found {
            clipboard::PastedAttachment::Path(path) => self.attach_pasted_path(agent, &path, cx),
            clipboard::PastedAttachment::Image { bytes, format } => {
                self.attach_pasted_image(agent, bytes, format, cx)
            }
        }
        // The user was in the middle of writing, and the chord was taken off the field to do
        // this; the keyboard goes straight back, exactly as the picker's answer returns it.
        if let Some(input) = self.column_inputs.get(slot).cloned() {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// Attach a file that already exists, by the path the platform named.
    ///
    /// **Project-relative where it can be, absolute where it cannot.** A file inside the
    /// conversation's own project attaches under exactly the path the `+` picker would have given
    /// it — one tag, not two, and the `@path` the harness reads. Anything else keeps its absolute
    /// path: the harness is running somewhere, and an absolute path is the only thing that still
    /// names the file from there.
    ///
    /// **Relative only to the agent's *own* project.** `project_relative` answers which project a
    /// path fell inside, and that answer is the point: a window holding projects A and B, with an
    /// agent in A, must not turn a file copied out of B into `@src/lib.rs` — the harness runs in A
    /// and would resolve it against a different file that happens to exist, silently. So the
    /// relative form is taken when the ids match and the absolute path is kept otherwise, which is
    /// the same use `deliver_paths` makes of the id one function over.
    ///
    /// No size is reported. Nothing has read the folder, and a size the interface guessed at
    /// would be a warning colour standing for nothing — `Attachment::size` is `None` for exactly
    /// this case.
    fn attach_pasted_path(&mut self, agent: AgentId, path: &Path, cx: &mut Context<Self>) {
        let Some(id) = self.project_of_agent(agent, cx) else {
            return;
        };
        let named = match self.project_relative(path, cx) {
            Some((holder, rel)) if holder == id => rel,
            _ => path.to_string_lossy().into_owned(),
        };
        if let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent)
        {
            conversation.attach(named, None);
        }
    }

    /// Attach a picture that exists nowhere, by writing it into the project first.
    ///
    /// **This writes a file into the user's project**, under `clipboard::PASTED_DIR`, and that is
    /// the decision rather than a side effect of one: a turn carries attachments as `@path`
    /// mentions (`G171`), so a screenshot that is only on the pasteboard cannot be attached at
    /// all until something gives it a path. `Message::WriteProjectFile` is how it gets one —
    /// nothing new crosses the bus.
    ///
    /// **The tag goes up on the send, not on the answer**, which is the bet `save_untitled_as`
    /// makes for the same reason: the path is already decided, the user is mid-sentence, and a
    /// chip that appeared a round trip later would arrive after the turn it belongs to. **The bet
    /// is paid for by tracking the write**: `file_failed` only reacts for a path the editor has
    /// open and `.ubiq/pasted/…` never is, so a refused write would otherwise be a log line and a
    /// dangling `@path`. The row goes on `pasted_writes`, and
    /// [`Self::pasted_write_failed`] takes the chip back off and says so. The size is the board's
    /// own byte count, which is exact — this is the one attachment nothing had to guess.
    ///
    /// **The format may not be the board's.** A screenshot arrives as TIFF on macOS and as a DIB
    /// on Windows, and the only reason to write it at all is for a harness to open it — see
    /// `clipboard::pasted_image_bytes`.
    ///
    /// With no project open there is nowhere to write and nothing happens: a picture cannot be
    /// attached to a conversation that has no folder behind it.
    fn attach_pasted_image(
        &mut self,
        agent: AgentId,
        bytes: Vec<u8>,
        format: gpui::ImageFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project_of_agent(agent, cx) else {
            return;
        };
        let (rel_path, size) = self.write_pasted_image(project, bytes, format);
        let attachment = self
            .projects
            .get_mut(&project)
            .and_then(|open| open.conversations.get_mut(&agent))
            .and_then(|conversation| conversation.attach(rel_path.clone(), Some(size)));
        if let Some(attachment) = attachment {
            self.pasted_writes.push(PastedWrite {
                project,
                rel_path,
                into: PastedInto::Composer { agent, attachment },
            });
        }
    }

    /// The same picture, written for a task instead of a turn.
    ///
    /// **The attachment waits for the answer**, which is the one place this parts company with
    /// [`Self::attach_pasted_image`] — see [`Self::paste_into_task`] for why a stored record does
    /// not take the composer's bet. So nothing is sent to the board here: the row on
    /// `pasted_writes` is the whole of it, and [`Self::pasted_write_settled`] sends the
    /// `SetTaskField` once the file is really there.
    fn paste_image_into_task(
        &mut self,
        task: TaskId,
        bytes: Vec<u8>,
        format: gpui::ImageFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let (rel_path, _) = self.write_pasted_image(project, bytes, format);
        self.pasted_writes.push(PastedWrite {
            project,
            rel_path,
            into: PastedInto::Task(task),
        });
    }

    /// Put a pasted picture in the project under `clipboard::PASTED_DIR`, and say what it is
    /// called and how big it is. The half both paste destinations share.
    ///
    /// **The format may not be the board's.** A screenshot arrives as TIFF on macOS and as a DIB
    /// on Windows, and the only reason to write it at all is for something to open it later — see
    /// `clipboard::pasted_image_bytes`.
    fn write_pasted_image(
        &mut self,
        project: ProjectId,
        bytes: Vec<u8>,
        format: gpui::ImageFormat,
    ) -> (String, u64) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default();
        let (format, bytes) = clipboard::pasted_image_bytes(format, bytes);
        let rel_path = clipboard::pasted_image_path(format, stamp);
        let size = bytes.len() as u64;
        self.ignore_pasted_dir(project);
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path: rel_path.clone(),
            bytes,
            expected: None,
            overwrite: false,
        });
        (rel_path, size)
    }

    /// Make the pasted folder ignore itself, the first time this window writes into one.
    ///
    /// **A pasted screenshot is not a file the user chose to keep**, unlike a project-stored
    /// knowledge base under `.ubiq/local/kb`, so a folder of untracked binaries turning up in `git
    /// status` is this gesture littering in the user's repository. The answer is one
    /// `.gitignore` holding `*` inside the folder itself — never a line appended to the user's
    /// own ignore file, which is a tracked file nobody asked to change and would then have to be
    /// merged, deduplicated and unwound. It goes through `WriteProjectFile` like the picture
    /// beside it, so the interface still names no filesystem path of its own.
    ///
    /// `overwrite: false` means an ignore file already there is left exactly as it is, and the
    /// refusal that comes back is not one of [`Self::pasted_write_failed`]'s rows: nothing was
    /// attached for it, and the folder is already ignored either way.
    fn ignore_pasted_dir(&mut self, project: ProjectId) {
        if !self.pasted_ignored.insert(project) {
            return;
        }
        let (rel_path, body) = clipboard::PASTED_IGNORE;
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path: rel_path.to_string(),
            bytes: body.as_bytes().to_vec(),
            expected: None,
            overwrite: false,
        });
    }

    /// A pasted picture's write came back, one way or the other.
    ///
    /// Success only forgets the row — the chip was right all along. Failure takes the attachment
    /// back off and raises a notification, because there is no other surface for it: the chip is
    /// in a composer that may not be on screen, the path is in no editor tab, and an `@path` sent
    /// for a file that was never written is a turn the harness answers with a read error.
    /// Answers whether this was a pasted write at all, so the ordinary file path keeps its own.
    pub(super) fn pasted_write_settled(
        &mut self,
        project: ProjectId,
        rel_path: &str,
        failure: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ix) = self
            .pasted_writes
            .iter()
            .position(|write| write.project == project && write.rel_path == rel_path)
        else {
            return false;
        };
        let write = self.pasted_writes.remove(ix);
        let Some(reason) = failure else {
            // A task's picture waits for this answer rather than anticipating it, so the write
            // landing is what puts it on the record. A composer's chip was already up.
            match write.into {
                PastedInto::Task(task) => {
                    self.add_task_attachments(task, vec![rel_path.to_string()], cx);
                }
                PastedInto::NewMission => {
                    self.add_new_mission_attachments(vec![rel_path.to_string()], cx);
                }
                PastedInto::Composer { .. } => {}
            }
            return true;
        };
        // The composer's optimistic chip is the only thing there is to take back off: a task's
        // was never put up.
        if let PastedInto::Composer { agent, attachment } = write.into
            && let Some(open) = self.projects.get_mut(&project)
            && let Some(conversation) = open.conversations.get_mut(&agent)
        {
            conversation.detach(attachment);
        }
        self.raise_notification(
            NotificationRequest::warning(
                Family::Files,
                format!(
                    "The pasted picture could not be saved \u{2014} {reason}. It was not attached."
                ),
            )
            .with_category("paste"),
        );
        cx.notify();
        true
    }

    // ── The preview behind a sent chip ──────────────────────────

    /// Show what is behind one chip on a sent turn.
    ///
    /// **A panel, not a tab.** The question a chip answers is *which file was that* — a glance,
    /// not a reading — and opening the editor for it would take the reader off the transcript
    /// they are following. The editor is still one click further on, through the `Open` the panel
    /// carries.
    ///
    /// The bytes are fetched, never held: a project-relative path asks the host the ordinary way
    /// (`Message::ReadProjectFile`, answered into [`Self::fill_attachment_preview`]), and an
    /// absolute one is read here, because there is no project to answer for a path outside every
    /// project — the same split `open_guest_file` makes.
    pub fn open_attachment_preview(
        &mut self,
        agent: AgentId,
        attachment: u64,
        path: String,
        size: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        // `open_menu` clears whatever the last one left behind, so the field is empty here and the
        // invariant `WorkbenchState::attachment_preview` states — `Some` exactly while this menu
        // is open — holds through an open as well as a close.
        self.open_menu(MenuId::AttachmentPreview, cx);
        let mut preview = AttachmentPreview {
            agent,
            attachment,
            project: None,
            path: path.clone(),
            size,
            bytes: None,
            failed: None,
        };

        let absolute = Path::new(&path);
        if absolute.is_absolute() {
            match read_guest_file(absolute) {
                Ok(contents) => match contents.truncated {
                    true => preview.failed = Some(TOO_BIG.to_string()),
                    false => preview.bytes = Some(contents.bytes),
                },
                Err(reason) => preview.failed = Some(reason),
            }
        } else if let Some(project) = self.project_of_agent(agent, cx) {
            preview.project = Some(project);
            self.bus.send(Message::ReadProjectFile {
                project_id: project,
                rel_path: path,
                max_bytes: Some(MAX_FILE_BYTES),
            });
        } else {
            preview.failed = Some("no project holds this file".to_string());
        }

        self.workbench.attachment_preview = Some(preview);
        cx.notify();
    }

    /// Hand a read the preview panel is waiting on to it, if this is the one it asked for.
    ///
    /// Called from the `ProjectFileContents` arm beside the editor's own queue rather than
    /// instead of it: a file can be open in a tab and previewed at the same moment, and both
    /// readers want the same bytes.
    ///
    /// **Matched on the project as well as the path**, because a path is not unique across the
    /// projects one window holds — see `AttachmentPreview::project`.
    ///
    /// **A truncated read is not a picture.** The host answers `MAX_FILE_BYTES` of a larger file
    /// and says so, and a prefix of a PNG handed to the image renderer draws an empty box with no
    /// explanation. The chip for such a file is already coloured as huge, so this is the likeliest
    /// file anybody clicks — it gets the note instead.
    pub(super) fn fill_attachment_preview(
        &mut self,
        project: ProjectId,
        rel_path: &str,
        contents: &FileContents,
    ) {
        if let Some(preview) = &mut self.workbench.attachment_preview
            && preview.project == Some(project)
            && preview.path == rel_path
            && preview.bytes.is_none()
            && preview.failed.is_none()
        {
            match contents.truncated {
                true => preview.failed = Some(TOO_BIG.to_string()),
                false => preview.bytes = Some(contents.bytes.clone()),
            }
        }
    }

    /// Tell the preview panel the read it is waiting on will never arrive.
    ///
    /// Without this the popover says `Reading…` for as long as it is up: the failure goes to
    /// `file_failed`, which only touches paths the editor has open. The panel already draws a
    /// "cannot preview" state for a guest read that failed — this is the same state, reached from
    /// the other half of the split. Answers whether the panel wanted this path.
    pub(super) fn fail_attachment_preview(
        &mut self,
        project: ProjectId,
        rel_path: &str,
        reason: &str,
    ) -> bool {
        if let Some(preview) = &mut self.workbench.attachment_preview
            && preview.project == Some(project)
            && preview.path == rel_path
            && preview.bytes.is_none()
        {
            preview.failed = Some(reason.to_string());
            return true;
        }
        false
    }

    /// Take the preview down — its own outside click, and the `Open` that hands the file to the
    /// editor instead.
    pub fn close_attachment_preview(&mut self, cx: &mut Context<Self>) {
        self.close_menu(cx);
    }

    /// Open the previewed file properly: in the editor for a path inside a project, as a guest
    /// tab for one outside every project. Exactly what clicking an unsent tag does, which is why
    /// the panel offers it rather than inventing a second way in.
    pub fn open_attachment(&mut self, path: String, cx: &mut Context<Self>) {
        self.close_attachment_preview(cx);
        let absolute = Path::new(&path);
        if absolute.is_absolute() {
            self.open_guest_file(absolute, cx);
        } else {
            self.select_file(path, cx);
        }
    }

    /// Take the dialog down with nothing chosen. Dismissed is not the same answer as an empty one,
    /// so whoever asked is told which it was.
    pub fn cancel_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = None;
                self.sink.picker.dismissed = true;
            }
            // Nothing to write back: a composer that was not answered keeps what it already had.
            // The keyboard goes back to it, which is where it was before the dialog took it.
            PickerOwner::Composer { slot, .. } => {
                if let Some(input) = self.column_inputs.get(slot).cloned() {
                    input.update(cx, |state, cx| state.focus(window, cx));
                }
            }
            // Nothing to write back here either — closing the dialog with nothing chosen leaves
            // no project to open, and no folder for the form behind it.
            // Nor here: a task keeps exactly the attachments it already carried.
            PickerOwner::HostProject
            | PickerOwner::KbFolder
            | PickerOwner::ToolFolder
            | PickerOwner::TaskAttachment { .. }
            | PickerOwner::NewMissionAttachment => {}
        }
        self.close_host_browse();
        cx.notify();
    }

    pub fn start_picker_resize(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.start_drag(at);
        }
        cx.notify();
    }

    /// Follow a corner drag. The window is what the dialog has to fit inside, so its size comes in
    /// with the pointer.
    pub fn drag_picker_resize(&mut self, at: (f32, f32), window: &Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        if let Some(picker) = self.file_picker.as_mut()
            && picker.drag_to(at, (f32::from(viewport.width), f32::from(viewport.height)))
        {
            cx.notify();
        }
    }

    pub fn end_picker_resize(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut()
            && picker.is_resizing()
        {
            picker.end_drag();
            cx.notify();
        }
    }

    // ── The log console ────────────────────────────────────────

    pub fn pick_log_subsystem(&mut self, index: usize, cx: &mut Context<Self>) {
        self.logs.pick_subsystem(index);
        self.close_menu(cx);
    }

    pub fn pick_log_level(&mut self, index: usize, cx: &mut Context<Self>) {
        self.logs.pick_level(index);
        self.close_menu(cx);
    }

    pub fn toggle_log_follow(&mut self, cx: &mut Context<Self>) {
        self.logs.follow = !self.logs.follow;
        cx.notify();
    }

    /// Empty the sink. The ring is the whole process's, so this clears every window's console.
    pub fn clear_logs(&mut self, cx: &mut Context<Self>) {
        ubiq_proto::log::logs().clear();
        cx.notify();
    }

    // ── Projects ────────────────────────────────────────────────────
}
