//! The feedback modal, as the interface holds it while it is up.
//!
//! One struct, on [`crate::state::clone::CloneState`]'s shape and for its reason: every field is
//! answerable at any time and none of them is a step.
//!
//! **The screenshot is taken before the modal opens, never after.** A window photographed with the
//! modal on it is a picture of the modal, so the control captures first and raises this second —
//! which is why `shot` is a field here rather than something the modal asks for.
//!
//! **An answer naming a report this state no longer holds is discarded**, the discipline every
//! id-carrying family in the window follows.

use ubiq_proto::feedback::{FeedbackKind, FeedbackOffer};
use ubiq_proto::ids::FeedbackId;

/// The feedback modal, while it is up.
#[derive(Default)]
pub struct FeedbackForm {
    /// One line, mirrored from the title field. What the report is called.
    pub title: String,
    pub kind: FeedbackKind,
    /// The body, mirrored from the textarea.
    pub description: String,
    /// The window as it was when the control was pressed, PNG encoded. `None` where the platform
    /// offers no capture, where the user has it turned off, or where the capture failed — all
    /// three are the same thing to the modal: there is no picture to show or to send.
    pub shot: Option<Vec<u8>>,
    /// The report in flight, if one is. Set when Send is pressed and cleared by the answer;
    /// the modal is read-only while it is set, so one press is one report.
    pub sending: Option<FeedbackId>,
    /// Why the last attempt did not land. A sentence, drawn above the fields.
    pub error: Option<String>,
    /// What the destination called the report that did land. Set once, and the modal becomes a
    /// thank-you with a link.
    pub sent: Option<Sent>,
}

/// A report that landed, as the modal reports it back.
pub struct Sent {
    /// What the destination called it — an issue number, a ticket id. `None` where it named
    /// nothing, which is still a success.
    pub reference: Option<String>,
    /// Where to go and look, when the destination returned a link.
    pub url: Option<String>,
}

impl FeedbackForm {
    /// Whether Send does anything. A title is the one thing a report cannot be without — a
    /// description is optional, and so is the picture.
    pub fn ready(&self, offer: &FeedbackOffer) -> bool {
        offer.enabled
            && self.sending.is_none()
            && self.sent.is_none()
            && !self.title.trim().is_empty()
    }

    /// Whether an answer naming this report is still wanted.
    pub fn awaits(&self, report_id: FeedbackId) -> bool {
        self.sending == Some(report_id)
    }
}
