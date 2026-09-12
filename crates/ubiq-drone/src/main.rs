//! The drone's argv, and nothing else.
//!
//! Parsed by hand, as `ubiq-app` parses its own: the lean tree carries no argument parser, and a
//! binary whose whole surface is four flags does not need one brought in for it.

use std::path::PathBuf;

use ubiq_drone::relay::Relay;

const USAGE: &str = "\
ubiq-drone — one machine's terminal, files and facts, over one duplex byte stream

    --stdio            serve one session on standard input and output (the default)
    --root <path>      add a project at <path>; may be given more than once
    --probe            print this machine's facts and exit
    --version          print the version and exit
";

fn main() {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            // The default, so naming it changes nothing. It exists so a caller can be explicit,
            // and so a second carrier added later has something to be told apart from.
            "--stdio" => {}
            "--root" => {
                let Some(path) = args.next() else {
                    eprintln!("ubiq-drone: --root wants a path");
                    std::process::exit(2);
                };
                roots.push(PathBuf::from(path));
            }
            "--probe" => {
                println!("{}", ubiq_drone::probe_line());
                return;
            }
            "--version" => {
                println!("ubiq-drone {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return;
            }
            other => {
                eprintln!("ubiq-drone: {other} is not an option\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    // On standard error, never standard output: the session's frames own standard output, and a
    // log line on it is a corrupt frame.
    ubiq_proto::log::install();
    // A desktop launcher's thin `PATH` breaks every bare-name spawn, and a drone is started by
    // something even thinner than a launcher. Before any thread exists, for the reason
    // `ubiq_host::shells::repair_path` gives.
    ubiq_host::shells::repair_path();

    // A refused handshake is not a crash and not a session: nothing was spawned, so there is
    // nothing to unwind. It is reported on standard error — the frames own standard output — and
    // the exit code is what a deployer watching the `ssh` sees.
    if let Err(refusal) = ubiq_drone::carrier::serve_stdio(Relay::new(roots)) {
        eprintln!("ubiq-drone: {refusal}");
        std::process::exit(1);
    }
}
