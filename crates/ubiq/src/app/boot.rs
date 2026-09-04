use super::*;

impl AppState {
    /// Build a window pointed at one project, named by `label`. A second window is the same view
    /// registered under its own letter — see [`open_project_window`], which is the only caller.
    ///
    /// The project is optional: on a first run the catalogue is empty, and a window with nothing
    /// open still has "Add a project…" to offer.
    pub fn for_project(
        project: Option<ProjectId>,
        label: char,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The window takes the project from whatever window held it, so opening one somewhere can
        // leave another showing nothing. That window stays: Ubiq never closes one on the user's
        // behalf.
        let window_id = window.window_handle().window_id();
        cx.global_mut::<WindowRegistry>()
            .register(window_id, label, project);
        // A handle back to this window's own state, so a path arriving from outside it — the
        // command line, a Finder open, a dock-icon drop — has somewhere to be delivered.
        OpenWindows::register(window_id, cx.weak_entity(), cx);

        let chat_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Reply, or describe the next change\u{2026}")
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });

        let agent_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe a task for this agent\u{2026}")
                .auto_grow(1, 6)
                .submit_on_enter(true)
        });

        // The whole pool, before the first frame. Each one's placeholder names the agent its
        // column is showing and is set when the column opens or changes tab; until then it says
        // what the field is for.
        let column_inputs: Vec<Entity<TextareaState>> = (0..COMPOSER_SLOTS)
            .map(|_| {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("Ask me\u{2026}")
                        .auto_grow(1, 5)
                        .submit_on_enter(true)
                })
            })
            .collect();

        let file_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Go to file\u{2026}"));

        let git_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search message, author or SHA\u{2026}")
        });
        let git_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Commit message \u{2014} subject, blank line, body")
                .auto_grow(3, 8)
        });

        let picker_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name or path\u{2026}"));

        let task_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter tasks\u{2026}"));

        let task_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Task title"));

        // The one field in the window that must not submit on Enter: a newline is a paragraph
        // break in Markdown, so Save is a button rather than a key.
        let task_description_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe the task in Markdown\u{2026}")
                .auto_grow(3, 14)
        });

        // One field for renaming whichever sub-task is open, because only one ever is.
        let step_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Rename this sub-task"));

        let new_step_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a sub-task\u{2026}"));

        let command_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search files, or run a command\u{2026}")
        });

        let project_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a project\u{2026}"));

        let search_query =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search in project\u{2026}"));

        // Seeded from the host's answer rather than here — see `sync_search_settings_fields`.
        let search_excludes_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("node_modules, target, .git\u{2026}")
        });
        let search_fallbacks_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("grep, ag\u{2026}"));

        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        let project_form_about = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Two lines about this codebase\u{2026}")
                .auto_grow(3, 6)
        });
        let project_form_hex = cx.new(|cx| InputState::new(window, cx).placeholder("#RRGGBB"));

        // The kitchen sink's fixtures become buffers here, where there is a window to build one
        // with. They are constants, so this is the whole of their lifecycle: nothing arrives late,
        // nothing is saved, and a change event would have nothing to compare against.
        let sink_buffers: HashMap<&'static str, Entity<EditorState>> = crate::state::sink::docs()
            .iter()
            .map(|doc| {
                let buffer = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .language(ui::editor::highlighter_language(doc.language()))
                        .line_number(true)
                        .folding(true)
                        .show_whitespaces(false)
                        .tab_size(TabSize {
                            tab_size: 2,
                            ..Default::default()
                        })
                        .default_value(doc.source)
                });
                (doc.key, buffer)
            })
            .collect();

        let sink_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("a field, with nothing behind it")
                .default_value("crates/ubiq/src/theme.rs")
        });

        let sink_textarea = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("a textarea, with nothing behind it\u{2026}")
                .auto_grow(2, 6)
        });

        let sink_modal_input = cx.new(|cx| InputState::new(window, cx).placeholder("Session name"));

        let login_account_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("work, personal\u{2026}"));

        // Placeholder is set fresh whenever the naming prompt opens (the picked harness's own
        // label), so this construction-time one is only ever seen if the prompt somehow paints
        // before its own open path runs.
        let new_agent_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name this conversation\u{2026}"));

        let sink_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search settings\u{2026}"));
        let sink_harness_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Display name")
                .default_value("Claude Code — work")
        });
        let sink_harness_exec = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/absolute/path")
                .default_value("/opt/homebrew/bin/claude")
        });
        let sink_harness_prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("House rules, not a task\u{2026}")
                .auto_grow(3, 8)
                .default_value(
                    "You work in a Tauri + React codebase. Prefer small diffs, keep the existing \
                     file conventions, and never touch src-tauri without saying so first.",
                )
        });
        let sink_harness_env = cx.new(|cx| InputState::new(window, cx).placeholder("KEY=value"));
        let sink_project_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Project name")
                .default_value(crate::state::sink::PROJECT_NAME)
        });
        let sink_project_about = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Two lines about this codebase\u{2026}")
                .auto_grow(3, 6)
                .default_value(crate::state::sink::PROJECT_ABOUT)
        });
        let sink_project_hex = {
            let swatch = theme::project_colour(crate::state::sink::PROJECT_COLOUR);
            let hex = crate::state::sink::hex_string(crate::state::sink::rgb_from_channels(
                swatch.r, swatch.g, swatch.b,
            ));
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("#RRGGBB")
                    .default_value(hex)
            })
        };

        // Every panel reads this window's state, and `cx.entity()` is that handle before the state
        // it names exists — the slot is reserved for the duration of the constructor.
        let app = cx.weak_entity();

        // The window's arrangement. It is built before anything is put in it, because a panel is
        // an entity that has to be handed somewhere the moment it exists.
        let new_pane = crate::ui::dock::skin::NewPane {
            available: {
                let app = app.clone();
                Rc::new(move |cx| {
                    app.upgrade()
                        .is_some_and(|this| this.read(cx).project(cx).is_some())
                })
            },
            run: {
                let app = app.clone();
                Rc::new(move |_window, cx| {
                    if let Some(this) = app.upgrade() {
                        this.update(cx, |this, cx| this.spawn_pane(None, Vec::new(), cx));
                    }
                })
            },
            menu: {
                let app = app.clone();
                Rc::new(move |x, y, _window, cx| {
                    if let Some(this) = app.upgrade() {
                        this.update(cx, |this, cx| this.open_new_pane_menu((x, y), cx));
                    }
                })
            },
            region: {
                let app = app.clone();
                Rc::new(move |node, cx| {
                    app.upgrade()
                        .is_some_and(|this| this.read(cx).is_pane_region(node, cx))
                })
            },
        };
        let dock = cx.new(|cx| {
            let menu_app = app.clone();
            let file_tab_menu: crate::ui::dock::skin::FileTabMenuRun =
                Rc::new(move |key, x, y, _window, cx| {
                    if let Some(this) = menu_app.upgrade() {
                        this.update(cx, |this, cx| this.open_file_tab_menu(key, (x, y), cx));
                    }
                });
            let promote_app = app.clone();
            let file_tab_promote: crate::ui::dock::skin::FileTabPromoteRun =
                Rc::new(move |key, _window, cx| {
                    if let Some(this) = promote_app.upgrade() {
                        let path = crate::state::editor::from_tab_key(key).0;
                        this.update(cx, |this, cx| this.select_file(path, cx));
                    }
                });
            DockArea::new("ubiq-workbench", Some(dock::LAYOUT_VERSION), window, cx).with_renderer(
                crate::ui::dock::skin::Skin::new()
                    .with_new_pane(new_pane)
                    .with_file_tab_menu(file_tab_menu)
                    .with_file_tab_promote(file_tab_promote),
            )
        });
        let mut panels: HashMap<PanelKind, Entity<WorkbenchPanel>> = HashMap::new();
        {
            // The default arrangement asks for no terminal, so nothing here is ever refused; the
            // answer is an `Option` because a restored arrangement's can be — see `settle_layout`.
            let mut build = |kind: PanelKind, cx: &mut App| {
                Some(
                    panels
                        .entry(kind.clone())
                        .or_insert_with(|| WorkbenchPanel::new(kind, app.clone(), cx))
                        .clone(),
                )
            };
            dock::default_layout(&dock, &mut build, window, cx);
        }

        let mut subscriptions = Vec::new();

        // The dock is where the arrangement lives, so it is the dock that says it changed. Every
        // edit fires this — a drag, a split, a divider let go of — and the host debounces the
        // write, so it may fire as freely as it likes. The placement policy is applied first: a
        // panel that landed somewhere its kind forbids is put back before the layout is written
        // down, so what is remembered is what is on screen.
        subscriptions.push(cx.subscribe_in(
            &dock,
            window,
            |this, _, event: &DockEvent, window, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.enforce_placement(window, cx);
                    this.hide_emptied_regions(window, cx);
                    this.remember_view(cx);
                    cx.notify();
                }
            },
        ));

        // Every window draws from the same registry, so a project moved in one window redraws the
        // picker in all of them. Another window taking a project is also a change this window has
        // to act on, and it learns about it the same way it learns about its own.
        subscriptions.push(cx.observe_global::<WindowRegistry>(|this, cx| this.sync_projects(cx)));

        subscriptions.push(cx.subscribe_in(
            &chat_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.chat.draft = input.read(cx).value().to_string();
                    cx.notify();
                }
                // The textarea submits on a bare Enter; Shift+Enter still inserts a newline and
                // must not send.
                InputEvent::PressEnter { shift: false, .. } => this.send_chat(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &agent_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let draft = input.read(cx).value().to_string();
                    // A window with no project has no composer on screen to have typed into, so
                    // there is nothing to mirror it onto.
                    if let Some(graph) = this.graph_mut(cx) {
                        graph.draft = draft;
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { shift: false, .. } => this.send_to_agent(window, cx),
                _ => {}
            },
        ));

        // One subscription per column composer, each carrying the slot it belongs to. What is
        // typed lands in that slot of the project's drafts; a bare Enter steers the column.
        for (slot, input) in column_inputs.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                move |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let draft = input.read(cx).value().to_string();
                        if let Some(agents) = this.agents_mut(cx) {
                            agents.set_draft(slot, draft);
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { shift: false, .. } => {
                        this.steer_column(slot, window, cx)
                    }
                    _ => {}
                },
            ));
        }

        subscriptions.push(cx.subscribe_in(
            &file_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let draft = input.read(cx).value().to_string();
                    this.schedule_explorer_filter(draft, cx);
                }
            },
        ));

        // The Git screen's two fields mirror into the project's own view. Neither commits
        // anything: the search filters as it is typed, and the message is a draft nothing writes.
        subscriptions.push(cx.subscribe_in(
            &git_search,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let search = input.read(cx).value().to_string();
                    if let Some(git) = this.git_view_mut(cx) {
                        git.search = search;
                    }
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &git_message,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let message = input.read(cx).value().to_string();
                    if let Some(git) = this.git_view_mut(cx) {
                        git.message = message;
                    }
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &picker_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    if let Some(picker) = this.file_picker.as_mut() {
                        picker.set_filter(filter);
                    }
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &task_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.filter = filter;
                    }
                    cx.notify();
                }
            },
        ));

        // The panel's four fields mirror into the project's own form and commit on Enter or on the
        // control beside them — never on losing focus, which is the project picker's rename rule
        // and for its reason: a blur fires before the click that caused it, so a field that
        // committed on blur could not be cancelled by the button next to it. A commit is an act
        // rather than a keystroke, which is also what keeps the host's writes un-debounced.
        subscriptions.push(cx.subscribe_in(
            &task_title_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.title = text;
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.commit_task_title(window, cx),
                _ => {}
            },
        ));

        // Enter is the whole of the search field's contract: the panel declares no action, and a
        // search is an act rather than a keystroke, so nothing runs while the query is typed.
        subscriptions.push(cx.subscribe_in(
            &search_query,
            window,
            |this, _input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.run_project_search(window, cx);
                }
            },
        ));

        // The titlebar's field is the same contract, one level up: Enter is the only thing it
        // does, and what it does is hand off to the search panel — see `submit_header_search`.
        subscriptions.push(cx.subscribe_in(
            &command_input,
            window,
            |this, _input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.submit_header_search(window, cx);
                }
            },
        ));

        // The two settings lists commit on Enter and on blur, never on a keystroke: a commit puts
        // `SetSettings` on the bus and the host writes a file. A blur has to commit — the gesture
        // is type and click away, and there is no button beside the field to be cancelled by it.
        subscriptions.push(cx.subscribe_in(
            &search_excludes_input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let list = comma_list(&input.read(cx).value());
                    this.set_search_excludes(list, cx);
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &search_fallbacks_input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let list = comma_list(&input.read(cx).value());
                    this.set_search_fallbacks(list, cx);
                }
            },
        ));

        // No `PressEnter` arm, and no `Blur` arm: Enter is a newline here, and a blur would commit
        // on the very click that asks for the preview.
        subscriptions.push(cx.subscribe_in(
            &task_description_input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.description = text;
                    }
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &step_title_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.step_title = text;
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.commit_step_title(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &new_step_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.new_step = text;
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.add_task_step(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &project_search,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.workbench.project_filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &sink_project_hex,
            window,
            |this, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_sink_project_hex(cx);
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &project_form_hex,
            window,
            |this, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_project_form_hex(cx);
                }
            },
        ));

        // A field's underline is drawn by the parent, so a focus change has to redraw the window
        // rather than only the library widget.
        for handle in [
            sink_input.read(cx).focus_handle(cx),
            sink_textarea.read(cx).focus_handle(cx),
            sink_modal_input.read(cx).focus_handle(cx),
            login_account_input.read(cx).focus_handle(cx),
            new_agent_name_input.read(cx).focus_handle(cx),
            sink_search.read(cx).focus_handle(cx),
            sink_harness_name.read(cx).focus_handle(cx),
            sink_harness_exec.read(cx).focus_handle(cx),
            sink_harness_prompt.read(cx).focus_handle(cx),
            sink_harness_env.read(cx).focus_handle(cx),
            sink_project_name.read(cx).focus_handle(cx),
            sink_project_about.read(cx).focus_handle(cx),
            sink_project_hex.read(cx).focus_handle(cx),
            rename_input.read(cx).focus_handle(cx),
            project_form_about.read(cx).focus_handle(cx),
            project_form_hex.read(cx).focus_handle(cx),
        ] {
            subscriptions.push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_focus_out(&handle, window, |_, _, _, cx| cx.notify()));
        }

        // This window's connection to the host, which is process-wide and already running. The
        // window never starts one: two hosts would race the catalogue and disagree about what
        // exists.
        let bus = BusHub::read(cx).connect();

        let from_host = bus.from_host().clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while let Ok(message) = from_host.recv_async().await {
                if this
                    .update(cx, |this, cx| this.receive(message, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // The log sink nudges the window when a record arrives. A nudge carries nothing: the
        // console reads the ring itself, so a burst is coalesced into one redraw and a window
        // with the console shut is not woken at all.
        let nudges = ubiq_proto::log::logs().subscribe();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while nudges.recv_async().await.is_ok() {
                while nudges.try_recv().is_ok() {}
                // The console is a panel of its own now, so there is no tab to test: a window
                // whose console is a background tab redraws it and lays it out anyway, and one
                // whose bottom region is closed draws nothing for it.
                let shown = this.update(cx, |_, cx| cx.notify());
                if shown.is_err() {
                    break;
                }
                // A record emitted while drawing would otherwise redraw the frame that emitted
                // it, forever.
                let settle = cx.background_executor().timer(Duration::from_millis(120));
                settle.await;
            }
        })
        .detach();

        // An emulator measures its own geometry from the bounds it is given; this is how the
        // measurement gets back into the pane it belongs to.
        let (geometry, measurements) = flume::unbounded::<(PaneId, u16, u16)>();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while let Ok((pane_id, cols, rows)) = measurements.recv_async().await {
                if this
                    .update(cx, |this, cx| this.resize_pane(pane_id, cols, rows, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let mut this = Self {
            window_id,
            this: cx.weak_entity(),
            projects: HashMap::new(),
            active_seen: None,
            parked: HashMap::new(),
            session: SessionId::generate(),
            bus,
            terminals: HashMap::new(),
            geometry,
            pending_focus: None,
            pending_editor_focus: None,
            dock,
            panels,
            pending_panels: Vec::new(),
            pending_layout: None,
            pending_regions: None,
            region_had_content: (false, false, false),
            workbench: WorkbenchState::default(),
            chat: sample::chat(),
            sink: SinkState::default(),
            file_picker: None,
            logs: LogState::default(),
            search: SearchState::new(search_query.clone()),
            adopt_on_list: false,
            adding: false,
            adding_select: None,
            pending_files: Vec::new(),
            diagrams: RefCell::new(HashMap::new()),
            diagram_asks: RefCell::new(Vec::new()),
            viewports: RefCell::new(HashMap::new()),
            viewport_drag: RefCell::new(None),
            chat_input,
            agent_input,
            column_inputs,
            file_filter,
            git_search,
            git_message,
            picker_filter,
            task_filter,
            task_title_input,
            task_description_input,
            step_title_input,
            new_step_input,
            command_input,
            project_search,
            search_query,
            search_excludes_input,
            search_fallbacks_input,
            rename_input,
            project_form_about,
            project_form_hex,
            sink_buffers,
            sink_input,
            sink_textarea,
            sink_modal_input,
            login_account_input,
            new_agent_name_input,
            sink_search,
            sink_harness_name,
            sink_harness_exec,
            sink_harness_prompt,
            sink_harness_env,
            sink_project_name,
            sink_project_about,
            sink_project_hex,
            chat_scroll: ScrollHandle::new(),
            picker_scroll: ScrollHandle::new(),
            explorer_scroll: ScrollHandle::new(),
            agents_scroll: ScrollHandle::new(),
            explorer_filter_gen: 0,
            md_reflow: 0,
            md_reflow_gen: 0,
            log_scroll: UniformListScrollHandle::new(),
            form_filled: None,
            refill_fields: false,
            refill_columns: false,
            fill_project_form: false,
            _subscriptions: subscriptions,
        };

        // Nothing about a project is known until the host says so: the interface reads no disk.
        this.bus.send(Message::ListProjects);
        this.bus.send(Message::GetPreferences {
            scope: Scope::Interface,
        });
        this.bus.send(Message::GetSettings {
            layer: SettingsLayer::Ui,
        });
        this.bus.send(Message::GetSettings {
            layer: SettingsLayer::Host,
        });

        // Whatever the registry says this window holds, it now holds — including the pane a
        // project gets when it is first entered. A window opening on nothing spawns nothing.
        this.sync_projects(cx);
        this
    }

    // ── Which projects this window holds ────────────────────────────
}
