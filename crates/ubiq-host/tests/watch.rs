//! The filesystem watch, against real files: no coordinator, no project record, just a `Job` and a
//! `Mailbox` addressed at a plain bus client.

use std::fs;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use ubiq_host::watch;
use ubiq_proto::bus::{self, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

/// Generous: a platform watcher's first event can lag the write by a lot on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(5);

#[test]
fn a_new_file_is_reported_and_an_ignored_one_is_not() {
    let dir = TempDir::new().unwrap();
    // Production roots are canonical (`ProjectRecord.path`); a temp dir is not on macOS, where
    // `/var` resolves to `/private/var` and every event path would fail to strip the prefix.
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join(".gitignore"), "secret.txt\n").unwrap();

    let (hub, host) = bus::hub();
    let client = hub.connect();
    let project_id = ProjectId::generate();
    let _watcher = watch::start(watch::Job {
        project_id,
        root: root.clone(),
        excludes: Vec::new(),
        reply_to: host.mailbox(To::Client(client.id())),
    })
    .expect("the watch to start");

    fs::write(root.join("secret.txt"), "no").unwrap();
    fs::write(root.join("seen.txt"), "yes").unwrap();

    // Keep reading until the file we expect is named — the ignored write may have produced its own
    // (empty-`changed`) flush first, but must never appear in one.
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match client
            .from_host()
            .recv_timeout(left)
            .expect("the watcher to name the changed file")
        {
            Message::ProjectFilesChanged {
                project_id: named,
                changed,
                ..
            } => {
                assert_eq!(named, project_id);
                assert!(
                    !changed.iter().any(|path| path == "secret.txt"),
                    "a .gitignore'd file must not be reported: {changed:?}"
                );
                if changed.iter().any(|path| path == "seen.txt") {
                    return;
                }
            }
            other => panic!("unexpected message from the watcher: {other:?}"),
        }
    }
}
