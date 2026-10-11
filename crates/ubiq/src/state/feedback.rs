//! The feedback modal, as the interface holds it while it is up.
//!
//! One struct, on [`crate::state::clone::CloneState`]'s shape and for its reason: every field is
//! answerable at any time and none of them is a step.
//!
//! **The screenshot is optional and on request.** The modal opens with none; the user can take one
//! (the modal hides itself for the shot — `capturing`) or paste an image from the clipboard.
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
    /// The attached picture, PNG encoded: a window shot taken on request, or a pasted image.
    /// `None` is a valid report.
    pub shot: Option<Vec<u8>>,
    /// The modal is out of the picture while the window is photographed. The form (and so the
    /// fields) stays; only the drawing is skipped.
    pub capturing: bool,
    /// A short inline sentence about the picture: why a capture or a paste attached nothing.
    pub note: Option<String>,
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
