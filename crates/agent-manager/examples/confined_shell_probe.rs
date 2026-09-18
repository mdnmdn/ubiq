//! Which shells survive a confined pane, and why the ones that do not fail.
//!
//! A harness spawns shells: Claude Code's own `Bash` and `PowerShell` tools are
//! children of the confined process, so whatever isol8's hook denies them, the
//! agent reports as "my shell is broken". The two probes beside this one measure
//! the *pane* — the ConPTY, its size, a keystroke reaching the child. This one
//! measures the layer above: a confined command that spawns `pwsh`, `bash` or
//! `cmd` and wants an exit code back.
//!
//! It runs against a payload the interface actually wrote — `_data/config/isol8/
//! confine/<pid>-<n>.json` — rather than a policy rebuilt here, so a failure it
//! reports is a failure a pane had. Each case swaps only [`ConfinePayload::cmd`];
//! the profile, the environment and the working directory are the pane's own.
//!
//! Every case is bounded. A denial isol8 turns into `STATUS_ACCESS_DENIED` shows
//! up as a non-zero exit with a message; a denial the *guest* mishandles shows up
//! as a timeout, and the two are different bugs.
//!
//! Run: `cargo run -p agent-manager --example confined_shell_probe
//!       -- _data/config/isol8/confine/<pid>-<n>.json`
//!
//! An explicit command after the payload narrows the battery to that one case.
//!
//! `PROBE_UNCONFINED=1` runs the same commands with no confiner, which is the
//! control: a case that fails there is measuring the machine, not the sandbox.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use agent_manager::isolate::{CONFINE_ARG, ConfinePayload, confine_entrypoint};
use isol8::{Access, MatchKind, PathGrant};

/// What a case grants on top of the pane's own policy, to tell a missing grant
/// from a broken mechanism: whichever addition turns a failure into a pass names
/// the line the Windows policy is missing.
#[derive(Clone, Copy)]
enum Extra {
    /// The pane's policy, untouched.
    None,
    /// PowerShell 7's per-user configuration directories.
    Documents,
    /// NT device object names, read-write.
    Devices(&'static [&'static str]),
}

/// How long a case may take before it counts as hung rather than denied.
const DEADLINE: Duration = Duration::from_secs(25);

fn main() -> anyhow::Result<()> {
    // A confine run is this same binary invoked again, exactly as `ubiq.exe` does.
    if let Some(code) = confine_entrypoint() {
        std::process::exit(code);
    }

    let arg = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!("usage: confined_shell_probe <payload.json>  (one the interface wrote)")
    })?;
    let bytes = std::fs::read(&arg)?;
    let base: ConfinePayload = serde_json::from_slice(&bytes)?;

    let state = std::env::temp_dir().join("am-shell-probe");
    std::fs::create_dir_all(&state)?;
    let confiner = std::env::current_exe()?;

    println!("payload : {arg}");
    println!("cwd     : {}", base.cwd.display());
    println!("grants  : {}", base.profile.paths.len());
    println!("env     : {} variables", base.env.len());
    println!(
        "mode    : {}\n",
        if unconfined() {
            "UNCONFINED (control)"
        } else {
            "confined"
        }
    );

    // An explicit command after the payload runs that and nothing else, which is
    // how a case gets narrowed once the battery below says which one to chase.
    let extra: Vec<String> = std::env::args().skip(2).collect();
    if !extra.is_empty() {
        let payload = ConfinePayload {
            profile: base.profile.clone(),
            env: base.env.clone(),
            cmd: extra,
            cwd: base.cwd.clone(),
        };
        return report("explicit", &confiner, &state, &payload);
    }

    let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    let bash = git_bash.to_string_lossy().into_owned();
    // The NT device the null sink resolves to. msys2's `/dev/null` and cmd's
    // `NUL` both open it, and isol8's hook checks a device object name against
    // the same path policy as a file, so a spec that names no device denies it.
    let null_device: &[&str] = &[r"\Device\Null"];
    let console: &[&str] = &[r"\Device\ConDrv", r"\Device\NamedPipe"];
    // msys2 resolves `/usr/bin` through the 8.3 short name of the directory that
    // holds it, and isol8's matcher never expands a short name back — so a grant
    // on `C:\Program Files` does not cover the `C:\PROGRA~1` the loader is
    // actually handed. Granting the short form is the test, not the fix.
    let short: &[&str] = &[r"\Device\Null", r"C:\PROGRA~1"];
    let cases: Vec<(&str, Vec<String>, Extra)> = vec![
        ("cmd.exe", argv(&["cmd", "/c", "echo CMD-OK"]), Extra::None),
        (
            "powershell 5.1",
            argv(&[
                "powershell",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output PS5-OK",
            ]),
            Extra::None,
        ),
        (
            "pwsh 7",
            argv(&[
                "pwsh",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output PWSH-OK",
            ]),
            Extra::None,
        ),
        (
            "pwsh 7 + Documents grant",
            argv(&[
                "pwsh",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output PWSH-OK",
            ]),
            Extra::Documents,
        ),
        (
            "git bash",
            vec![bash.clone(), "-c".into(), "echo BASH-OK".into()],
            Extra::None,
        ),
        (
            "git bash --login",
            vec![bash.clone(), "-lc".into(), "echo BASH-LOGIN-OK".into()],
            Extra::None,
        ),
        // msys2 emulates `fork` by cloning an address space into a suspended
        // child, which is the one thing a DLL injected at every process-creation
        // hop is most likely to disturb. A pipeline is the cheapest way to make
        // bash actually fork rather than `exec` its only command.
        (
            "git bash + fork",
            vec![
                bash.clone(),
                "-c".into(),
                "echo a | cat >/dev/null; echo BASH-FORK-OK".into(),
            ],
            Extra::None,
        ),
        // cmd's own null sink, which is the same NT device under a different
        // name: if this fails too, nothing about the failure is msys-specific.
        (
            "cmd > NUL",
            argv(&["cmd", "/c", "echo x > NUL && echo NUL-OK"]),
            Extra::None,
        ),
        // The shape that matters: a *confined* process spawning the shell, so the
        // hook's own `CreateProcessInternalW` detour runs — suspend, inject,
        // resume — instead of isol8's backend doing the first injection.
        (
            "node -> git bash (the real shape)",
            argv(&[
                "node",
                "-e",
                r#"console.log(require('child_process').execSync('"C:\\Program Files\\Git\\bin\\bash.exe" -c "echo HOP2-NODE-BASH-OK"').toString())"#,
            ]),
            Extra::None,
        ),
        (
            "node alone",
            argv(&["node", "-e", "console.log('NODE-OK')"]),
            Extra::None,
        ),
        // The candidate fix, one device at a time: whichever line turns a HANG
        // into a PASS names the grant the Windows policy is missing.
        (
            "cmd > NUL + null-device grant",
            argv(&["cmd", "/c", "echo x > NUL && echo NUL-OK"]),
            Extra::Devices(null_device),
        ),
        (
            "git bash + fork + null-device grant",
            vec![
                bash.clone(),
                "-c".into(),
                "echo a | cat >/dev/null; echo BASH-FORK-OK".into(),
            ],
            Extra::Devices(null_device),
        ),
        (
            "git bash --login + null-device grant",
            vec![bash.clone(), "-lc".into(), "echo BASH-LOGIN-OK".into()],
            Extra::Devices(null_device),
        ),
        (
            "git bash --login + null/condrv/pipe",
            vec![bash.clone(), "-lc".into(), "echo BASH-LOGIN-OK".into()],
            Extra::Devices(console),
        ),
        (
            "git bash + fork + null + short name",
            vec![
                bash.clone(),
                "-c".into(),
                "echo a | cat >/dev/null; echo BASH-FORK-OK".into(),
            ],
            Extra::Devices(short),
        ),
        (
            "git bash --login + null + short name",
            vec![bash.clone(), "-lc".into(), "echo BASH-LOGIN-OK".into()],
            Extra::Devices(short),
        ),
        // The line that separates the two possible bugs. Grant the whole drive
        // and every device: if bash still hangs, no path grant can fix it and the
        // fault is in the mechanism — isol8 injects its hook at every process
        // creation, and msys2's `fork` clones an address space that injection
        // has already changed. If it passes, some named path is simply missing.
        (
            "git bash + fork + everything",
            vec![
                bash.clone(),
                "-c".into(),
                "echo a | cat >/dev/null; echo BASH-FORK-OK".into(),
            ],
            Extra::Devices(&[r"C:\", r"\Device", r"\??"]),
        ),
        (
            "git bash --login + everything",
            vec![bash.clone(), "-lc".into(), "echo BASH-LOGIN-OK".into()],
            Extra::Devices(&[r"C:\", r"\Device", r"\??"]),
        ),
    ];

    for (name, cmd, extra) in cases {
        let mut payload = ConfinePayload {
            profile: base.profile.clone(),
            env: base.env.clone(),
            cmd,
            cwd: base.cwd.clone(),
        };
        match extra {
            Extra::None => {}
            Extra::Documents => {
                for dir in documents_dirs() {
                    payload.profile.paths.push(PathGrant {
                        path: dir.to_string_lossy().into_owned(),
                        access: Access::Ro,
                        r#match: MatchKind::Subpath,
                    });
                }
            }
            Extra::Devices(devices) => {
                for device in devices {
                    payload.profile.paths.push(PathGrant {
                        path: (*device).to_string(),
                        access: Access::Rw,
                        r#match: MatchKind::Subpath,
                    });
                }
            }
        }
        report(name, &confiner, &state, &payload)?;
    }

    Ok(())
}

fn unconfined() -> bool {
    std::env::var_os("PROBE_UNCONFINED").is_some()
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_string()).collect()
}

/// The per-user PowerShell configuration directories, under the real profile.
///
/// PowerShell 7 reads `powershell.config.json` from `Documents\PowerShell` at
/// startup and treats an unreadable-but-present file as fatal, so a policy that
/// names no grant there kills `pwsh` before it runs a line. 5.1 uses
/// `WindowsPowerShell` and does not, which is why the two other probes — both of
/// which run `powershell` — never saw this.
fn documents_dirs() -> Vec<PathBuf> {
    let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    let documents = profile.join("Documents");
    vec![
        documents.join("PowerShell"),
        documents.join("WindowsPowerShell"),
    ]
}

/// Run one case to completion, a timeout, and print what happened.
fn report(
    name: &str,
    confiner: &Path,
    state: &Path,
    payload: &ConfinePayload,
) -> anyhow::Result<()> {
    let mut command = if unconfined() {
        let mut c = std::process::Command::new(&payload.cmd[0]);
        c.args(&payload.cmd[1..]);
        c
    } else {
        let mut c = std::process::Command::new(confiner);
        c.arg(CONFINE_ARG).arg(payload.write(state)?);
        c
    };
    command
        .current_dir(&payload.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started = Instant::now();
    let mut child = command.spawn()?;

    // Drain on threads: a child that fills a pipe while the probe waits on the
    // exit code deadlocks for a reason that has nothing to do with the sandbox.
    let mut out = child.stdout.take().map(drain);
    let mut err = child.stderr.take().map(drain);

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() >= DEADLINE {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let stdout = out.take().map(join).unwrap_or_default();
    let stderr = err.take().map(join).unwrap_or_default();
    let elapsed = started.elapsed();

    match status {
        Some(status) if status.success() => {
            println!("PASS  {name}  ({:?})", elapsed);
        }
        Some(status) => {
            println!("FAIL  {name}  exit {:?}  ({:?})", status.code(), elapsed);
        }
        None => {
            println!("HANG  {name}  still running after {DEADLINE:?}, killed");
        }
    }
    for line in stdout.lines().chain(stderr.lines()) {
        if !line.trim().is_empty() {
            println!("      | {line}");
        }
    }
    println!();
    Ok(())
}

fn drain<R: std::io::Read + Send + 'static>(mut r: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn join(handle: std::thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}
