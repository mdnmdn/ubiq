//! The feedback control: what the balloon does, and what the host says back.
//!
//! **The picture is optional and on request.** The modal opens at once with none, so opening it
//! never asks the OS for screen-recording permission. "Take screenshot" hides the modal, waits for
//! a frame without it, photographs the window through [`AppState::shoot_window`] and restores the
//! modal; "Paste image" (or ⌘V outside the fields) attaches the clipboard's image as PNG. A report
//! is worth sending without a picture.
//!
//! **Where the report goes is not the interface's business.** It sends
//! [`Message::SendFeedback`] and draws the receipt. Which service is behind it, and what key buys
//! the right to post, are the host's and are compiled into the binary — see
//! `ubiq_host::feedback`. The interface learns one thing about it, once at boot: whether there is
//! a destination at all, which is what disables the send button in a build compiled without one.

use ubiq_proto::feedback::{FeedbackKind, FeedbackReport};
use ubiq_proto::ids::FeedbackId;

use crate::state::feedback::{FeedbackForm, Sent};

use super::*;

impl AppState {
    /// Ask the host whether this build can send feedback. Asked once, at boot; the destination is
    /// compiled in but an override set at run time can change it, and the host then broadcasts a
    /// fresh offer.
    pub fn ask_feedback_offer(&mut self) {
        self.bus.send_to(HostRef::Local, Message::QueryFeedback);
    }

    /// Press the balloon: raise the modal at once, with no picture. Nothing is captured here, so
    /// nothing asks the operating system for screen-recording permission.
    pub fn open_feedback(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.feedback.is_some() {
            return;
        }
        self.workbench.open_menu = None;
        self.workbench.feedback = Some(FeedbackForm::default());
        cx.notify();
    }

    /// "Take screenshot": hide the modal (the form stays), let a frame without it reach the
    /// screen, photograph the window, then restore the modal with the picture attached.
    ///
    /// Sequence: `capturing` is set and the window notified; `on_next_frame` fires after the frame
    /// that draws no modal; a short settle timer then lets the compositor present it before the
    /// display is streamed, because `shoot_window` captures the *display*, not our own buffer.
    pub fn take_feedback_shot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let idle = self
            .workbench
            .feedback
            .as_ref()
            .is_some_and(|f| !f.capturing && f.sending.is_none());
        if !idle || !self.capture_offered(cx) {
            return;
        }
        let Some(feedback) = self.workbench.feedback.as_mut() else {
            return;
        };
        feedback.capturing = true;
        feedback.note = None;
        self.workbench.open_menu = None;
        cx.notify();
        let view = cx.entity();
        window.on_next_frame(move |window, cx| {
            view.update(cx, |_, cx| {
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(150))
                        .await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.shoot_window(window, cx, |this, shot, cx| {
                            if let Some(feedback) = this.workbench.feedback.as_mut() {
                                feedback.capturing = false;
                                match shot {
                                    Some(png) => feedback.shot = Some(png),
                                    None => {
                                        feedback.note = Some(
                                            "Could not capture the window. Check the screen \
                                             recording permission."
                                                .to_string(),
                                        )
                                    }
                                }
                            }
                            cx.notify();
                        });
                    });
                })
                .detach();
            });
        });
    }

    /// "Paste image" and ⌘V/Ctrl+V outside the fields: attach the clipboard's image as PNG.
    /// Returns whether the modal took the gesture, so a caller can hand the key back.
    pub fn paste_feedback_image(&mut self, cx: &mut Context<Self>) -> bool {
        match &self.workbench.feedback {
            // The edit dialog is over the modal: the paste is swallowed, not turned into a tab.
            Some(feedback) if feedback.shot_edit.is_some() => return true,
            Some(feedback) if !feedback.capturing && feedback.sending.is_none() => {}
            _ => return false,
        }
        let image = cx.read_from_clipboard().and_then(|item| {
            item.entries().iter().find_map(|entry| match entry {
                gpui::ClipboardEntry::Image(image) if !image.bytes.is_empty() => {
                    Some((image.format, image.bytes.clone()))
                }
                _ => None,
            })
        });
        let png = image.and_then(|(format, bytes)| {
            // Everything but PNG is re-encoded; a format with no decoder here comes back as it
            // arrived and is refused, since the report's picture is PNG by contract.
            match super::clipboard::pasted_image_bytes(format, bytes) {
                (gpui::ImageFormat::Png, png) => Some(png),
                _ => None,
            }
        });
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            match png {
                Some(png) => {
                    feedback.shot = Some(png);
                    feedback.note = None;
                }
                None => feedback.note = Some("No image on the clipboard.".to_string()),
            }
        }
        cx.notify();
        true
    }

    /// Close it. A report already in flight is not cancelled — it has left the window, and the
    /// host answers it whether anything is still drawing or not.
    pub fn close_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.feedback = None;
        if self.workbench.open_menu == Some(MenuId::FeedbackKind) {
            self.workbench.open_menu = None;
        }
        self.set_feedback_fields(window, cx);
        cx.notify();
    }

    /// Mirror the form's two text fields into their inputs. Called when the modal opens and when
    /// it closes, so a second report never opens on the first one's words.
    fn set_feedback_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (title, description) = match &self.workbench.feedback {
            Some(feedback) => (feedback.title.clone(), feedback.description.clone()),
            None => (String::new(), String::new()),
        };
        self.feedback_title_input
            .clone()
            .update(cx, |input, cx| input.set_value(&title, window, cx));
        self.feedback_description
            .clone()
            .update(cx, |input, cx| input.set_value(&description, window, cx));
    }

    // ── The controls ────────────────────────────────────────────────

    pub fn retitle_feedback(&mut self, typed: String, cx: &mut Context<Self>) {
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            feedback.title = typed;
            cx.notify();
        }
    }

    pub fn redescribe_feedback(&mut self, typed: String, cx: &mut Context<Self>) {
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            feedback.description = typed;
            cx.notify();
        }
    }

    /// Pick what the report is about.
    pub fn pick_feedback_kind(&mut self, kind: FeedbackKind, cx: &mut Context<Self>) {
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            feedback.kind = kind;
        }
        self.close_menu(cx);
    }

    /// Drop the screenshot from the report. The user's own call: a window can have anything on it,
    /// and a report is not worth a picture the user would rather not send.
    pub fn drop_feedback_shot(&mut self, cx: &mut Context<Self>) {
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            feedback.shot = None;
            feedback.note = None;
            cx.notify();
        }
    }

    /// "Edit": open the attached picture in the image editor, in a dialog over the modal.
    pub fn open_feedback_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(feedback) = self.workbench.feedback.as_mut() else {
            return;
        };
        if feedback.sending.is_some() || !feedback.begin_edit() {
            return;
        }
        let key = feedback.edit_key();
        self.workbench.open_menu = None;
        if let Some(key) = key {
            // A fresh fit, whatever an earlier edit left the camera at.
            self.reset_viewport(&key, cx);
        }
        // The form's inputs sit under the dialog and would take Cmd+Z, Backspace and the rest of
        // the editor's keys: the workbench root holds focus while the dialog is up.
        self.workbench_focus.focus(window, cx);
        cx.notify();
    }

    /// Apply: the edited PNG replaces the attached picture.
    pub fn apply_feedback_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            feedback.apply_edit();
            feedback.note = None;
        }
        self.focus_feedback_title(window, cx);
        cx.notify();
    }

    /// Cancel (and Escape): the dialog goes, the picture and the rest of the form stay as they were.
    pub fn cancel_feedback_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(feedback) = self.workbench.feedback.as_mut() {
            feedback.shot_edit = None;
        }
        self.focus_feedback_title(window, cx);
        cx.notify();
    }

    /// Back under the modal: the title field takes the keyboard again.
    fn focus_feedback_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.feedback.is_some() {
            self.feedback_title_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        }
    }

    /// Send it. Refuses in place while anything is missing, on `start_clone`'s discipline: the
    /// button is drawn dim rather than unwired, so the rule lives in one place.
    pub fn send_feedback(&mut self, cx: &mut Context<Self>) {
        let offer = self.workbench.feedback_offer.clone();
        let Some(feedback) = self.workbench.feedback.as_mut() else {
            return;
        };
        if !feedback.ready(&offer) {
            return;
        }
        let report_id = FeedbackId::generate();
        let report = FeedbackReport {
            report_id,
            title: feedback.title.trim().to_string(),
            kind: feedback.kind,
            description: feedback.description.trim().to_string(),
            screenshot: feedback.shot.clone(),
            version: crate::version::FULL.to_string(),
            platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        };
        feedback.sending = Some(report_id);
        feedback.error = None;
        // The local host, never whichever one the window is looking at: the destination is this
        // build's, and the offer that enabled the button came from the same place.
        self.bus.send_to(
            HostRef::Local,
            Message::SendFeedback {
                report: Box::new(report),
            },
        );
        cx.notify();
    }

    // ── What the host says back ─────────────────────────────────────

    /// The feedback family: the one-time offer, and a report's single answer.
    ///
    /// Answers with the message when it belongs to another family, on the convention every other
    /// `receive_*` in `wire.rs` follows.
    pub(super) fn receive_feedback(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            // Only the local host's answer counts: the destination is compiled into *this*
            // binary, and a remote host attached later reports its own build's, which is not what
            // this window's control would send through.
            Message::FeedbackOffered { offer } => {
                if host == HostRef::Local {
                    self.workbench.feedback_offer = offer;
                    cx.notify();
                }
            }
            Message::FeedbackSent { receipt } => {
                if let Some(feedback) = self.workbench.feedback.as_mut()
                    && feedback.awaits(receipt.report_id)
                {
                    feedback.sending = None;
                    feedback.sent = Some(Sent {
                        reference: receipt.reference,
                        url: receipt.url,
                    });
                    cx.notify();
                }
            }
            Message::FeedbackFailed { failure } => {
                if let Some(feedback) = self.workbench.feedback.as_mut()
                    && feedback.awaits(failure.report_id)
                {
                    feedback.sending = None;
                    feedback.error = Some(failure.error);
                    cx.notify();
                }
            }
            other => return Some(other),
        }
        None
    }
}
