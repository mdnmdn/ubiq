//! The log sink: the one place every subsystem's diagnostics land, and the one the log console
//! reads.
//!
//! Collection goes through `tracing`, so a subsystem logs with `tracing::info!` and nothing else —
//! no registration, no sink to thread through a signature, and a crate that has never heard of
//! Ubiq is collected on the same terms as Ubiq's own modules. [`install`] puts a layer on the
//! global subscriber that classifies an event by its target, stamps it, and pushes it into a ring
//! the whole process shares.
//!
//! Records travel one way. A producer writes and never reads; the window reads and never writes
//! anything a producer can see. Nothing here holds a pane's state, a path or a descriptor, which
//! is why a sink shared by both halves is not a way around the bus — see `D24`.

use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use parking_lot::Mutex;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;

/// How many records the ring keeps. The oldest goes when the next one arrives, and the count of
/// what went is kept, because a console that silently loses its beginning is a console that lies.
pub const CAPACITY: usize = 5_000;

/// What is collected when `RUST_LOG` says nothing: Ubiq's own subsystems and the harness library
/// down to debug, everything else only when it complains.
pub const DEFAULT_FILTER: &str = "ubiq=debug,ubiq_app=debug,ubiq_host=debug,ubiq_proto=debug,agent_manager=debug,\
     gpui_terminal=debug,warn";

// ── What a record says ──────────────────────────────────────────────

/// The part of the application a record came from. It is derived from the emitting module's path,
/// so nothing has to declare itself and a crate outside the workspace still lands somewhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subsystem {
    /// The window: `AppState`, the screens under `ui/`, the state they draw, and the emulator.
    Ui,
    /// The coordinator and the bus it answers.
    Coordinator,
    /// Pseudo-terminals: the one place a descriptor or a process is held.
    Pty,
    /// The embedded harness library.
    Harness,
    /// The MCP surface Ubiq exposes to the agents it hosts.
    Mcp,
    /// Everything else that logs: the framework, and the crates under it.
    External,
}

impl Subsystem {
    /// Every subsystem, in the order the selector lists them.
    pub const ALL: [Subsystem; 6] = [
        Subsystem::Ui,
        Subsystem::Coordinator,
        Subsystem::Pty,
        Subsystem::Harness,
        Subsystem::Mcp,
        Subsystem::External,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Subsystem::Ui => "UI",
            Subsystem::Coordinator => "Coordinator",
            Subsystem::Pty => "PTY",
            Subsystem::Harness => "Harness",
            Subsystem::Mcp => "MCP",
            Subsystem::External => "External",
        }
    }

    /// Which subsystem a `tracing` target belongs to.
    ///
    /// A target is the emitting module's path, so it begins with the crate the record came from.
    /// The more specific prefixes are tested first, because `ubiq_host::pty` is also `ubiq_host`,
    /// and the bare `ubiq` arm is last because every crate here starts with it.
    fn of(target: &str) -> Subsystem {
        if target.starts_with("ubiq_host::pty") {
            Subsystem::Pty
        } else if target.starts_with("ubiq_host::coordinator")
            || target.starts_with("ubiq_proto::bus")
        {
            Subsystem::Coordinator
        } else if target.starts_with("ubiq_host::mcp_server") {
            Subsystem::Mcp
        } else if target.starts_with("ubiq") || target.starts_with("gpui_terminal") {
            Subsystem::Ui
        } else if target.starts_with("agent_manager") {
            Subsystem::Harness
        } else {
            Subsystem::External
        }
    }
}

/// How loud a record is. Ordered, so the console's filter is a floor rather than a set.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Every level, quietest first — which is the order a floor is chosen in.
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Trace,
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Warn,
        LogLevel::Error,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }

    fn of(level: &Level) -> LogLevel {
        match *level {
            Level::TRACE => LogLevel::Trace,
            Level::DEBUG => LogLevel::Debug,
            Level::INFO => LogLevel::Info,
            Level::WARN => LogLevel::Warn,
            Level::ERROR => LogLevel::Error,
        }
    }
}

/// One thing a subsystem said.
#[derive(Clone, Debug)]
pub struct LogRecord {
    /// Monotonic across the process, so a row has an identity the ring cannot reuse.
    pub seq: u64,
    pub at: SystemTime,
    pub level: LogLevel,
    pub subsystem: Subsystem,
    /// The emitting module's path, as `tracing` reports it.
    pub target: String,
    /// The event's message, with any other fields appended as `key=value`.
    pub message: String,
}

impl LogRecord {
    /// The wall-clock time the console prints, in the reader's own zone.
    pub fn time(&self) -> String {
        chrono::DateTime::<chrono::Local>::from(self.at)
            .format("%H:%M:%S%.3f")
            .to_string()
    }
}

/// What a console is asking for: one subsystem or all of them, from a level upward.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Filter {
    /// `None` is every subsystem at once.
    pub subsystem: Option<Subsystem>,
    pub min_level: LogLevel,
}

impl Filter {
    fn accepts(&self, record: &LogRecord) -> bool {
        record.level >= self.min_level
            && self
                .subsystem
                .is_none_or(|subsystem| subsystem == record.subsystem)
    }
}

// ── The ring ────────────────────────────────────────────────────────

/// The process-wide ring, and the windows waiting to hear that it changed.
pub struct Logs {
    inner: Mutex<Inner>,
}

struct Inner {
    records: VecDeque<Arc<LogRecord>>,
    next_seq: u64,
    /// How many records the ring has dropped off its front since the last clear.
    dropped: u64,
    /// The loudest level in the ring since the last clear. It is what the dock's tab reports, so
    /// a warning is visible without the console being the tab on screen.
    loudest: Option<LogLevel>,
    /// One per console. The message is a nudge, not a record — the reader takes what it wants
    /// from the ring, so a listener that misses a nudge misses nothing.
    listeners: Vec<flume::Sender<()>>,
}

impl Logs {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                records: VecDeque::with_capacity(256),
                next_seq: 0,
                dropped: 0,
                loudest: None,
                listeners: Vec::new(),
            }),
        }
    }

    /// Append a record, dropping the oldest if the ring is full.
    fn push(&self, level: LogLevel, subsystem: Subsystem, target: String, message: String) {
        let mut inner = self.inner.lock();
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.records.push_back(Arc::new(LogRecord {
            seq,
            at: SystemTime::now(),
            level,
            subsystem,
            target,
            message,
        }));
        inner.loudest = Some(inner.loudest.map_or(level, |worst| worst.max(level)));
        while inner.records.len() > CAPACITY {
            inner.records.pop_front();
            inner.dropped += 1;
        }
        inner.wake();
    }

    /// The records a console wants, oldest first. Records are shared rather than copied, so a
    /// snapshot costs a pointer each and the ring is never held across a frame.
    pub fn snapshot(&self, filter: Filter) -> Vec<Arc<LogRecord>> {
        let inner = self.inner.lock();
        inner
            .records
            .iter()
            .filter(|record| filter.accepts(record))
            .cloned()
            .collect()
    }

    /// How many records the ring holds, and how many it has dropped off its front.
    pub fn counts(&self) -> (usize, u64) {
        let inner = self.inner.lock();
        (inner.records.len(), inner.dropped)
    }

    /// The loudest level the ring holds, which is what a closed console is judged by.
    pub fn loudest(&self) -> Option<LogLevel> {
        self.inner.lock().loudest
    }

    /// Forget everything. The consoles are woken, because what they are showing has just gone.
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.records.clear();
        inner.dropped = 0;
        inner.loudest = None;
        inner.wake();
    }

    /// Ask to be told when a record arrives. Dropping the receiver is how a console unsubscribes.
    pub fn subscribe(&self) -> flume::Receiver<()> {
        let (sender, receiver) = flume::unbounded();
        self.inner.lock().listeners.push(sender);
        receiver
    }
}

impl Inner {
    /// Nudge every console, and forget the ones whose window has gone.
    fn wake(&mut self) {
        self.listeners.retain(|listener| listener.send(()).is_ok());
    }
}

/// The ring every subsystem writes to and every console reads.
pub fn logs() -> &'static Logs {
    static LOGS: OnceLock<Logs> = OnceLock::new();
    LOGS.get_or_init(Logs::new)
}

// ── Collection ──────────────────────────────────────────────────────

/// Install the collector. Called once, before anything that might log.
///
/// Two layers sit behind one filter: the ring the console reads, and a plain writer on standard
/// error, so a run from a terminal still says what it is doing. `RUST_LOG` sets the filter;
/// [`DEFAULT_FILTER`] is what it falls back to.
pub fn install() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::builder().parse_lossy(DEFAULT_FILTER));

    // A second call would be a wiring mistake rather than a condition to handle: the first
    // subscriber stays, and this one is dropped.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(Sink)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(true),
        )
        .try_init();
}

/// The layer that turns a `tracing` event into a record in the ring.
struct Sink;

impl<S: Subscriber> Layer<S> for Sink {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);

        logs().push(
            LogLevel::of(metadata.level()),
            Subsystem::of(metadata.target()),
            metadata.target().to_string(),
            fields.finish(),
        );
    }
}

/// An event's message, plus whatever else it carried.
#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
}

impl Fields {
    fn write(&mut self, name: &str, value: &str) {
        if name == "message" {
            self.message.push_str(value);
        } else {
            if !self.rest.is_empty() {
                self.rest.push(' ');
            }
            let _ = write!(self.rest, "{name}={value}");
        }
    }

    fn finish(self) -> String {
        match (self.message.is_empty(), self.rest.is_empty()) {
            (true, _) => self.rest,
            (false, true) => self.message,
            (false, false) => format!("{} {}", self.message, self.rest),
        }
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.write(field.name(), value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.write(field.name(), &format!("{value:?}"));
    }
}
