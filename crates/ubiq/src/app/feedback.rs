//! The feedback control: what the balloon does, and what the host says back.
//!
//! **The picture is taken first and the modal second.** Pressing the control photographs the
//! window through [`AppState::shoot_window`] and raises the modal from the answer, so what the
//! report carries is the window the user was looking at when something went wrong — not the modal
//! they opened to say so. Where no capture is possible the modal is raised all the same, with no
//! picture: a report is worth sending without one.
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
    /// Ask the host whether this build can send feedback. Asked once, at boot: the destination is
    /// compiled in, so the answer cannot change while the process runs.
    pub fn ask_feedback_offer(&mut self) {
        self.bus.send_to(HostRef::Local, Message::QueryFeedback);
    }

    /// Press the balloon: photograph the window, then raise the modal over the picture.
    pub fn open_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.feedback.is_some() {
            return;
        }
        self.workbench.open_menu = None;
        self.shoot_window(window, cx, |this, shot, cx| {
            this.workbench.feedback = Some(FeedbackForm {
                shot,
                ..FeedbackForm::default()
            });
            cx.notify();
        });
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
            cx.notify();
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
