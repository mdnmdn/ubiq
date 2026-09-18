//! The first destination: Ubiq's own intake API.
//!
//! **The service does not exist yet.** What is settled is the shape of the call, which is what a
//! build has to be compiled against — one `POST` of one JSON object to [`super::API_URL`],
//! authenticated with [`super::API_KEY`] in `X-Api-Key`. The request body below is the contract;
//! the response is read leniently, because the only things taken from it are the two the interface
//! draws, and a service that names neither has still accepted the report.
//!
//! ```json
//! {
//!   "id": "01JC…",            // the report's ULID, so a resend is not a second ticket
//!   "title": "…",
//!   "kind": "bug",            // bug | feature | eco-feature
//!   "description": "…",
//!   "version": "0.4.1",
//!   "platform": "macos/aarch64",
//!   "screenshot_png_base64": "iVBOR…"   // absent when there is no screenshot
//! }
//! ```
//!
//! The reply is read for `reference` (or `id`, or `number`) and `url` (or `html_url`), both
//! optional.

use std::time::Duration;

use anyhow::bail;
use serde_json::{Value, json};
use ubiq_proto::feedback::{FeedbackChannel, FeedbackReceipt, FeedbackReport};

use super::{Destination, USER_AGENT, base64};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// One POST, one key.
pub struct Api {
    url: &'static str,
    key: &'static str,
}

impl Api {
    pub fn new(url: &'static str, key: &'static str) -> Self {
        Self { url, key }
    }
}

impl Destination for Api {
    fn channel(&self) -> FeedbackChannel {
        FeedbackChannel::Api
    }

    fn send(&self, report: &FeedbackReport) -> anyhow::Result<FeedbackReceipt> {
        let mut body = json!({
            "id": report.report_id.to_string(),
            "title": report.title,
            "kind": report.kind.slug(),
            "description": report.description,
            "version": report.version,
            "platform": report.platform,
        });
        if let Some(png) = &report.screenshot {
            body["screenshot_png_base64"] = Value::String(base64(png));
        }

        let response = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .post(self.url)
            .set("X-Api-Key", self.key)
            .send_json(body);

        let response = match response {
            Ok(response) => response,
            // The status alone, never the body: an intake service's error body is its own and may
            // echo what was posted.
            Err(ureq::Error::Status(code, _)) => {
                bail!("the feedback service answered {code}")
            }
            Err(ureq::Error::Transport(transport)) => {
                bail!(
                    "the feedback service could not be reached: {}",
                    transport.kind()
                )
            }
        };

        // A service that accepted the report and named nothing is a success with no reference —
        // the user is told it landed, which is the only thing the receipt is for.
        let answered: Value = response.into_json().unwrap_or(Value::Null);
        Ok(FeedbackReceipt {
            report_id: report.report_id,
            reference: string(&answered, &["reference", "id", "number"]),
            url: string(&answered, &["url", "html_url"]),
        })
    }
}

/// The first of `keys` the body states, as a string. A number is taken too — an intake service
/// that answers `{"number": 12}` has named the ticket just as well as one that quotes it.
fn string(body: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match body.get(key) {
        Some(Value::String(found)) if !found.is_empty() => Some(found.clone()),
        Some(Value::Number(found)) => Some(found.to_string()),
        _ => None,
    })
}

/// Read a JSON body the way [`Destination::send`] does. Kept as its own function so the shape the
/// service is expected to answer with is testable without a service.
#[cfg(test)]
fn receipt_of(body: &Value) -> (Option<String>, Option<String>) {
    (
        string(body, &["reference", "id", "number"]),
        string(body, &["url", "html_url"]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_taken_from_whichever_name_the_service_used() {
        assert_eq!(
            receipt_of(&json!({"number": 12, "html_url": "https://x/12"})),
            (Some("12".to_string()), Some("https://x/12".to_string()))
        );
        assert_eq!(
            receipt_of(&json!({"reference": "FB-9"})),
            (Some("FB-9".to_string()), None)
        );
    }

    #[test]
    fn an_empty_answer_is_still_a_receipt() {
        assert_eq!(receipt_of(&Value::Null), (None, None));
    }
}
