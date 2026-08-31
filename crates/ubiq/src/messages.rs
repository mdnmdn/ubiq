/// Transport contract messages
///
/// Defines message types for communication between UI and coordinator:
///
/// Downstream (coordinator → UI):
/// - output{ pane_id, bytes }
/// - exited{ pane_id, code }
///
/// Upstream (UI → coordinator):
/// - input{ pane_id, bytes }
///
/// Control (bidirectional):
/// - spawn{ pane_id, harness, args }
/// - resize{ pane_id, cols, rows }
/// - focus{ pane_id }
///
/// TODO: Define Message enum and serialization
