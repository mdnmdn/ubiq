//! `am-confine` — the process that stands between a Windows pane and its
//! confined harness.
//!
//! isol8 confines only a process it creates itself: it has no ConPTY seam, and
//! `SandboxChild` cannot be handed descriptors from outside. So a host that has
//! already opened a pseudoconsole has nothing to exec — on macOS
//! `isolate::confined_launch` renders `sandbox-exec -p <policy>`, which replaces
//! itself in place, and Windows has no equivalent.
//!
//! What it does have is the inverse. isol8's Windows backend calls
//! `CreateProcessW` with no console creation flag — no `CREATE_NEW_CONSOLE`, no
//! `DETACHED_PROCESS`, no `CREATE_NO_WINDOW` — so the confined child attaches to
//! its caller's console. Run this shim *inside* the pane and isol8 creates the
//! harness from in there: the harness lands on the very ConPTY the host opened,
//! at the right size, as a real console rather than a pipe.
//!
//! The shim decides nothing. `isolate::confined_launch` resolved the layer
//! stack, materialized the home and confined the executable before writing the
//! payload; this reads those values and hands them to the backend.
//!
//! Two duties beyond that, both about what a child inherits:
//!
//! - **The working directory.** isol8 grants the resolving process's directory
//!   read-write, and a confined child inherits its parent's, so the shim adopts
//!   the directory the policy was resolved against before spawning anything. A
//!   pane started with no explicit directory begins in the user's home, and a
//!   harness whose policy grants somewhere else dies on startup.
//! - **The process tree.** The host kills a pane by terminating the process it
//!   spawned, which is this shim, and the harness underneath would outlive it.
//!   So the shim puts itself in a job object that kills on close: every
//!   descendant joins by inheritance, and the harness and its build tools go
//!   when the shim does.
//!
//! Invoked as `am-confine <payload.json>`. Not a user-facing command.

fn main() -> anyhow::Result<()> {
    let payload = match std::env::args_os().nth(1) {
        Some(path) => std::path::PathBuf::from(path),
        None => anyhow::bail!("usage: am-confine <payload.json>"),
    };
    run(&payload)
}

#[cfg(target_os = "windows")]
fn run(payload: &std::path::Path) -> anyhow::Result<()> {
    use agent_manager::isolate::ConfinePayload;
    use anyhow::{Context as _, anyhow};

    let payload = ConfinePayload::take(payload)?;

    // Both before the spawn: from here on, whatever this process creates
    // inherits the directory the policy grants, and belongs to the job.
    std::env::set_current_dir(&payload.cwd).with_context(|| {
        format!(
            "entering {} — the directory this run's policy was resolved against",
            payload.cwd.display()
        )
    })?;
    kill_on_close_job().context("putting the confined run in a kill-on-close job")?;

    let env: std::collections::HashMap<String, String> = payload.env.into_iter().collect();
    let mut child = isol8::backends::select()
        .spawn(&payload.profile, &env, &payload.cmd)
        .map_err(|e| anyhow!("spawning the confined harness: {e}"))?;

    let code = child
        .wait()
        .map_err(|e| anyhow!("waiting on the confined harness: {e}"))?;
    std::process::exit(code);
}

/// Create an unnamed job object with `KILL_ON_JOB_CLOSE` and put this process in
/// it, keeping the handle for the life of the process.
///
/// The handle closes when the process ends — including a `TerminateProcess` from
/// the host — and closing the last handle to such a job terminates everything
/// still in it. Nested jobs are fine: a host that is itself in a job does not
/// stop this one being created.
#[cfg(target_os = "windows")]
fn kill_on_close_job() -> anyhow::Result<()> {
    use std::mem::size_of;

    use anyhow::anyhow;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: every call below is a plain Win32 call on handles this function
    // owns. `info` is zeroed and fully initialized before it is passed, and its
    // size is the one the API is told to read.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(anyhow!(
                "CreateJobObjectW failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(anyhow!("SetInformationJobObject failed: {err}"));
        }

        if AssignProcessToJobObject(job, GetCurrentProcess()) == 0 {
            let err = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(anyhow!("AssignProcessToJobObject failed: {err}"));
        }

        // Deliberately never closed: the job must outlive this function, and the
        // kernel closes the handle when the process ends — which is exactly the
        // moment the job should take the harness with it.
        let _ = job;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn run(_payload: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!(
        "am-confine exists for Windows, where isol8 has no ConPTY seam. \
         Everywhere else `isolate::confined_launch` renders a policy the caller execs itself."
    )
}
