//! Collecting the directories of projects that are no longer in the catalogue, and of
//! conversations that have outlived their retention window.
//!
//! Forgetting a project drops the record first and its directory second, so a crash between the
//! two leaves a directory with no record. That is garbage, and this is where it is collected.
//!
//! A conversation's directory is not garbage in that sense — nothing ever fails to write one — but
//! nothing has ever deleted one either (`G164`), and it holds the user's own data: the harness's
//! transcript copy and Ubiq's own `SessionMeta` sidecar. `stale_conversations` and
//! `collect_conversations` are the other half of this module, one project's worth of code away:
//! the same shape and the same safety rule, applied to a directory a timer may take rather than
//! one a crash already orphaned.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_manager::session;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::work::AgentId;

use crate::conversation_record;

/// Every `projects/<ulid>/` under `root` with no matching record.
///
/// A directory whose name is not a ULID is left alone and logged: never delete what you cannot
/// identify, because it was not this that put it there.
pub fn orphans(root: &Path, keep: &HashSet<ProjectId>) -> Vec<PathBuf> {
    let projects = root.join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };

        match ProjectId::from_str(name) {
            Ok(id) if !keep.contains(&id) => found.push(entry.path()),
            Ok(_) => {}
            Err(_) => {
                tracing::debug!(
                    "leaving {} alone: its name is not a project id",
                    entry.path().display()
                );
            }
        }
    }
    found.sort();
    found
}

/// Collect them. Answers how many went.
///
/// **Only ever called after a load that succeeded.** Running this against an empty catalogue that
/// came from a *corrupt* file would delete every project's view state, which is precisely the
/// thing preserving the file was meant to avoid.
pub fn collect(root: &Path, keep: &HashSet<ProjectId>) -> usize {
    let mut removed = 0;
    for path in orphans(root, keep) {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::info!("collected {}, which no record names", path.display());
                removed += 1;
            }
            Err(error) => tracing::warn!("could not collect {}: {error}", path.display()),
        }
    }
    removed
}

/// Every `sessions/<agent id>/` older than `retain_days` and not exempt.
///
/// `retain_days == 0` answers empty immediately: zero is the user saying never, and it is a real
/// answer rather than a degenerate one. Otherwise a directory is named here unless one of three
/// things exempts it, whatever its age — a record the user marked `persistent`, an `AgentId` the
/// caller passed in `keep` (the coordinator's live and revivable conversations), or a directory
/// this cannot identify.
///
/// Age comes from `SessionMeta::finished_at`, falling back to `created_at` when a run never wrote
/// one — a crash. A half-written record is not evidence of youth or of age, and `created_at` is
/// the honest reading.
///
/// A directory whose name is not an `AgentId`, or with no readable `SessionMeta`, is left alone and
/// logged: never delete what you cannot identify, because it was not this that put it there. Same
/// rule as `orphans`.
pub fn stale_conversations(
    sessions: &Path,
    keep: &HashSet<AgentId>,
    retain_days: u32,
    now: SystemTime,
) -> Vec<PathBuf> {
    if retain_days == 0 {
        return Vec::new();
    }
    let window = Duration::from_secs(u64::from(retain_days) * 24 * 60 * 60);

    let Ok(entries) = std::fs::read_dir(sessions) else {
        return Vec::new();
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };

        let Ok(agent) = AgentId::from_str(name) else {
            tracing::debug!(
                "leaving {} alone: its name is not an agent id",
                entry.path().display()
            );
            continue;
        };
        if keep.contains(&agent) {
            continue;
        }
        // Fail-safe: a row that exists but will not parse counts as persistent. This is a
        // destructive path, and a parse error is not evidence that the user stopped wanting the
        // conversation — see `conversation_record::is_persistent_at`.
        if conversation_record::is_persistent_at(sessions, &agent.to_string()) {
            continue;
        }
        let Ok(meta) = session::load(sessions, &agent.to_string()) else {
            tracing::debug!(
                "leaving {} alone: no readable session record",
                entry.path().display()
            );
            continue;
        };

        let age = UNIX_EPOCH + Duration::from_millis(meta.finished_at.unwrap_or(meta.created_at));
        if now.duration_since(age).unwrap_or_default() >= window {
            found.push(entry.path());
        }
    }
    found.sort();
    found
}

/// Collect them. Answers how many went.
pub fn collect_conversations(
    sessions: &Path,
    keep: &HashSet<AgentId>,
    retain_days: u32,
    now: SystemTime,
) -> usize {
    let mut removed = 0;
    for path in stale_conversations(sessions, keep, retain_days, now) {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::info!("collected {}, past its retention window", path.display());
                removed += 1;
            }
            Err(error) => tracing::warn!("could not collect {}: {error}", path.display()),
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::settings::AgentHome;

    /// A `SessionMeta` for `agent`, with its two timestamps set directly rather than stamped by
    /// `SessionMeta::new`, so a test can age it without sleeping.
    fn meta(agent: AgentId, created_at: u64, finished_at: Option<u64>) -> session::SessionMeta {
        let mut meta = session::SessionMeta::new(
            "claude-code".to_string(),
            PathBuf::from("/tmp/project"),
            Vec::new(),
            None,
            "structured".to_string(),
            PathBuf::from("/tmp/config"),
        );
        meta.id = agent.to_string();
        meta.created_at = created_at;
        meta.finished_at = finished_at;
        meta
    }

    fn record(persistent: bool) -> conversation_record::ConversationRecord {
        conversation_record::ConversationRecord {
            project_id: ProjectId::generate(),
            agent_type: "claude-code".to_string(),
            cwd: PathBuf::from("/tmp/project"),
            account: None,
            profile: None,
            model: None,
            thinking: None,
            mode: None,
            next_seq: 0,
            title: None,
            persistent,
            accept_all: false,
            debug_dump: false,
            forked_from: None,
            agent_home: AgentHome::Inherit,
        }
    }

    /// Unix millis for a point `ago` before `now`.
    fn millis_ago(now: SystemTime, ago: Duration) -> u64 {
        (now - ago)
            .duration_since(UNIX_EPOCH)
            .expect("before epoch")
            .as_millis() as u64
    }

    #[test]
    fn a_record_aged_past_the_window_is_collected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let agent = AgentId::generate();
        session::save(
            dir.path(),
            &meta(
                agent,
                0,
                Some(millis_ago(now, Duration::from_secs(10 * 86400))),
            ),
        )
        .expect("save");

        let stale = stale_conversations(dir.path(), &HashSet::new(), 7, now);
        assert_eq!(stale, vec![dir.path().join(agent.to_string())]);
    }

    #[test]
    fn a_record_inside_the_window_is_not_collected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let agent = AgentId::generate();
        session::save(
            dir.path(),
            &meta(agent, 0, Some(millis_ago(now, Duration::from_secs(3600)))),
        )
        .expect("save");

        assert!(stale_conversations(dir.path(), &HashSet::new(), 7, now).is_empty());
    }

    #[test]
    fn a_persistent_record_is_never_collected_however_old() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let agent = AgentId::generate();
        session::save(
            dir.path(),
            &meta(
                agent,
                0,
                Some(millis_ago(now, Duration::from_secs(365 * 86400))),
            ),
        )
        .expect("save");
        conversation_record::save(dir.path(), agent, &record(true));

        assert!(stale_conversations(dir.path(), &HashSet::new(), 7, now).is_empty());
    }

    #[test]
    fn a_kept_agent_is_never_collected_however_old() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let agent = AgentId::generate();
        session::save(
            dir.path(),
            &meta(
                agent,
                0,
                Some(millis_ago(now, Duration::from_secs(365 * 86400))),
            ),
        )
        .expect("save");

        let keep = HashSet::from([agent]);
        assert!(stale_conversations(dir.path(), &keep, 7, now).is_empty());
    }

    #[test]
    fn retain_days_zero_collects_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let agent = AgentId::generate();
        session::save(
            dir.path(),
            &meta(
                agent,
                0,
                Some(millis_ago(now, Duration::from_secs(365 * 86400))),
            ),
        )
        .expect("save");

        assert!(stale_conversations(dir.path(), &HashSet::new(), 0, now).is_empty());
    }

    #[test]
    fn a_directory_whose_name_is_not_an_agent_id_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("not-an-agent-id")).expect("mkdir");

        assert!(stale_conversations(dir.path(), &HashSet::new(), 7, SystemTime::now()).is_empty());
    }

    #[test]
    fn a_record_with_no_finished_at_is_aged_by_created_at() {
        let dir = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let agent = AgentId::generate();
        session::save(
            dir.path(),
            &meta(
                agent,
                millis_ago(now, Duration::from_secs(10 * 86400)),
                None,
            ),
        )
        .expect("save");

        let stale = stale_conversations(dir.path(), &HashSet::new(), 7, now);
        assert_eq!(stale, vec![dir.path().join(agent.to_string())]);
    }
}
