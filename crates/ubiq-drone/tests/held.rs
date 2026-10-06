//! `held::serve` driven by a link source that is not a socket listener: socket pairs handed over
//! through a channel, which is also an iterator that blocks in `next()` between links.
//!
//! What is under test is the part that does not depend on the transport: a pane outlives its
//! link, a second link re-announces it, and the linger ends `serve` while the source is still
//! blocked waiting for a link that never comes.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use ubiq_drone::held::{self, Link, Options};
use ubiq_drone::linger::Linger;
use ubiq_drone::relay::Root;
use ubiq_proto::carrier::welcome;
use ubiq_proto::ids::{PaneId, SessionId};
use ubiq_proto::messages::Message;
use ubiq_proto::wire;

const PATIENCE: Duration = Duration::from_secs(20);

/// The client end of one link.
struct Client {
    reader: UnixStream,
    writer: UnixStream,
}

/// A link for `serve` and the client end of it.
fn pair() -> (Link, Client) {
    let (drone, client) = UnixStream::pair().expect("a socket pair");
    let (reader, writer) = (drone.try_clone().unwrap(), drone.try_clone().unwrap());
    let link = Link::new(Box::new(reader), Box::new(writer), Box::new(Hangup(drone)));
    let writer = client.try_clone().unwrap();
    client.set_read_timeout(Some(PATIENCE)).unwrap();
    (
        link,
        Client {
            reader: client,
            writer,
        },
    )
}

struct Hangup(UnixStream);

impl ubiq_host::carrier::Closer for Hangup {
    fn close(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

impl Client {
    /// The handshake, which only goes out once `serve` has been handed the link.
    fn shake(&mut self) {
        welcome(&mut self.reader, &mut self.writer, "held-tests").expect("the handshake");
    }

    fn say(&mut self, message: Message) {
        wire::write_frame(&mut self.writer, &message).expect("writing a frame");
        self.writer.flush().expect("flushing");
    }

    fn wait_for<T>(&mut self, what: &str, mut want: impl FnMut(&Message) -> Option<T>) -> T {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            let Ok(message) = wire::read_frame(&mut self.reader) else {
                break;
            };
            if let Some(found) = want(&message) {
                return found;
            }
        }
        panic!("never saw {what}");
    }

    fn open_pane(&mut self) -> PaneId {
        self.say(Message::ListProjects);
        let project_id = self.wait_for("ProjectList", |message| match message {
            Message::ProjectList { projects } => Some(projects[0].id()),
            _ => None,
        });
        self.say(Message::SpawnWorkspace {
            session_id: SessionId::generate(),
            project_id,
            rel_path: None,
            agent_type: Some("/bin/sh".to_string()),
            args: Vec::new(),
            picks: Default::default(),
        });
        self.wait_for("WorkspaceSpawned", |message| match message {
            Message::WorkspaceSpawned { workspace } => Some(workspace.id),
            _ => None,
        })
    }
}

#[test]
fn a_pane_outlives_its_link_and_the_linger_ends_serve_with_the_source_blocked() {
    let home = tempfile::tempdir().expect("a home for this test");
    let (sender, receiver) = flume::unbounded();
    let options = Options::new(Linger::For(Duration::from_secs(2)));
    let served = std::thread::spawn({
        let root = Root::new(home.path().to_path_buf());
        move || held::serve(vec![root], options, receiver.into_iter())
    });

    let (link, mut first) = pair();
    sender.send(link).unwrap();
    first.shake();
    let pane = first.open_pane();
    drop(first);

    let (link, mut again) = pair();
    sender.send(link).unwrap();
    again.shake();
    let announced = again.wait_for("the pane re-announced", |message| match message {
        Message::WorkspaceSpawned { workspace } => Some(workspace.id),
        _ => None,
    });
    assert_eq!(announced, pane, "the same pane, on a second link");
    drop(again);

    // No third link ever comes, and `sender` is still alive, so the iterator stays blocked in
    // `next()`: only the linger can end `serve`.
    let deadline = Instant::now() + PATIENCE;
    while !served.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(served.is_finished(), "the linger must end serve");
    served.join().unwrap();
    drop(sender);
}

#[test]
fn an_ended_iterator_does_not_end_serve_before_the_linger() {
    let home = tempfile::tempdir().expect("a home for this test");
    let options = Options::new(Linger::For(Duration::from_secs(2)));
    let (link, mut client) = pair();
    let started = Instant::now();
    let served = std::thread::spawn({
        let root = Root::new(home.path().to_path_buf());
        move || held::serve(vec![root], options, std::iter::once(link))
    });
    client.shake();
    let _pane = client.open_pane();
    drop(client);
    served.join().unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "an iterator that ended must leave the linger in charge, not end serve at once"
    );
}

#[test]
fn a_stop_ends_serve_with_no_socket() {
    let home = tempfile::tempdir().expect("a home for this test");
    let (sender, receiver) = flume::unbounded::<Link>();
    let options = Options::new(Linger::Never);
    let live = options.live.clone();
    let served = std::thread::spawn({
        let root = Root::new(home.path().to_path_buf());
        move || held::serve(vec![root], options, receiver.into_iter())
    });
    live.stop();
    let deadline = Instant::now() + PATIENCE;
    while !served.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(served.is_finished(), "a stop must end serve");
    served.join().unwrap();
    drop(sender);
}

#[test]
fn the_state_file_is_written_and_removed() {
    let home = tempfile::tempdir().expect("a home for this test");
    let socket = home.path().join("drone.sock");
    let state = ubiq_drone::state::state_path(&socket);
    let (sender, receiver) = flume::unbounded::<Link>();
    let mut options = Options::new(Linger::Never);
    options.state = Some(socket);
    let live = options.live.clone();
    let served = std::thread::spawn({
        let root = Root::new(home.path().to_path_buf());
        move || held::serve(vec![root], options, receiver.into_iter())
    });
    let deadline = Instant::now() + PATIENCE;
    while !state.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(state.exists(), "the state file is written at start");
    live.stop();
    served.join().unwrap();
    assert!(!state.exists(), "the state file is removed on return");
    drop(sender);
}
