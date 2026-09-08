//! Telling the desktop, on the one path that leaves Ubiq.
//!
//! This is the only thing in the notification family the user can see without Ubiq's window in
//! front of them, which is why nothing reaches it unless a request asked for it with `os: true`
//! and no mute rule silenced the result.
//!
//! `notify-rust` is the whole implementation: it is the Freedesktop D-Bus service on Linux and
//! the BSDs, `NSUserNotification` on macOS, and a WinRT toast on Windows. Ubiq writes no platform
//! code of its own here, and a platform the crate does not cover simply logs and returns — a
//! desktop that cannot be told is never an error the user has to read.

use ubiq_proto::notifications::{Family, Level};

/// The application name the desktop shows beside the notification.
const APP: &str = "Ubiq";

/// Hand one notification to the operating system, off this thread.
///
/// Fire and forget, on a thread of its own, because showing one is a round trip to a desktop
/// service — a D-Bus call on Linux — and the coordinator's thread is the thread that drains every
/// pseudo-terminal. Blocking it to draw a toast would stall a harness, which is the one thing the
/// host may never do.
pub fn post(level: Level, family: Family, text: &str) {
    let summary = format!("{} · {}", family.label(), level.label());
    let body = text.to_string();

    std::thread::Builder::new()
        .name("ubiq-os-notify".to_string())
        .spawn(move || show(&summary, &body))
        .map(|_| ())
        .unwrap_or_else(|error| {
            tracing::warn!("could not spawn the OS notification thread: {error}");
        });
}

fn show(summary: &str, body: &str) {
    match notify_rust::Notification::new()
        .appname(APP)
        .summary(summary)
        .body(body)
        .show()
    {
        Ok(_) => tracing::debug!("told the desktop: {summary}"),
        // The desktop having no notification service — a bare X session, a locked-down Windows —
        // is ordinary, not a fault of Ubiq's. The bell in the window has the same text either way.
        Err(error) => tracing::debug!("the desktop would not take a notification: {error}"),
    }
}
