//! Every saved target's tunnels, keyed by a string the carrier chooses (`D227`): one
//! [`TunnelManager`] per key, process-wide, whatever opens the channel under it.
//!
//! - **Link.** [`Registry::link`] runs the carrier's dial on a thread of its own ([`Mux::window`]
//!   blocks for the hello). The first link for a key starts the manager — a loopback listener per
//!   enabled spec; a later one with the same specs and rate is [`TunnelManager::attach`]ed under
//!   the listeners already bound. Different specs or rate replace the manager. With `redial`, the
//!   thread stays and dials again, with a capped backoff, whenever the link dies on its own.
//! - **Down.** [`Registry::down`] detaches the link; the listeners stay and refuse until the next.
//! - **Reconfigure.** [`Registry::reconfigure`] drops a manager whose specs or rate no longer match,
//!   which closes its link; the next link starts over.
//! - **Forget.** [`Registry::forget`] stops listening — the host was disconnected or deleted.
//!
//! **Every link, down, reconfigure and forget bumps the key's generation**, and a dial lands only
//! under the generation it started with: an older dial that finishes after a newer one, or after a
//! forget, closes its link instead of attaching it. A manager being dropped is stopped on a thread
//! of its own, so no caller waits for its listeners to let go.
//!
//! The carrier is only the dial: an RDP virtual channel bridged to a Unix socket, a second `ssh`
//! exec. Nothing here knows which.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use ubiq_proto::settings::TunnelSpec;

use crate::local::{TunnelManager, TunnelStatus};
use crate::mux::Mux;

/// How often a redialling link looks at whether its link is still up.
const WATCH: Duration = Duration::from_millis(500);
/// The redial backoff: doubling from the first, capped at the last.
const BACKOFF_FIRST: Duration = Duration::from_secs(2);
const BACKOFF_CAP: Duration = Duration::from_secs(60);

/// What a target's row shows: whether a link is up, each enabled spec's state, a link error.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub linked: bool,
    pub status: Vec<TunnelStatus>,
    pub error: Option<String>,
}

/// A carrier's dial, callable again for a redial.
pub type Dial = Arc<dyn Fn() -> Result<Mux, String> + Send + Sync>;

struct Entry {
    specs: Vec<TunnelSpec>,
    rate: Option<u64>,
    manager: TunnelManager,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    /// The last link failure per key — a dial or a hello — until the next link succeeds.
    errors: HashMap<String, String>,
    generations: HashMap<String, u64>,
}

impl Inner {
    fn bump(&mut self, key: &str) -> u64 {
        let g = self.generations.entry(key.to_string()).or_default();
        *g += 1;
        *g
    }

    fn current(&self, key: &str, generation: u64) -> bool {
        self.generations.get(key).copied().unwrap_or(0) == generation
    }
}

/// The managers, by key. [`global`] is the process's; a test makes its own.
#[derive(Default)]
pub struct Registry {
    inner: Mutex<Inner>,
}

static GLOBAL: LazyLock<Registry> = LazyLock::new(Registry::default);

/// The process's registry: every window's tunnels, under the keys their carriers chose.
pub fn global() -> &'static Registry {
    &GLOBAL
}

/// Stop a manager off the caller's thread: [`TunnelManager::stop`] joins its accept loops.
fn drop_later(entries: Vec<Entry>) {
    if entries.is_empty() {
        return;
    }
    let spawned = thread::Builder::new()
        .name("ubiq-tunnel-stop".into())
        .spawn(move || drop(entries));
    if let Err(error) = spawned {
        tracing::warn!(target: "tunnel", "the tunnel stop was not started: {error}");
    }
}

impl Registry {
    /// The state of `key`'s tunnels, or `None` when it has neither a manager nor an error.
    pub fn live(&self, key: &str) -> Option<Live> {
        let inner = self.inner.lock();
        let error = inner.errors.get(key).cloned();
        match inner.entries.get(key) {
            Some(e) => Some(Live {
                linked: e.manager.linked(),
                status: e.manager.status(),
                error,
            }),
            None => error.map(|error| Live {
                error: Some(error),
                ..Live::default()
            }),
        }
    }

    /// Dial on a thread of its own and put the link under `key`'s tunnels, superseding any link
    /// still in flight for it. With `redial`, keep the link up: when it dies on its own — or a dial
    /// fails — dial again after a doubling backoff, until a later call for `key` supersedes it.
    pub fn link(
        &'static self,
        key: String,
        specs: Vec<TunnelSpec>,
        rate: Option<u64>,
        dial: Dial,
        redial: bool,
    ) {
        let generation = self.inner.lock().bump(&key);
        let spawned = thread::Builder::new()
            .name("ubiq-tunnel-dial".into())
            .spawn(move || self.keep(key, specs, rate, generation, dial, redial));
        if let Err(error) = spawned {
            tracing::warn!(target: "tunnel", "the tunnel dial was not started: {error}");
        }
    }

    fn keep(
        &self,
        key: String,
        specs: Vec<TunnelSpec>,
        rate: Option<u64>,
        generation: u64,
        dial: Dial,
        redial: bool,
    ) {
        let mut attempt = 0u32;
        loop {
            let linked = self.land(&key, &specs, rate, generation, dial());
            if !redial || !self.is_current(&key, generation) {
                return;
            }
            if linked {
                attempt = 0;
                while self.is_current(&key, generation) && self.is_linked(&key) {
                    thread::sleep(WATCH);
                }
            } else {
                attempt = attempt.saturating_add(1);
            }
            let delay = BACKOFF_FIRST
                .saturating_mul(1u32 << attempt.min(5))
                .min(BACKOFF_CAP);
            let until = std::time::Instant::now() + delay;
            while std::time::Instant::now() < until {
                if !self.is_current(&key, generation) {
                    return;
                }
                thread::sleep(WATCH);
            }
            if !self.is_current(&key, generation) {
                return;
            }
        }
    }

    fn is_current(&self, key: &str, generation: u64) -> bool {
        self.inner.lock().current(key, generation)
    }

    fn is_linked(&self, key: &str) -> bool {
        self.inner
            .lock()
            .entries
            .get(key)
            .is_some_and(|e| e.manager.linked())
    }

    /// A dial's outcome, for the generation it started under: attach it, start a manager, or file
    /// the error. A later link, down, reconfigure or forget since then closes the link instead.
    /// Answers whether the link was put under the tunnels.
    fn land(
        &self,
        key: &str,
        specs: &[TunnelSpec],
        rate: Option<u64>,
        generation: u64,
        dialled: Result<Mux, String>,
    ) -> bool {
        let mut inner = self.inner.lock();
        let mux = match dialled {
            Ok(mux) if inner.current(key, generation) => mux,
            Ok(mux) => {
                drop(inner);
                mux.close();
                return false;
            }
            Err(error) => {
                if inner.current(key, generation) {
                    tracing::warn!(target: "tunnel", "the tunnel channel was not linked: {error}");
                    inner.errors.insert(key.to_string(), error);
                }
                return false;
            }
        };
        inner.errors.remove(key);
        if let Some(e) = inner.entries.get(key)
            && e.specs == specs
            && e.rate == rate
        {
            e.manager.attach(mux);
            return true;
        }
        if let Some(old) = inner.entries.remove(key) {
            // Stopped outside the lock — it joins the accept loops, which frees the ports — and
            // the key looked at again after, since anything may have happened meanwhile.
            drop(inner);
            drop(old);
            inner = self.inner.lock();
            if !inner.current(key, generation) || inner.entries.contains_key(key) {
                drop(inner);
                mux.close();
                return false;
            }
        }
        let manager = TunnelManager::start(specs, Some(mux));
        inner.entries.insert(
            key.to_string(),
            Entry {
                specs: specs.to_vec(),
                rate,
                manager,
            },
        );
        true
    }

    /// A link that cannot be opened at all — the far end serves no tunnels: no manager, just the
    /// sentence, superseding whatever was in flight.
    pub fn fail(&self, key: &str, error: String) {
        let old = {
            let mut inner = self.inner.lock();
            inner.bump(key);
            inner.errors.insert(key.to_string(), error);
            inner.entries.remove(key)
        };
        drop_later(old.into_iter().collect());
    }

    /// The carrier's link went down: the listeners stay, refusing, until the next link, and a
    /// redial or a dial in flight stops.
    pub fn down(&self, key: &str) {
        let mut inner = self.inner.lock();
        inner.bump(key);
        if let Some(e) = inner.entries.get(key) {
            e.manager.detach();
        }
    }

    /// The target was saved: with `keep` false, or a change to its specs or its rate, drop the
    /// manager and its link. Answers whether it did.
    pub fn reconfigure(
        &self,
        key: &str,
        specs: &[TunnelSpec],
        rate: Option<u64>,
        keep: bool,
    ) -> bool {
        let stale = {
            let mut inner = self.inner.lock();
            let stale = inner
                .entries
                .get(key)
                .is_some_and(|e| !keep || e.specs != specs || e.rate != rate);
            if stale {
                inner.bump(key);
                inner.entries.remove(key)
            } else {
                None
            }
        };
        let dropped = stale.is_some();
        drop_later(stale.into_iter().collect());
        dropped
    }

    /// Stop listening and forget the error: the host was disconnected or deleted.
    pub fn forget(&self, key: &str) {
        let old = {
            let mut inner = self.inner.lock();
            inner.bump(key);
            inner.errors.remove(key);
            inner.entries.remove(key)
        };
        drop_later(old.into_iter().collect());
    }

    /// [`Self::forget`] every key that starts with `prefix` — a window's, when it closes.
    pub fn forget_prefix(&self, prefix: &str) {
        let old = {
            let mut inner = self.inner.lock();
            let keys: Vec<String> = inner
                .generations
                .keys()
                .chain(inner.entries.keys())
                .chain(inner.errors.keys())
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect();
            let mut old = Vec::new();
            for key in keys {
                inner.bump(&key);
                inner.errors.remove(&key);
                old.extend(inner.entries.remove(&key));
            }
            old
        };
        drop_later(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Shutdown, TcpListener, TcpStream};
    use ubiq_proto::settings::TunnelKind;

    fn pair() -> (Mux, Mux) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (b, _) = l.accept().unwrap();
        let shut = |s: &TcpStream| -> Option<Box<dyn FnOnce() + Send>> {
            let s = s.try_clone().unwrap();
            Some(Box::new(move || {
                let _ = s.shutdown(Shutdown::Both);
            }))
        };
        let cb = shut(&b);
        let drone = thread::spawn(move || Mux::drone(b.try_clone().unwrap(), b, cb).unwrap());
        let ca = shut(&a);
        let window = Mux::window(a.try_clone().unwrap(), a, None, ca).unwrap();
        (window, drone.join().unwrap())
    }

    fn spec(port: u16) -> TunnelSpec {
        TunnelSpec {
            id: "t".into(),
            kind: TunnelKind::Tcp,
            local_port: port,
            host: "127.0.0.1".into(),
            port: 1,
            upstream_tls: false,
            insecure: false,
            enabled: true,
            rewrite_host: true,
            host_rewrite: None,
            rewrite_content: false,
            content_types: Vec::new(),
        }
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn generation(reg: &Registry, key: &str) -> u64 {
        reg.inner.lock().bump(key)
    }

    #[test]
    fn link_down_relink_reconfigure_forget() {
        let reg = Registry::default();
        let specs = vec![spec(free_port())];
        let (window, _drone) = pair();
        let g = generation(&reg, "k");
        assert!(reg.land("k", &specs, None, g, Ok(window)));
        let live = reg.live("k").unwrap();
        assert!(live.linked && live.status[0].listening);

        reg.down("k");
        let live = reg.live("k").unwrap();
        assert!(!live.linked && live.status[0].listening, "listeners outlive the link");

        let (window, _drone2) = pair();
        let g = generation(&reg, "k");
        assert!(reg.land("k", &specs, None, g, Ok(window)));
        assert!(reg.live("k").unwrap().linked, "a relink attaches under the listeners");

        assert!(!reg.reconfigure("k", &specs, None, true), "nothing changed");
        assert!(reg.reconfigure("k", &specs, Some(1), true), "a new rate drops it");
        assert!(reg.live("k").is_none());

        let g = generation(&reg, "k");
        reg.land("k", &specs, None, g, Err("no route".into()));
        assert_eq!(reg.live("k").unwrap().error.as_deref(), Some("no route"));

        reg.forget("k");
        assert!(reg.live("k").is_none());
    }

    #[test]
    fn a_superseded_dial_lands_on_nothing() {
        let reg = Registry::default();
        let specs = vec![spec(free_port())];
        let older = generation(&reg, "k");
        let newer = generation(&reg, "k");
        let (window, _drone) = pair();
        assert!(reg.land("k", &specs, None, newer, Ok(window)));
        let (late, _drone2) = pair();
        let stale = late.clone();
        assert!(!reg.land("k", &specs, None, older, Ok(late)));
        assert!(!stale.is_alive(), "the older dial's link is closed");
        assert!(reg.live("k").unwrap().linked, "the newer one stays");

        // A forget supersedes a dial in flight too.
        let g = generation(&reg, "k");
        reg.forget("k");
        let (late, _drone3) = pair();
        let stale = late.clone();
        assert!(!reg.land("k", &specs, None, g, Ok(late)));
        assert!(reg.live("k").is_none() && !stale.is_alive());
    }

    #[test]
    fn forget_prefix_takes_only_its_keys() {
        let reg = Registry::default();
        let specs = vec![spec(free_port())];
        let (window, _drone) = pair();
        let g = generation(&reg, "w1:a");
        reg.land("w1:a", &specs, None, g, Ok(window));
        let g = generation(&reg, "w2:a");
        reg.land("w2:a", &[], None, g, Err("x".into()));
        reg.forget_prefix("w1:");
        assert!(reg.live("w1:a").is_none());
        assert!(reg.live("w2:a").is_some());
    }
}
