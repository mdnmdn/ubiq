//! Run a harness's real confined login in a pseudoconsole and show what it says.
//!
//! When a confined login pane opens and closes at once, the host reports only
//! what it can see afterwards — "the login wrote no credential, so nothing was
//! captured" — and the harness's own words went to a terminal that no longer
//! exists. isol8 writes no denial log on Windows either (`G289`), so a policy
//! refusal looks the same as a harness that simply declined.
//!
//! This runs the same three calls `Agents::begin_login` makes — `Harness::login`,
//! `isolate::login_confined`, `isolate::confined_launch` — against a scratch home,
//! puts the result on a real ConPTY, and streams every byte it produces. Then it
//! reports the exit code and whether the credential the harness promised appeared.
//!
//! The login is interactive: it may wait for a browser. Ctrl-C ends it, and the
//! output up to that point is the diagnosis.
//!
//! Run: `cargo run -p agent-manager --features pty --example login_confined_probe -- claude-code`

use std::io::{Read as _, Write as _};

use agent_manager::harness;
use agent_manager::isolate::{self, IsolateOptions, confine_entrypoint};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 120;
const ROWS: u16 = 30;

fn main() -> anyhow::Result<()> {
    // A confined run is this binary invoked again, so the probe answers that
    // first, exactly as `ubiq.exe` does.
    if let Some(code) = confine_entrypoint() {
        std::process::exit(code);
    }

    let agent = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "claude-code".into());
    let harness =
        harness::resolve(&agent).ok_or_else(|| anyhow::anyhow!("unknown agent type '{agent}'"))?;

    let state = std::env::temp_dir().join("login-confine-probe");
    let home = state.join("home");
    std::fs::create_dir_all(&home)?;

    let plan = harness.login(&home)?;
    println!("harness      : {}", harness.id());
    println!("home         : {}", home.display());
    println!(
        "argv         : {} {:?}",
        plan.launch.program, plan.launch.args
    );
    println!("env          : {:?}", plan.launch.env);
    println!("credential   : {:?}", plan.credential_files);

    let confined = isolate::login_confined(&home, &plan, None, &IsolateOptions::new(&state))?;
    let launch = isolate::confined_launch(&confined)?;
    println!("launch       : {} {:?}\n", launch.program, launch.args);

    let pair = native_pty_system().openpty(PtySize {
        rows: ROWS,
        cols: COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut spawn = CommandBuilder::new(&launch.program);
    for arg in &launch.args {
        spawn.arg(arg);
    }
    let mut child = pair.slave.spawn_command(spawn)?;

    let mut reader = pair.master.try_clone_reader()?;
    drop(pair.slave);
    // Streamed rather than collected: a login that hangs waiting for a browser
    // still shows what it printed before it stopped.
    let pump = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut out = std::io::stdout();
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            let _ = out.write_all(&buf[..n]);
            let _ = out.flush();
        }
    });

    let status = child.wait()?;
    drop(pair.master);
    let _ = pump.join();

    println!("\n=== exit: {status:?} ===");
    for file in &plan.credential_files {
        let path = home.join(file);
        println!(
            "{}: {}",
            file.display(),
            if path.exists() { "written" } else { "absent" }
        );
    }
    Ok(())
}
