//! The drone's half: dial what an `open` names, and splice it.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::thread;
use std::time::Duration;

use crate::mux::{Incoming, Mux};
use crate::pipe;

/// How long a dial may take before the stream is refused.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Which `host:port` pairs a drone dials. Any, by default.
///
/// A pattern is `host:port`, each side a glob where `*` matches any run of characters; hosts
/// compare case-insensitively, and an IPv6 host is written bracketed, `[::1]:5432`. `*` alone
/// admits everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowList {
    /// `None` admits everything.
    rules: Option<Vec<(String, String)>>,
}

impl AllowList {
    pub fn any() -> Self {
        Self::default()
    }

    /// Parse patterns; blank ones are skipped, and no patterns at all is [`Self::any`].
    pub fn parse<I, S>(patterns: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut rules = Vec::new();
        for pattern in patterns {
            let pattern = pattern.as_ref().trim();
            if pattern.is_empty() {
                continue;
            }
            if pattern == "*" {
                return Ok(Self::any());
            }
            let Some((host, port)) = pattern.rsplit_once(':') else {
                return Err(format!("`{pattern}` is not host:port"));
            };
            let host = host.trim_start_matches('[').trim_end_matches(']');
            if host.is_empty() || port.is_empty() {
                return Err(format!("`{pattern}` is not host:port"));
            }
            rules.push((host.to_ascii_lowercase(), port.to_string()));
        }
        Ok(Self {
            rules: if rules.is_empty() { None } else { Some(rules) },
        })
    }

    pub fn allows(&self, host: &str, port: u16) -> bool {
        let Some(rules) = &self.rules else {
            return true;
        };
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let port = port.to_string();
        rules.iter().any(|(h, p)| {
            glob(h.as_bytes(), host.as_bytes()) && glob(p.as_bytes(), port.as_bytes())
        })
    }
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|i| glob(rest, &text[i..])),
        Some((c, rest)) => text.first() == Some(c) && glob(rest, &text[1..]),
    }
}

/// Serve every `open` on `mux` — a [`Mux::drone`] — until the link ends: refuse what `allow`
/// does not admit, dial the rest with a timeout, and splice each on a thread of its own.
pub fn serve(mux: &Mux, allow: &AllowList) {
    let Some(incoming) = mux.incoming() else {
        tracing::warn!(target: "tunnel", "serve was handed the window's end of a tunnel");
        return;
    };
    for open in incoming.iter() {
        let (mux, allow) = (mux.clone(), allow.clone());
        let _ = thread::Builder::new()
            .name("ubiq-tunnel-dial".into())
            .spawn(move || dial_and_pipe(&mux, &allow, open));
    }
}

fn dial_and_pipe(mux: &Mux, allow: &AllowList, open: Incoming) {
    let (host, port) = (open.host.clone(), open.port);
    if !allow.allows(&host, port) {
        open.refuse(format!(
            "{host}:{port} is not on this drone's tunnel allow list"
        ));
        return;
    }
    match dial(&host, port) {
        Ok(socket) => {
            let _ = socket.set_nodelay(true);
            let stream = open.accept();
            pipe::pipe(mux, socket, stream, None, None);
        }
        Err(e) => open.refuse(format!("could not reach {host}:{port}: {e}")),
    }
}

fn dial(host: &str, port: u16) -> io::Result<TcpStream> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let mut last = io::Error::new(io::ErrorKind::NotFound, "the name resolved to no address");
    for addr in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(socket) => return Ok(socket),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_glob_each_side() {
        let allow = AllowList::parse(["localhost:*", "*.internal:5432", "[::1]:80", " "]).unwrap();
        assert!(allow.allows("LOCALHOST", 9000));
        assert!(allow.allows("db.internal", 5432));
        assert!(!allow.allows("db.internal", 5433));
        assert!(allow.allows("::1", 80));
        assert!(!allow.allows("example.com", 80));
        assert!(AllowList::parse(["*"]).unwrap().allows("anything", 1));
        assert!(
            AllowList::parse(Vec::<String>::new())
                .unwrap()
                .allows("anything", 1)
        );
        assert!(AllowList::parse(["no-port"]).is_err());
    }
}
