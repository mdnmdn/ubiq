//! The switchboard: who a message reaches, and what attaching and leaving look like to the host.
//!
//! No coordinator and no pseudo-terminal here — just the routing.

use std::time::Duration;

use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::Message;

const PATIENCE: Duration = Duration::from_secs(2);

fn pane(id: PaneId, byte: u8) -> Message {
    Message::TerminalOutput {
        pane_id: id,
        bytes: vec![byte],
    }
}

/// The pane id a `TerminalOutput` carries, so a test can say which message it got.
fn which(message: Message) -> PaneId {
    match message {
        Message::TerminalOutput { pane_id, .. } => pane_id,
        other => panic!("expected output, got {other:?}"),
    }
}

#[test]
fn attaching_and_leaving_are_announced_to_the_host() {
    let (hub, host) = bus::hub();

    let client = hub.connect();
    let id = client.id();
    assert!(matches!(host.recv(), Ok(FromClient::Connected(c)) if c == id));
    assert_eq!(host.attached(), vec![id]);

    drop(client);
    assert!(matches!(host.recv(), Ok(FromClient::Gone(c)) if c == id));
    assert!(
        host.attached().is_empty(),
        "a client that left is not attached"
    );
}

#[test]
fn a_message_addressed_to_one_window_reaches_only_that_window() {
    let (hub, host) = bus::hub();
    let a = hub.connect();
    let b = hub.connect();
    let (mine, theirs) = (PaneId::generate(), PaneId::generate());

    host.send(To::Client(a.id()), pane(mine, b'a'));
    host.send(To::Client(b.id()), pane(theirs, b'b'));

    assert_eq!(which(a.from_host().recv_timeout(PATIENCE).unwrap()), mine);
    assert_eq!(which(b.from_host().recv_timeout(PATIENCE).unwrap()), theirs);
    assert!(a.from_host().try_recv().is_err(), "a got only its own");
    assert!(b.from_host().try_recv().is_err(), "b got only its own");
}

#[test]
fn a_broadcast_reaches_every_window() {
    let (hub, host) = bus::hub();
    let a = hub.connect();
    let b = hub.connect();
    let id = PaneId::generate();

    // The project family goes this way, so every picker agrees without asking again.
    host.send(To::Everyone, pane(id, b'x'));

    assert_eq!(which(a.from_host().recv_timeout(PATIENCE).unwrap()), id);
    assert_eq!(which(b.from_host().recv_timeout(PATIENCE).unwrap()), id);
}

#[test]
fn what_a_window_says_is_attributed_to_it() {
    let (hub, host) = bus::hub();
    let a = hub.connect();
    let b = hub.connect();
    assert!(matches!(host.recv(), Ok(FromClient::Connected(_))));
    assert!(matches!(host.recv(), Ok(FromClient::Connected(_))));

    b.send(Message::Focus {
        pane_id: PaneId::generate(),
    });

    match host.recv() {
        Ok(FromClient::Said { client, .. }) => assert_eq!(client, b.id()),
        other => panic!("expected b to have spoken, got {other:?}"),
    }
    drop(a);
}

#[test]
fn a_mailbox_for_a_window_that_has_gone_says_so() {
    let (hub, host) = bus::hub();
    let a = hub.connect();
    let id = a.id();
    let mailbox = host.mailbox(To::Client(id));

    assert!(
        mailbox.send(pane(PaneId::generate(), b'1')),
        "still attached"
    );

    // This is what stops a pane's reader thread once nothing is left to draw it.
    drop(a);
    assert!(
        !mailbox.send(pane(PaneId::generate(), b'2')),
        "a mailbox for a departed window must report the failure"
    );
}

#[test]
fn a_broadcast_with_nobody_attached_is_not_a_failure() {
    let (hub, host) = bus::hub();
    let mailbox = host.mailbox(To::Everyone);
    drop(hub);

    // Nothing is listening, but that is not a reason for a producer to stop.
    assert!(mailbox.send(pane(PaneId::generate(), b'0')));
}

#[test]
fn the_host_ends_when_the_hub_and_every_client_have_gone() {
    let (hub, host) = bus::hub();
    let client = hub.connect();
    drop(hub);
    drop(client);

    // Connected, then Gone, then the end of the stream — which is what stops the host's thread.
    assert!(matches!(host.recv(), Ok(FromClient::Connected(_))));
    assert!(matches!(host.recv(), Ok(FromClient::Gone(_))));
    assert!(
        host.recv().is_err(),
        "the host's inbox closes behind the last sender"
    );
}
