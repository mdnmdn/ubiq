//! What a conversation is, written down — the half of a launch recipe that outlives the process.
//!
//! A conversation the user marked persistent has to be relaunchable after Ubiq has been quit and
//! reopened, and `PendingConversation` (the coordinator's own recipe) is process memory. This is
//! that recipe on disk.
//!
//! **It sits beside the library's `meta.json`, not inside it.** `agent_manager::session::SessionMeta`
//! already records what the *library* knows about a run — the harness, the cwd, the account, the
//! argv, and `harness_session_id`, which is the token a resume needs. What it does not know about is
//! Ubiq's own vocabulary: which definition the start named, which model, thinking level and permission
//! mode the picker settled on, where the transcript's sequence had reached, and whether the user
//! asked for any of it to be kept. Those are Ubiq's questions, so they are in Ubiq's file, and the
//! boundary that keeps `crates/agent-manager` from learning what a chat tab is stays where it is.
//!
//! So a revive reads both: this row for the recipe, `SessionMeta` for the token.
//!
//! The file is `<ubiq root>/sessions/<agent id>/conversation.json`, the same directory the library
//! already writes its own record into, keyed by the same `AgentId` the window mints and the host
//! adopts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::settings::AgentHome;
use ubiq_proto::work::AgentId;

/// The file, inside a conversation's own directory under the sessions root.
const FILE: &str = "conversation.json";

/// One conversation's launch recipe, as it survives a restart.
///
/// Every field added after the first release is `#[serde(default)]`, so a row written by an older
/// build opens with the new fields empty rather than being discarded — the same rule the interface's
/// preference blobs follow, and for the same reason: a conversation is worth more than a field.
///
/// No `Default`: the three fields at the top have no sensible empty value, and `ProjectId` refuses
/// one for the reason every id in this tree does — an id that was not minted is a nil id that looks
/// real.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationRecord {
    /// Which project this conversation belongs to. Needed before anything else on a restore: a
    /// conversation is drawn inside a project or not at all.
    pub project_id: ProjectId,
    /// The library's harness id.
    pub agent_type: String,
    /// Where it ran. A resume must land on the same directory — Claude Code keys its own session
    /// store by a slug of the cwd, so a resume from somewhere else finds nothing and starts blank
    /// without saying so.
    pub cwd: PathBuf,
    /// Which identity it answered as, when one was named.
    #[serde(default)]
    pub account: Option<String>,
    /// The saved setup the picks below sit on top of. `alias` reads the records written while
    /// this was called `profile` (`D174`), so a resumed conversation keeps what it started from.
    #[serde(default, alias = "profile")]
    pub definition: Option<String>,
    /// What the pickers settled on. Empty and absent both mean "whatever the harness defaults to",
    /// which is the convention the coordinator's `chosen_*` fields already read by.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    /// Where the transcript's sequence had reached, so a relaunched pump continues the numbering
    /// rather than restarting it — a restarted sequence reads as a gap, and the window draws it as
    /// one.
    #[serde(default)]
    pub next_seq: u64,
    /// What the conversation is called, once the naming pass has answered. Held here because a name
    /// that lives only in the window that started it is a name a restart loses.
    #[serde(default)]
    pub title: Option<String>,
    /// The naming pass's five-word reading beside [`Self::title`], for the same reason: a revive
    /// restores both onto the live `WorkAgent`.
    #[serde(default)]
    pub summary: Option<String>,
    /// Whether the user asked for this conversation to be kept.
    ///
    /// **This is the single flag every retention decision reads.** It decides whether closing a
    /// window parks the run directory or deletes it, whether the boot sweep passes it over, and
    /// whether the collector may ever take its record. Everything else about persistence follows
    /// from this one boolean.
    #[serde(default)]
    pub persistent: bool,
    /// Whether every permission this conversation's harness asks for is answered `allow` here,
    /// without the ask reaching a window.
    ///
    /// On the row rather than in the launch recipe because it is a property of the *conversation*
    /// and not of any one process: a harness that is unloaded and resumed is the same
    /// conversation, and a user who said "stop asking me" said it about the conversation.
    #[serde(default)]
    pub accept_all: bool,
    /// Whether this conversation's harness can load it back by id (ACP `loadSession`), which is
    /// what lets a conversation left open when Ubiq stopped come back at the next start.
    ///
    /// Read beside [`Self::persistent`] by [`is_kept_at`]: a restorable conversation's run
    /// directory is parked rather than deleted when its window goes, and spared by the boot sweep,
    /// so a restore finds whatever the harness kept there. Unlike `persistent` it does not exempt
    /// the row from the retention collector — a conversation nobody asked to keep still ages out.
    #[serde(default)]
    pub restorable: bool,
    /// Whether this conversation's traffic is being written to a file.
    ///
    /// The flag is durable; the file is not. Each launch opens its own capture, appending to the
    /// same deterministic name, so a conversation left dumping goes on dumping across a restart
    /// without the path having to be stored beside the flag.
    #[serde(default)]
    pub debug_dump: bool,
    /// The conversation this one was forked from, where it was forked. Kept so a transcript that
    /// starts mid-thought can say what it came from.
    #[serde(default)]
    pub forked_from: Option<AgentId>,
    /// Which `$HOME` the run was composed under.
    ///
    /// Recorded because a resume under a different one is not the same run: an agent that ran with
    /// an ephemeral home and resumes under the inherited one silently answers from a different
    /// machine's worth of state. The library's own record does not carry this, which is the gap
    /// `G182` names.
    #[serde(default)]
    pub agent_home: AgentHome,
}

/// Where a conversation's row lives.
pub fn path(sessions: &Path, agent: AgentId) -> PathBuf {
    path_at(sessions, &agent.to_string())
}

/// The same, by the run's own key.
///
/// A run directory is named by whatever owns it — an agent or a pane — and the callers that sweep
/// or collect have that string in hand rather than a parsed id. Parsing it first would buy nothing:
/// both ids are ULIDs, so a pane's key parses as an `AgentId` perfectly well and tells you nothing
/// about which it was. **The presence of the row is the discriminator**, because a pane never has
/// one.
pub fn path_at(sessions: &Path, key: &str) -> PathBuf {
    sessions.join(key).join(FILE)
}

/// Read one row, or nothing.
pub fn load(sessions: &Path, agent: AgentId) -> Option<ConversationRecord> {
    load_at(sessions, &agent.to_string())
}

/// Read one row by the run's own key.
///
/// Every failure answers `None` alike — no file, unreadable, or written by a build whose shape this
/// one cannot parse. A row is a convenience over a conversation that still exists on disk; refusing
/// to start because one would not parse would be the tail wagging the dog.
pub fn load_at(sessions: &Path, key: &str) -> Option<ConversationRecord> {
    let path = path_at(sessions, key);
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text)
        .inspect_err(|error| tracing::debug!("discarding {}: {error}", path.display()))
        .ok()
}

/// Whether a run's conversation is one to keep — the question every destructive path asks.
///
/// **A row that exists but will not parse answers `true`.** The three callers of this are all about
/// to delete something: a window closing, the boot sweep, and the collector. A row is written only
/// for a conversation somebody asked Ubiq to keep, so its presence is the intent even when its
/// contents have been truncated by a crash mid-write or written by a build whose shape this one
/// cannot read. Answering `false` there would delete the harness's whole session store on the
/// strength of a parse error, which is the one outcome none of those callers can undo.
///
/// Only a *missing* row means not persistent — and that is the common case, because a pane never
/// has one.
pub fn is_persistent_at(sessions: &Path, key: &str) -> bool {
    let path = path_at(sessions, key);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    match serde_json::from_str::<ConversationRecord>(&text) {
        Ok(record) => record.persistent,
        Err(error) => {
            tracing::warn!(
                "{} will not parse ({error}); keeping the run directory rather than \
                 deleting a conversation on the strength of a parse error",
                path.display()
            );
            true
        }
    }
}

/// Whether a run's directory is to be kept when its process goes — the conversation is
/// persistent, or its harness can load it back after a restart. The question the close and the
/// boot sweep ask; the retention collector asks [`is_persistent_at`] instead.
///
/// Fails safe the same way: a row that will not parse counts as kept.
pub fn is_kept_at(sessions: &Path, key: &str) -> bool {
    let path = path_at(sessions, key);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    match serde_json::from_str::<ConversationRecord>(&text) {
        Ok(record) => record.persistent || record.restorable,
        Err(error) => {
            tracing::warn!(
                "{} will not parse ({error}); keeping the run directory",
                path.display()
            );
            true
        }
    }
}

/// The user prompts written down for one conversation, one JSON object per line.
const PROMPTS: &str = "prompts.jsonl";

/// One user prompt, as [`append_prompt`] writes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPrompt {
    /// When it was sent, epoch milliseconds.
    pub at: i64,
    pub text: String,
}

/// Append one prompt the user sent to the conversation's own log, beside its row.
///
/// One line per turn, appended, so a crash loses at most the turn in flight. It is what a
/// restore draws the user's side of the conversation from when the harness will not replay it
/// itself. Best effort: a log that cannot be written must never stop a turn going out.
pub fn append_prompt(sessions: &Path, agent: AgentId, at: i64, text: &str) {
    use std::io::Write as _;
    let path = sessions.join(agent.to_string()).join(PROMPTS);
    let Ok(mut line) = serde_json::to_vec(&SavedPrompt {
        at,
        text: text.to_string(),
    }) else {
        return;
    };
    line.push(b'\n');
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        })
        .and_then(|mut file| file.write_all(&line));
    if let Err(error) = written {
        tracing::warn!("could not append to {}: {error}", path.display());
    }
}

/// Every prompt written down for one conversation, oldest first. A line that will not parse — the
/// one a crash cut short — is skipped rather than costing the rest.
pub fn prompts(sessions: &Path, agent: AgentId) -> Vec<SavedPrompt> {
    let path = sessions.join(agent.to_string()).join(PROMPTS);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Write one row, atomically.
///
/// Best effort, like every other write under the sessions root: a root that cannot be written must
/// never fail a launch the user already saw happen. The failure is logged rather than returned
/// because no caller has anything better to do with it.
pub fn save(sessions: &Path, agent: AgentId, record: &ConversationRecord) {
    let path = path(sessions, agent);
    let Ok(bytes) = serde_json::to_vec_pretty(record) else {
        return;
    };
    if let Err(error) = crate::atomic::write_atomic(&path, &bytes) {
        tracing::warn!("could not write {}: {error}", path.display());
    }
}

/// Forget one row. The run directory and the library's own record are separate deletions — see
/// `Agents::retire_agent` — and a caller taking a conversation away owes all three.
pub fn forget(sessions: &Path, agent: AgentId) {
    let path = path(sessions, agent);
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("could not remove {}: {error}", path.display());
    }
}

/// Every row under the sessions root, by the agent it belongs to.
///
/// A directory whose name is not an `AgentId` is left alone and not reported — never claim what you
/// cannot identify, because it was not this that put it there. The same rule `gc::orphans` follows.
pub fn all(sessions: &Path) -> HashMap<AgentId, ConversationRecord> {
    let Ok(entries) = std::fs::read_dir(sessions) else {
        return HashMap::new();
    };
    let mut found = HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(agent) = name.parse::<AgentId>() else {
            continue;
        };
        if let Some(record) = load(sessions, agent) {
            found.insert(agent, record);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> ConversationRecord {
        ConversationRecord {
            project_id: ProjectId::generate(),
            agent_type: "claude-code".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            account: Some("work".to_string()),
            definition: None,
            model: Some("sonnet".to_string()),
            thinking: None,
            mode: None,
            next_seq: 42,
            title: Some("Fixing the parser".to_string()),
            summary: Some("Fixing the markdown parser".to_string()),
            persistent: true,
            accept_all: false,
            restorable: false,
            debug_dump: false,
            forked_from: None,
            agent_home: AgentHome::Inherit,
        }
    }

    #[test]
    fn a_row_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        let written = row();
        save(dir.path(), agent, &written);
        assert_eq!(load(dir.path(), agent), Some(written));
    }

    #[test]
    fn a_missing_row_is_nothing_rather_than_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(load(dir.path(), AgentId::generate()), None);
    }

    #[test]
    fn an_unparseable_row_is_discarded_rather_than_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        let path = path(dir.path(), agent);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "{ not json").expect("write");
        assert_eq!(load(dir.path(), agent), None);
    }

    #[test]
    fn a_row_written_before_a_field_existed_still_opens() {
        // Only the three fields that have no default. Everything added later must be `default`,
        // or a conversation is lost to a field.
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        let path = path(dir.path(), agent);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let id = ProjectId::generate();
        std::fs::write(
            &path,
            format!(r#"{{"project_id":"{id}","agent_type":"codex","cwd":"/tmp/p"}}"#),
        )
        .expect("write");
        let record = load(dir.path(), agent).expect("row");
        assert_eq!(record.agent_type, "codex");
        assert!(!record.persistent);
        assert_eq!(record.next_seq, 0);
    }

    /// The three callers of this are all about to delete a harness's whole session store, and a
    /// parse error is not the user saying they stopped wanting the conversation.
    #[test]
    fn a_row_that_will_not_parse_still_counts_as_persistent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        let path = path(dir.path(), agent);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        // Truncated mid-write, or written by a build whose shape this one cannot read.
        std::fs::write(&path, r#"{"persistent": true"#).expect("write");

        assert_eq!(load(dir.path(), agent), None);
        assert!(is_persistent_at(dir.path(), &agent.to_string()));
    }

    #[test]
    fn a_missing_row_is_the_only_thing_that_means_not_persistent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        assert!(!is_persistent_at(dir.path(), &agent.to_string()));

        let mut unmarked = row();
        unmarked.persistent = false;
        save(dir.path(), agent, &unmarked);
        assert!(!is_persistent_at(dir.path(), &agent.to_string()));

        save(dir.path(), agent, &row());
        assert!(is_persistent_at(dir.path(), &agent.to_string()));
    }

    /// A restorable row keeps its run directory without being exempt from the collector.
    #[test]
    fn a_restorable_row_is_kept_but_not_persistent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        let key = agent.to_string();
        let mut restorable = row();
        restorable.persistent = false;
        restorable.restorable = true;
        save(dir.path(), agent, &restorable);
        assert!(is_kept_at(dir.path(), &key));
        assert!(!is_persistent_at(dir.path(), &key));

        restorable.restorable = false;
        save(dir.path(), agent, &restorable);
        assert!(!is_kept_at(dir.path(), &key));
    }

    /// Prompts append in order, and a line a crash cut short costs only itself.
    #[test]
    fn prompts_append_in_order_and_survive_a_torn_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = AgentId::generate();
        assert!(prompts(dir.path(), agent).is_empty());
        append_prompt(dir.path(), agent, 1, "first");
        append_prompt(dir.path(), agent, 2, "second\nwith a newline");
        let path = dir.path().join(agent.to_string()).join(PROMPTS);
        let mut text = std::fs::read_to_string(&path).expect("read");
        text.push_str(r#"{"at":3,"te"#);
        std::fs::write(&path, text).expect("write");

        let read: Vec<String> = prompts(dir.path(), agent)
            .into_iter()
            .map(|prompt| prompt.text)
            .collect();
        assert_eq!(read, vec!["first", "second\nwith a newline"]);
    }

    #[test]
    fn forgetting_a_row_that_is_not_there_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        forget(dir.path(), AgentId::generate());
    }

    #[test]
    fn all_reports_every_row_and_ignores_what_it_cannot_identify() {
        let dir = tempfile::tempdir().expect("tempdir");
        let kept = AgentId::generate();
        save(dir.path(), kept, &row());
        std::fs::create_dir_all(dir.path().join("not-an-agent-id")).expect("mkdir");

        let found = all(dir.path());
        assert_eq!(found.len(), 1);
        assert!(found.contains_key(&kept));
    }
}
