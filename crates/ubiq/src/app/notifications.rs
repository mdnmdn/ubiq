use super::*;

use ubiq_proto::ids::NotificationId;

impl AppState {
    /// Raise a notification. **This is what every other subsystem calls**: build a
    /// [`NotificationRequest`] and hand it over, and the host decides whether a standing mute rule
    /// silences it, mints its id and tells every window. Nothing is filed here, so a raiser never
    /// has to know which window it is running in.
    pub fn raise_notification(&mut self, request: NotificationRequest) {
        self.bus.send(Message::RaiseNotification { request });
    }

    /// The bell itself.
    ///
    /// **A flashing bell carrying a link is a shortcut, not a switch.** The thing that just
    /// arrived is what the user is reaching for, so the click goes there and stops the flash; the
    /// list is what the same click opens once the bell is at rest again.
    pub fn toggle_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notifications.flash_until.is_some()
            && let Some(link) = self.notifications.flash_link.clone()
        {
            self.clear_flash();
            self.follow_notification_link(link, window, cx);
            cx.notify();
            return;
        }
        self.notifications.open = !self.notifications.open;
        cx.notify();
    }

    pub fn close_notifications(&mut self, cx: &mut Context<Self>) {
        self.notifications.open = false;
        self.notifications.muting = None;
        cx.notify();
    }

    /// Tick one off, or every one of them with `None`. The badge follows the host's answer rather
    /// than a local edit, so nothing is changed here.
    pub fn read_notification(&mut self, id: Option<NotificationId>) {
        self.bus.send(Message::ReadNotifications { id });
    }

    /// Drop one from the history, or every one of them with `None`.
    pub fn dismiss_notification(&mut self, id: Option<NotificationId>) {
        self.bus.send(Message::DismissNotifications { id });
    }

    /// Start collecting a mute rule for one scope. The level and the duration come next, and the
    /// duration is what sends it.
    pub fn begin_mute(&mut self, scope: MuteScope, cx: &mut Context<Self>) {
        self.notifications.muting = Some(MutePick::new(scope));
        cx.notify();
    }

    pub fn set_mute_level(&mut self, level: Level, cx: &mut Context<Self>) {
        if let Some(pick) = self.notifications.muting.as_mut() {
            pick.max_level = level;
        }
        cx.notify();
    }

    pub fn cancel_mute(&mut self, cx: &mut Context<Self>) {
        self.notifications.muting = None;
        cx.notify();
    }

    /// Send the rule the picker has been collecting. Picking a duration is the confirm, which is
    /// why there is no separate button to press after it.
    pub fn confirm_mute(&mut self, duration: MuteFor, cx: &mut Context<Self>) {
        let Some(pick) = self.notifications.muting.take() else {
            return;
        };
        self.bus.send(Message::MuteNotifications {
            scope: pick.scope,
            max_level: pick.max_level,
            duration,
        });
        cx.notify();
    }

    /// Lift the rule standing on one scope. Lifting a scope with no rule is not an error, so this
    /// never has to check first.
    pub fn unmute(&mut self, scope: MuteScope) {
        self.bus.send(Message::UnmuteNotifications { scope });
    }

    /// Go where a notification points. Best effort: a link naming something that has since gone
    /// does nothing rather than half-arriving somewhere.
    pub fn follow_notification_link(
        &mut self,
        link: UbiqLink,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match link {
            UbiqLink::Url(url) => cx.open_url(&url),
            UbiqLink::Pane(pane) => self.focus_pane(pane, cx),
            UbiqLink::Project(project) => self.activate_project(project, cx),
            // The agents screen, that agent in a column of its own — the same way a task card
            // reaches the conversation behind it. It arrives in the project on screen, which is
            // the only one this window can name an agent inside.
            UbiqLink::Agent(agent) => self.open_task_chat(agent, cx),
            UbiqLink::File { project, path } => self.navigate(
                Destination::new(
                    project,
                    View::Ide {
                        key: tab_key(&path, Subject::File),
                    },
                ),
                cx,
            ),
        }
    }

    /// Start the bell flashing for something that arrived loud.
    ///
    /// A second arrival mid-flash pushes the deadline out rather than starting a second timer:
    /// two loops flipping the same flag would cancel each other out and the bell would sit still.
    pub(super) fn flash_bell(
        &mut self,
        level: Level,
        link: Option<UbiqLink>,
        cx: &mut Context<Self>,
    ) {
        let running = self.notifications.flash_until.is_some();
        self.notifications.flash_until = Some(std::time::Instant::now() + FLASH);
        self.notifications.flash_level = Some(level);
        self.notifications.flash_link = link;
        self.notifications.flash_on = true;
        cx.notify();
        if running {
            return;
        }

        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            loop {
                cx.background_executor().timer(BLINK).await;
                // Two ways out, both here rather than in the body: the window has gone, or the
                // deadline the last arrival set has passed.
                let blinking = this.update(cx, |this, cx| {
                    let done = this
                        .notifications
                        .flash_until
                        .is_none_or(|until| std::time::Instant::now() >= until);
                    if done {
                        this.clear_flash();
                    } else {
                        this.notifications.flash_on = !this.notifications.flash_on;
                    }
                    cx.notify();
                    !done
                });
                if !matches!(blinking, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// The bell at rest. The badge is what carries the fact afterwards, so nothing but the flash
    /// is cleared.
    fn clear_flash(&mut self) {
        self.notifications.flash_until = None;
        self.notifications.flash_on = false;
        self.notifications.flash_level = None;
        self.notifications.flash_link = None;
    }
}
