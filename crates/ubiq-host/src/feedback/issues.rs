//! The second destination: an issue on a repository host.
//!
//! One `POST` to `https://api.github.com/repos/{owner}/{name}/issues` with the token in
//! `Authorization`, the report's title as the issue's, its kind as a label, and its description
//! plus the build's version and platform as the body.
//!
//! **The screenshot does not travel.** GitHub attaches a picture to an issue through a browser
//! upload, not through the issues API, and a `data:` URI in markdown renders as nothing. So the
//! body says a screenshot was taken and did not come with it, rather than posting a megabyte of
//! base64 into a comment nobody can read. Attaching it properly means a second store to upload to,
//! which is what the API destination already is.
//!
//! Nothing here is the user's own GitHub identity: the token is the build's, compiled in, and
//! [`crate::connectors`] — which holds identities the user authorised — is deliberately not
//! consulted. A report is not a thing to sign with somebody's repository credential.

use std::time::Duration;

use anyhow::bail;
use serde_json::{Value, json};
use ubiq_proto::feedback::{FeedbackChannel, FeedbackReceipt, FeedbackReport};

use super::{Destination, USER_AGENT};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The repository issues land in, and the token that may open one.
pub struct Issues {
    repo: &'static str,
    token: &'static str,
}

impl Issues {
    pub fn new(repo: &'static str, token: &'static str) -> Self {
        Self { repo, token }
    }
}

impl Destination for Issues {
    fn channel(&self) -> FeedbackChannel {
        FeedbackChannel::Issues
    }

    fn send(&self, report: &FeedbackReport) -> anyhow::Result<FeedbackReceipt> {
        let url = format!(
            "https://api.github.com/repos/{}/issues",
            self.repo.trim_matches('/')
        );
        let response = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .send_json(json!({
                "title": report.title,
                "body": body(report),
                "labels": [report.kind.slug()],
            }));

        let response = match response {
            Ok(response) => response,
            // The status alone. A 404 here is the usual shape of a token that may not open issues
            // on this repository, which is worth saying because the fix is a build's, not a user's.
            Err(ureq::Error::Status(code, _)) => bail!("the issue tracker answered {code}"),
            Err(ureq::Error::Transport(transport)) => {
                bail!(
                    "the issue tracker could not be reached: {}",
                    transport.kind()
                )
            }
        };

        let answered: Value = response.into_json().unwrap_or(Value::Null);
        Ok(FeedbackReceipt {
            report_id: report.report_id,
            reference: answered
                .get("number")
                .and_then(Value::as_u64)
                .map(|number| format!("#{number}")),
            url: answered
                .get("html_url")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

/// The issue's body: what the user wrote, then what the build was.
fn body(report: &FeedbackReport) -> String {
    let mut body = report.description.trim().to_string();
    if body.is_empty() {
        body.push_str("_No description._");
    }
    body.push_str("\n\n---\n\n");
    body.push_str(&format!(
        "- Kind: {}\n- Ubiq: {}\n- Platform: {}\n",
        report.kind.label(),
        report.version,
        report.platform
    ));
    if report.screenshot.is_some() {
        body.push_str(
            "- A screenshot was taken with this report. The issues API cannot carry it, so it \
             was not attached.\n",
        );
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::feedback::FeedbackKind;
    use ubiq_proto::ids::FeedbackId;

    fn report(description: &str, screenshot: Option<Vec<u8>>) -> FeedbackReport {
        FeedbackReport {
            report_id: FeedbackId::generate(),
            title: "It crashed".to_string(),
            kind: FeedbackKind::Bug,
            description: description.to_string(),
            screenshot,
            version: "0.1.0".to_string(),
            platform: "linux/x86_64".to_string(),
        }
    }

    #[test]
    fn the_body_states_the_build() {
        let body = body(&report("The pane went blank.", None));
        assert!(body.starts_with("The pane went blank."));
        assert!(body.contains("- Ubiq: 0.1.0"));
        assert!(body.contains("- Platform: linux/x86_64"));
        assert!(!body.contains("screenshot"));
    }

    #[test]
    fn an_empty_description_still_reads_as_something() {
        assert!(body(&report("   ", None)).starts_with("_No description._"));
    }

    #[test]
    fn a_screenshot_is_named_rather_than_attached() {
        assert!(body(&report("x", Some(vec![1, 2, 3]))).contains("not attached"));
    }
}
