//! Sending the user's report outward: what a destination is, and which one this build has.
//!
//! **A destination is compiled in, not configured.** Which service a build reports to is a
//! property of the build — `option_env!`, on [`crate::connectors::providers`]'s discipline and for
//! its reason: reading the environment at run time would let a user's shell decide where their
//! words and their screenshot are posted, and an intake key that ships in a binary is not a
//! credential the user holds. A build compiled without the variables has no destination, says so
//! in [`FeedbackOffer`], and the interface draws its send button disabled.
//!
//! **A destination is a trait, not a match.** [`Destination`] is the whole surface: one call that
//! takes a report and answers with a receipt. [`api::Api`] is the first implementation — one POST
//! to [`API_URL`] with [`API_KEY`] — and [`issues::Issues`] the second, which opens an issue on a
//! repository host. Adding a third is a file here and a row in [`select`], nothing else.
//!
//! **The call is blocking and never runs on the coordinator's thread.** [`Feedback`] is one
//! worker thread over a queue, on [`crate::quota`]'s pattern: first in, first answered, and the
//! coordinator returns to the panes' keystrokes the instant a job is submitted.
//!
//! Nothing here logs the report's body, and nothing logs a key.

#[cfg(feature = "listener")]
pub mod api;
#[cfg(feature = "listener")]
pub mod issues;

use std::thread;

use ubiq_proto::bus::Mailbox;
use ubiq_proto::feedback::{
    FeedbackChannel, FeedbackError, FeedbackOffer, FeedbackReceipt, FeedbackReport,
};
use ubiq_proto::messages::Message;

/// Where reports go, and what buys the right to post one. Compile-time only — see the module note.
pub const API_URL: Option<&str> = option_env!("UBIQ_FEEDBACK_URL");
pub const API_KEY: Option<&str> = option_env!("UBIQ_FEEDBACK_API_KEY");

/// The repository issues are opened on, `owner/name`, and the token that may open one.
pub const ISSUES_REPO: Option<&str> = option_env!("UBIQ_FEEDBACK_GITHUB_REPO");
pub const ISSUES_TOKEN: Option<&str> = option_env!("UBIQ_FEEDBACK_GITHUB_TOKEN");

/// What both halves put in the `User-Agent` of a feedback call.
pub const USER_AGENT: &str = concat!("ubiq/", env!("CARGO_PKG_VERSION"));

/// Somewhere a report can land.
///
/// One method and no state machine: a report is composed by the interface, sent once, and either
/// lands or does not. An implementation blocks — it is called on [`Feedback`]'s thread, never on
/// the coordinator's.
pub trait Destination: Send + Sync {
    /// Which destination this is, for the sentence the modal puts under its send button.
    fn channel(&self) -> FeedbackChannel;

    /// Post one report. `Ok` carries whatever the service called it; `Err`'s chain becomes the
    /// sentence the user reads, so it names what was being done and never carries a key.
    fn send(&self, report: &FeedbackReport) -> anyhow::Result<FeedbackReceipt>;
}

/// The destination this build ships, if it ships one.
///
/// The API wins where both are compiled in: it is the one that takes the screenshot. A build with
/// neither returns `None`, which is the state a source build is in and not a failure.
pub fn select() -> Option<Box<dyn Destination>> {
    #[cfg(feature = "listener")]
    {
        if let (Some(url), Some(key)) = (API_URL, API_KEY) {
            return Some(Box::new(api::Api::new(url, key)));
        }
        if let (Some(repo), Some(token)) = (ISSUES_REPO, ISSUES_TOKEN) {
            return Some(Box::new(issues::Issues::new(repo, token)));
        }
    }
    None
}

/// What the interface is told before it draws the control.
pub fn offer(destination: Option<&dyn Destination>) -> FeedbackOffer {
    match destination {
        Some(destination) => FeedbackOffer {
            enabled: true,
            channel: destination.channel(),
        },
        None => FeedbackOffer {
            enabled: false,
            channel: FeedbackChannel::None,
        },
    }
}

/// One report on its way out, and the window waiting to hear whether it landed.
pub struct Job {
    pub report: Box<FeedbackReport>,
    /// The window that sent it. It hears back whether the post worked or not.
    pub reply_to: Mailbox,
}

/// The thread that posts a report.
///
/// One thread over an unbounded queue, on [`crate::quota::Quota`]'s pattern and for its reason:
/// the post is a blocking HTTPS call, and anything blocking on the coordinator's thread stalls
/// every pane's keystrokes. No ticker and no retry — a report that failed is said to have failed,
/// and the user still has their words in the modal.
pub struct Feedback {
    jobs: flume::Sender<Job>,
    offer: FeedbackOffer,
}

impl Feedback {
    /// Start the worker over whatever destination this build was compiled with. It ends when the
    /// coordinator that holds this drops it.
    pub fn start() -> Self {
        let destination = select();
        let offer = offer(destination.as_deref());
        let (jobs, queue) = flume::unbounded::<Job>();
        thread::Builder::new()
            .name("ubiq-feedback".to_string())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    job.reply_to.send(answer(destination.as_deref(), &job));
                }
            })
            .expect("the feedback thread");
        Self { jobs, offer }
    }

    /// Whether this build can send feedback, and where to. A compile-time fact, so it is read
    /// rather than asked.
    pub fn offer(&self) -> FeedbackOffer {
        self.offer.clone()
    }

    /// Queue one report. Never blocks — the queue is unbounded, on the bus's own rule.
    pub fn submit(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            tracing::error!("the feedback thread has gone; a report was dropped");
        }
    }
}

/// Post once, and say what became of it.
fn answer(destination: Option<&dyn Destination>, job: &Job) -> Message {
    let report_id = job.report.report_id;
    let Some(destination) = destination else {
        return Message::FeedbackFailed {
            failure: FeedbackError {
                report_id,
                error: "this build of Ubiq was compiled without a feedback destination".to_string(),
            },
        };
    };
    match destination.send(&job.report) {
        Ok(receipt) => Message::FeedbackSent { receipt },
        Err(error) => {
            // The sentence the user reads. `anyhow`'s chain names what was being done and what
            // failed; nothing key-shaped ever reaches it.
            let error = format!("{error:#}");
            tracing::warn!("feedback could not be sent: {error}");
            Message::FeedbackFailed {
                failure: FeedbackError { report_id, error },
            }
        }
    }
}

/// Standard base64, for a screenshot that has to travel inside a JSON body.
///
/// Written here rather than taken as a dependency: it is fifteen lines, it is called once per
/// report, and the crate that would provide it draws nothing else in.
#[cfg_attr(not(feature = "listener"), expect(dead_code))]
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (at, byte)| {
            bits | u32::from(*byte) << (16 - 8 * at)
        });
        for at in 0..=chunk.len() {
            out.push(ALPHABET[(bits >> (18 - 6 * at)) as usize & 0x3f] as char);
        }
        for _ in chunk.len()..3 {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_build_with_no_destination_offers_nothing() {
        let offer = offer(None);
        assert!(!offer.enabled);
        assert_eq!(offer.channel, FeedbackChannel::None);
    }
}
