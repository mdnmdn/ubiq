//! `--tunnel`: the tunnel channel (`D227`) on this process's standard input and output.
//!
//! Over `ssh` the tunnel channel is a second exec beside the bus's, so it is a second process: the
//! window runs `ssh -T host 'exec ubiq-drone --tunnel'` and speaks `TunnelFrame` on its pipes. This
//! process serves the drone's end — dial what an `open` names, if the allow list admits it — until
//! the window closes the link, and then exits. It holds no pane and no project, and nothing here
//! touches the bus.

use std::io::{self, Read, Write};

use ubiq_tunnel::{AllowList, Mux};

/// Parse `--tunnel-allow` values: each a comma-separated list of `host:port` globs. None at all
/// admits everything, as the RDP drone's own default does.
pub fn allow_list(values: &[String]) -> Result<AllowList, String> {
    AllowList::parse(values.iter().flat_map(|value| value.split(',')))
}

/// Serve the drone's end of one tunnel link over `reader` / `writer` until it ends.
pub fn serve<R, W>(reader: R, writer: W, allow: &AllowList) -> io::Result<()>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let mux = Mux::drone(reader, writer, None)?;
    ubiq_tunnel::serve(&mux, allow);
    mux.close();
    Ok(())
}

/// [`serve`] on standard input and output.
pub fn serve_stdio(allow: &AllowList) -> io::Result<()> {
    serve(io::stdin(), io::stdout(), allow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_values_split_on_commas_and_default_to_any() {
        let allow = allow_list(&["localhost:*,db:5432".into(), "[::1]:80".into()]).unwrap();
        assert!(allow.allows("localhost", 1) && allow.allows("db", 5432));
        assert!(allow.allows("::1", 80) && !allow.allows("db", 5433));
        assert_eq!(allow_list(&[]).unwrap(), AllowList::any());
        assert!(allow_list(&["nope".into()]).is_err());
    }
}
