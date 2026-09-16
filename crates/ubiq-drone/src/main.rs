//! The drone's argv, and nothing else.
//!
//! Parsed by hand, as `ubiq-app` parses its own: the lean tree carries no argument parser, and a
//! binary whose whole surface is a handful of flags does not need one brought in for it.

use std::path::PathBuf;
use std::str::FromStr;

use ubiq_drone::linger::Linger;
use ubiq_drone::relay::{Relay, Root};
use ubiq_drone::socket;
use ubiq_proto::ids::ProjectId;

const USAGE: &str = "\
ubiq-drone — one machine's terminal, files and facts, over one duplex byte stream

    --stdio            serve one session on standard input and output (the default)
    --listen [<sock>]  bind a unix socket, detach, and hold the panes across links;
                       without a path the socket is derived from the roots
    --attach [<sock>]  relay this process's standard input and output to a listening drone
    --foreground       with --listen: serve here rather than handing the process to a
                       multiplexer. What the multiplexer itself runs.
    --linger <spec>    how long a drone with no client waits: 0, a number of seconds
                       (10m, 2h), or never. The default is 10m, and an attach re-asserts it
    --root <path>      add a project at <path>; may be given more than once
    --root-id <ulid>   bind the preceding --root to this id, so the interface sees one project
                       rather than two; without it the drone mints a fresh one, as it always has
    --list             print every running drone this machine knows of, as JSON, and exit
    --status [<sock>]  print one running drone's state as JSON; without a path the socket
                       is derived from the roots, as --attach does. Exits 1 if none is there
    --stop [<sock>]    ask a listening drone to end; same path rule as --status
    --probe            print this machine's facts and exit
    --version          print the version and exit
";

/// Which of the three things this process is.
enum Mode {
    Stdio,
    /// Bind and serve. `held` is true in the process a multiplexer or `setsid` is holding — the
    /// one that actually owns the socket — and false in the one that has to arrange for it.
    Listen {
        path: Option<PathBuf>,
        held: bool,
    },
    Attach {
        path: Option<PathBuf>,
    },
    /// Every running drone this machine knows of, as JSON.
    List,
    /// One running drone's state, as JSON.
    Status {
        path: Option<PathBuf>,
    },
    /// Ask a listening drone to end.
    Stop {
        path: Option<PathBuf>,
    },
}

fn main() {
    // Ahead of anything else, including argv parsing: this is what `capabilities()` reports, and
    // it must see this process's own `PATH` before `repair_path` widens it with a login shell's
    // own homes for the sake of spawning a shell by name. Caching happens inside `search::probe`
    // itself, so this is the only call site that matters for what gets remembered.
    ubiq_drone::search::probe();

    let mut roots: Vec<Root> = Vec::new();
    let mut mode = Mode::Stdio;
    let mut linger: Option<Linger> = None;
    let mut held = false;

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut at = 0;
    while at < argv.len() {
        let arg = argv[at].as_str();
        at += 1;
        match arg {
            // The default, so naming it changes nothing. It exists so a caller can be explicit,
            // and so a second carrier added later has something to be told apart from.
            "--stdio" => mode = Mode::Stdio,
            // The path is optional: given, it is used verbatim; omitted, it is derived from the
            // roots, which is what stops two windows starting rival drones for one project.
            "--listen" => {
                mode = Mode::Listen {
                    path: optional_value(&argv, &mut at).map(PathBuf::from),
                    held: false,
                }
            }
            "--attach" => {
                mode = Mode::Attach {
                    path: optional_value(&argv, &mut at).map(PathBuf::from),
                }
            }
            "--list" => mode = Mode::List,
            "--status" => {
                mode = Mode::Status {
                    path: optional_value(&argv, &mut at).map(PathBuf::from),
                }
            }
            "--stop" => {
                mode = Mode::Stop {
                    path: optional_value(&argv, &mut at).map(PathBuf::from),
                }
            }
            "--foreground" => held = true,
            "--linger" => {
                let given = value(&argv, &mut at, arg);
                linger = Some(Linger::parse(&given).unwrap_or_else(|why| {
                    eprintln!("ubiq-drone: --linger {given}: {why}");
                    std::process::exit(2);
                }));
            }
            "--root" => roots.push(Root::new(PathBuf::from(value(&argv, &mut at, arg)))),
            // Binds to whichever --root came immediately before it. An id before any --root, or
            // one that does not parse, is an argv error: there is nothing for it to bind to, and
            // a silently-ignored id would announce a project under an id the interface never
            // asked for.
            "--root-id" => {
                let given = value(&argv, &mut at, arg);
                let Some(root) = roots.last_mut() else {
                    eprintln!("ubiq-drone: --root-id {given}: no --root came before it");
                    std::process::exit(2);
                };
                let id = ProjectId::from_str(&given).unwrap_or_else(|_| {
                    eprintln!("ubiq-drone: --root-id {given}: not a project id");
                    std::process::exit(2);
                });
                root.id = Some(id);
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
    if let Mode::Listen { path, .. } = mode {
        mode = Mode::Listen { path, held };
    }

    // On standard error, never standard output: a session's frames own standard output, and a log
    // line on it is a corrupt frame.
    ubiq_proto::log::install();

    match mode {
        Mode::Stdio => serve_stdio(roots),
        Mode::Attach { path } => {
            let path = path.unwrap_or_else(|| socket::socket_path(&paths(&roots)));
            if let Err(error) = socket::attach(&path, linger) {
                eprintln!("ubiq-drone: attaching to {}: {error}", path.display());
                std::process::exit(1);
            }
        }
        Mode::Listen { path, held } => {
            let path = path.unwrap_or_else(|| socket::socket_path(&paths(&roots)));
            if held {
                listen(roots, &path, linger.unwrap_or_default());
            } else {
                detach(&roots, linger.unwrap_or_default(), &path);
            }
        }
        // `--list` and `--status` print JSON on standard output rather than standard error: they
        // never serve frames, so there is no session output for a log line to corrupt.
        Mode::List => {
            let drones = ubiq_drone::state::list(&socket::state_dir());
            println!(
                "{}",
                serde_json::to_string(&drones).expect("a drone's state serialises")
            );
        }
        Mode::Status { path } => {
            let path = path.unwrap_or_else(|| socket::socket_path(&paths(&roots)));
            match ubiq_drone::state::DroneState::read(&ubiq_drone::state::state_path(&path)) {
                Ok(state) if ubiq_drone::state::is_live(&state) => {
                    println!(
                        "{}",
                        serde_json::to_string(&state).expect("a drone's state serialises")
                    );
                }
                _ => {
                    eprintln!("ubiq-drone: no drone is listening on {}", path.display());
                    std::process::exit(1);
                }
            }
        }
        Mode::Stop { path } => {
            let path = path.unwrap_or_else(|| socket::socket_path(&paths(&roots)));
            match socket::stop(&path) {
                Ok(answer) => println!("{answer}"),
                Err(error) => {
                    eprintln!("ubiq-drone: stopping {}: {error}", path.display());
                    std::process::exit(1);
                }
            }
        }
    }
}

/// One session on this process's standard input and output, and nothing that outlives it.
fn serve_stdio(roots: Vec<Root>) {
    repair_path();
    // A refused handshake is not a crash and not a session: nothing was spawned, so there is
    // nothing to unwind. It is reported on standard error — the frames own standard output — and
    // the exit code is what a deployer watching the `ssh` sees.
    if let Err(refusal) = ubiq_drone::carrier::serve_stdio(Relay::new(roots)) {
        eprintln!("ubiq-drone: {refusal}");
        std::process::exit(1);
    }
}

/// Bind the socket and serve every attach on it until the linger runs out.
fn listen(roots: Vec<Root>, path: &std::path::Path, linger: Linger) {
    repair_path();
    if let Err(error) = socket::listen(roots, path, linger) {
        eprintln!("ubiq-drone: listening on {}: {error}", path.display());
        std::process::exit(1);
    }
}

/// Hand this same command line to whatever will hold it, and print the socket the held process
/// bound, so the caller — an `ssh` that is about to end — knows what to attach to next.
fn detach(roots: &[Root], linger: Linger, path: &std::path::Path) {
    let exe = std::env::current_exe().unwrap_or_else(|error| {
        eprintln!("ubiq-drone: this drone cannot find its own binary: {error}");
        std::process::exit(1);
    });
    // Built from what was parsed rather than passed through: the held process is given the socket
    // already resolved, so it cannot derive a different one from a different environment.
    let mut held = vec![
        "--listen".to_string(),
        path.to_string_lossy().to_string(),
        "--foreground".to_string(),
        "--linger".to_string(),
        linger.to_string(),
    ];
    for root in roots {
        held.push("--root".to_string());
        held.push(root.path.to_string_lossy().to_string());
        // Forwarded right after its --root, exactly as it was given: the held process must
        // announce the same ids the attached one did, or a reattach would look like a second
        // project to the interface rather than the same one coming back.
        if let Some(id) = root.id {
            held.push("--root-id".to_string());
            held.push(id.to_string());
        }
    }

    if let Err(error) = socket::hold(&exe, &held, path) {
        eprintln!("ubiq-drone: detaching: {error}");
        std::process::exit(1);
    }
    println!("{}", path.display());
}

/// The bare paths behind a set of roots, for the socket derivation that only ever needs those —
/// `socket_path` and `DroneState::new` hash and record the folders, never the ids bound to them.
fn paths(roots: &[Root]) -> Vec<PathBuf> {
    roots.iter().map(|root| root.path.clone()).collect()
}

/// A desktop launcher's thin `PATH` breaks every bare-name spawn, and a drone is started by
/// something even thinner than a launcher. Before any thread exists, for the reason
/// `ubiq_host::shells::repair_path` gives.
fn repair_path() {
    ubiq_host::shells::repair_path();
}

/// The value after a flag that must have one.
fn value(argv: &[String], at: &mut usize, flag: &str) -> String {
    let Some(given) = argv.get(*at) else {
        eprintln!("ubiq-drone: {flag} wants a value");
        std::process::exit(2);
    };
    *at += 1;
    given.clone()
}

/// The value after a flag that may have one: the next word, unless it is another flag.
fn optional_value(argv: &[String], at: &mut usize) -> Option<String> {
    let given = argv.get(*at)?;
    if given.starts_with('-') {
        return None;
    }
    *at += 1;
    Some(given.clone())
}
