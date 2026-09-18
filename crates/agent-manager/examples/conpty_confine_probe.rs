//! Does a confine run confine a harness onto the ConPTY its caller owns?
//!
//! isol8's Windows backend calls `CreateProcessW` with no console creation flag
//! (`backends/windows.rs`: no `CREATE_NEW_CONSOLE`, no `DETACHED_PROCESS`, no
//! `CREATE_NO_WINDOW`, a zeroed `STARTUPINFOW` with no `STARTF_USESTDHANDLES`),
//! so the confined child attaches to its caller's console. Inside a ConPTY that
//! console is the pseudoconsole — which is what lets a confiner in the pane stand in
//! for the seam isol8 does not have.
//!
//! This probe is the evidence, and it is both halves of the mechanism: a confine
//! run is this same binary invoked again under `CONFINE_ARG`, so the probe
//! re-invokes itself the way `ubiq.exe` does. Two runs of the same payload: one
//! on this process's console, one inside a 100x30 ConPTY. The second is
//! the claim — a confined child reporting `size=100x30, redirected=False` is on
//! a real pseudoconsole at the size the host asked for.
//!
//! It is also the regression test for the mismatch that cost the most to find.
//! isol8 grants the *resolving* process's working directory read-write, and a
//! confined child inherits its parent's, so a confiner that runs anywhere else hands
//! the harness a directory its own policy does not grant. The harness then dies
//! at startup with nothing to read: `analyze_denial_log_path` promises a denial
//! log that `isol8-winhook` never writes. `ConfinePayload::cwd` is the fix, and
//! a pane spawned with no explicit directory — which is what this probe does —
//! is exactly the case that exposed it.
//!
//! Run: `cargo run -p agent-manager --features pty --example conpty_confine_probe`

use agent_manager::isolate::{CONFINE_ARG, ConfinePayload, confine_entrypoint};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 100;
const ROWS: u16 = 30;

fn main() -> anyhow::Result<()> {
    // This example is both halves, which is the point: a confine run is the same
    // binary invoked again, so the probe re-invokes itself exactly as `ubiq.exe`
    // and `agent-manager.exe` do.
    if let Some(code) = confine_entrypoint() {
        std::process::exit(code);
    }

    let state = std::env::temp_dir().join("am-confine-probe");
    std::fs::create_dir_all(&state)?;

    let confiner = std::env::current_exe()?;

    let (profile, env, cmd, cwd) = resolved()?;
    let payload = || ConfinePayload {
        profile: profile.clone(),
        env: env.clone(),
        cmd: cmd.clone(),
        cwd: cwd.clone(),
    };

    // The control: same confiner, same payload, inheriting this process's console
    // rather than a ConPTY. Whatever differs between the two runs is the
    // ConPTY's doing and not the policy's.
    println!("=== run A: confiner on this process's console ===");
    let status = std::process::Command::new(&confiner)
        .arg(CONFINE_ARG)
        .arg(payload().write(&state)?)
        .status()?;
    println!("=== run A exit: {status} ===\n");

    let pair = native_pty_system().openpty(PtySize {
        rows: ROWS,
        cols: COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    // Deliberately no `.cwd()`: a pane that names no directory starts in the
    // user's home, and the payload's own `cwd` is what has to correct for that.
    let mut spawn = CommandBuilder::new(&confiner);
    spawn.arg(CONFINE_ARG);
    spawn.arg(payload().write(&state)?);
    let mut child = pair.slave.spawn_command(spawn)?;

    let mut reader = pair.master.try_clone_reader()?;
    // The slave goes now so the read can reach EOF, but the master must outlive
    // the child: dropping it closes the pseudoconsole, which tears the child
    // down mid-startup (STATUS_DLL_INIT_FAILED, 0xC0000142).
    drop(pair.slave);
    let pump = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut reader, &mut buf);
        buf
    });

    let status = child.wait()?;
    drop(pair.master);
    let buf = pump.join().unwrap_or_default();

    println!("=== run B: bytes seen on the {COLS}x{ROWS} ConPTY master ===");
    println!("{}", String::from_utf8_lossy(&buf));
    println!("=== run B exit: {status:?} ===");
    println!("pass: run B reports size={COLS}x{ROWS}, redirected=False, exit 0");

    input_round_trip(&confiner, &state, &profile, &env, &cwd)
}

/// Does a keystroke written to the pty master reach the confined child?
///
/// Run B proves output: the harness draws on the ConPTY the host opened. Output
/// is only half a terminal. isol8 spawns with `bInheritHandles = FALSE` and sets
/// no std handles, so the child takes the console's own — and whether the console
/// hands it what the master writes is a separate fact from whether what it draws
/// comes back, and the one a typist notices first.
fn input_round_trip(
    confiner: &std::path::Path,
    state: &std::path::Path,
    profile: &isol8::Profile,
    env: &[(String, String)],
    cwd: &std::path::Path,
) -> anyhow::Result<()> {
    // `Read-Host` reads the console the way an interactive program does.
    // `[Console]::In.ReadLine()` does not, and a probe that fails unconfined
    // measures nothing: that mistake once cleared the sandbox by accident.
    let script = "$line = Read-Host; Write-Output \"TYPED:$line\"";
    let payload = ConfinePayload {
        profile: profile.clone(),
        env: env.to_vec(),
        cmd: vec![
            "powershell".to_string(),
            "-NoProfile".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ],
        cwd: cwd.to_path_buf(),
    };

    let pair = native_pty_system().openpty(PtySize {
        rows: ROWS,
        cols: COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    // The control the whole claim rests on: the same command, the same ConPTY,
    // spawned directly. If input does not reach an unconfined child either, this
    // is measuring the pseudoconsole rather than the sandbox.
    let mut spawn = if std::env::var_os("PROBE_UNCONFINED").is_some() {
        let mut spawn = CommandBuilder::new(&payload.cmd[0]);
        for arg in &payload.cmd[1..] {
            spawn.arg(arg);
        }
        spawn
    } else {
        let mut spawn = CommandBuilder::new(confiner);
        spawn.arg(CONFINE_ARG);
        spawn.arg(payload.write(state)?);
        spawn
    };
    spawn.cwd(cwd);
    let mut child = pair.slave.spawn_command(spawn)?;

    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    drop(pair.slave);
    let pump = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut reader, &mut buf);
        buf
    });

    // The console needs the child attached and reading before anything typed at
    // it means something.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    // Carriage return, not newline: a console submits a line on CR, and a probe
    // that sends only LF leaves `Read-Host` waiting forever — which reads exactly
    // like input the sandbox swallowed.
    std::io::Write::write_all(&mut writer, b"ubiq-typed-this\r")?;
    std::io::Write::flush(&mut writer)?;

    // Bounded: a child that never sees the input blocks in `ReadLine` forever,
    // and a probe that hangs reports nothing at all.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    drop(pair.master);
    let buf = pump.join().unwrap_or_default();
    let seen = String::from_utf8_lossy(&buf).into_owned();

    println!("\n=== run C: what the confined child read from the ConPTY ===");
    println!("{seen}");
    match status {
        Some(status) => println!("=== run C exit: {status:?} ==="),
        None => println!("=== run C: still blocked in ReadLine after 10s, killed ==="),
    }
    println!(
        "pass: run C echoes TYPED:ubiq-typed-this  —  {}",
        if seen.contains("TYPED:ubiq-typed-this") {
            "PASS"
        } else {
            "FAIL: input never reached the confined child"
        }
    );
    Ok(())
}

/// Resolve a run the way `isolate::confined_launch` does — a relocated home with
/// read-write grants on it and on the working directory, which is the shape
/// `plan` and `login_confined` actually produce.
type Resolved = (
    isol8::Profile,
    Vec<(String, String)>,
    Vec<String>,
    std::path::PathBuf,
);

fn resolved() -> anyhow::Result<Resolved> {
    let home = std::env::temp_dir().join("am-confine-probe/home");
    std::fs::create_dir_all(&home)?;
    let home_str = home.to_string_lossy().into_owned();
    let cwd = std::env::current_dir()?;

    let mut spec = isol8::Spec::default();
    spec.home = Some(home_str.clone());
    spec.add_dirs_rw = vec![home_str, cwd.to_string_lossy().into_owned()];
    spec.profiles = vec!["windows/system-runtime".to_string()];
    spec.cmd = probe_argv();

    // Ambient resolution, as `isol8::Sandbox::run` does it. The probe is about
    // the ConPTY, so it must not introduce a hand-built `Context` as a second
    // variable; the real path resolves against `Confined::ctx`.
    let mut eff = isol8::effective_policy(&spec).map_err(|e| anyhow::anyhow!("resolving: {e}"))?;
    isol8::home::materialize(&eff.home).map_err(|e| anyhow::anyhow!("home: {e}"))?;
    isol8::confine_executable(&mut eff.profile, &mut eff.cmd)
        .map_err(|e| anyhow::anyhow!("confine_executable: {e}"))?;
    Ok((eff.profile, eff.env.into_iter().collect(), eff.cmd, cwd))
}

/// A PowerShell one-liner printing the console dimensions it is attached to.
///
/// Not `mode con`: ConPTY does not implement every console API, and mode.com
/// reports a failed one as "Out of memory" — a probe that cannot tell a missing
/// console from an unimplemented call is no probe at all.
fn probe_argv() -> Vec<String> {
    let script = "try { $s = $Host.UI.RawUI.WindowSize; \
         Write-Output \"CONFINED size=$($s.Width)x$($s.Height) \
         redirected=$([Console]::IsOutputRedirected)\" } \
         catch { Write-Output \"CONFINED no-console: $_\" }";
    vec![
        "powershell".to_string(),
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        script.to_string(),
    ]
}
