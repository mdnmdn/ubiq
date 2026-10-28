//! Forwarded ports over the tunnel channel (`D227`): what speaks [`ubiq_proto::tunnel`].
//!
//! Three pieces, each usable alone:
//!
//! - [`Mux`] runs over any duplex — a pair of `Read` / `Write` halves: a Unix socket the `--rdp`
//!   child bridges to the `ubiq::tunnel::v1` virtual channel, a second `ssh` exec's stdio. One
//!   reader thread demultiplexes frames into streams, one writer thread drains two queues (control
//!   frames first, then data). Each stream is credit-limited by its window, the link by
//!   [`ubiq_proto::tunnel::TUNNEL_LINK_IN_FLIGHT`], and the sending side by a token bucket at the
//!   lower of the two ends' `Hello.rate_bps`.
//! - [`TunnelManager`] is the window's half: a loopback listener per enabled
//!   [`ubiq_proto::settings::TunnelSpec`], each accepted socket an `Open` on the attached mux. The
//!   listeners outlive the link — with none attached they refuse, and [`TunnelManager::attach`]
//!   puts a new one under them.
//! - [`serve`] is the drone's half: dial what an `Open` names, if the [`AllowList`] admits it.
//!
//! Plain threads throughout, two per stream and one per listener: nothing on either end runs an
//! async runtime, and a tunnel is a handful of sockets, not thousands.

mod http;
mod local;
mod mux;
mod pipe;
mod rate;
mod remote;
mod tls;

pub use local::{TunnelManager, TunnelStatus};
pub use mux::{Incoming, Mux, Recv, TunnelStream};
pub use remote::{AllowList, serve};
