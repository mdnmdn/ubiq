//! Applying a downloaded update: how this install can take one, and the detached helper that does.
//!
//! **The helper outlives us on purpose.** A running app cannot replace itself, so a script is
//! spawned in its own process group (Unix) or detached (Windows), given our PID, and waits for that
//! PID to disappear before touching anything. The script builders are pure functions of their
//! arguments, so they are tested on every platform; only the spawning is `cfg`-gated.

use std::path::{Path, PathBuf};

use ubiq_proto::update::ApplyMode;

/// The platform key a manifest's `platforms` map uses, or `None` where updates are not offered.
pub fn platform_key() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-aarch64")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x86_64")
    } else {
        None
    }
}

/// How the running executable can apply an update.
pub fn mode_of(exe: &Path) -> ApplyMode {
    if cfg!(target_os = "macos") {
        match bundle_of(exe) {
            Some(bundle) if !bundle.to_string_lossy().contains("/AppTranslocation/") => {
                if bundle.parent().is_some_and(writable) {
                    ApplyMode::Swap
                } else {
                    ApplyMode::Manual
                }
            }
            _ => ApplyMode::Manual,
        }
    } else if cfg!(windows) {
        match exe.parent() {
            Some(dir) if dir.join("unins000.exe").is_file() => ApplyMode::Installer,
            _ => ApplyMode::Manual,
        }
    } else {
        ApplyMode::Manual
    }
}

/// `…/Ubiq.app` for `…/Ubiq.app/Contents/MacOS/ubiq`.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

/// Whether a file can be made in `dir` — the honest test of "the swap will be allowed".
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".ubiq-update-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Quote for `/bin/sh`.
fn sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Quote for a PowerShell single-quoted string.
fn ps(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The macOS swap script.
pub fn mac_script(pid: u32, zip: &Path, stage: &Path, bundle: &Path, relaunch: bool) -> String {
    let name = bundle.file_name().map_or("Ubiq.app".into(), |n| n.to_string_lossy());
    let mut script = format!(
        r#"#!/bin/sh
# Written by Ubiq: waits for pid {pid} to exit, then replaces the app bundle.
STAGE={stage}
exec >> "$STAGE/apply.log" 2>&1
echo "update helper started $(date)"
while kill -0 {pid} 2>/dev/null; do sleep 1; done
BUNDLE={bundle}
NEW="$STAGE/new/"{name}
OLD="$STAGE/backup/old.app"
rm -rf "$STAGE/new" "$STAGE/backup"
mkdir -p "$STAGE/new" "$STAGE/backup" || exit 1
/usr/bin/ditto -x -k {zip} "$STAGE/new" || {{ echo "unpack failed"; exit 1; }}
/usr/bin/codesign --verify --deep --strict "$NEW" || {{ echo "signature check failed; leaving the installed app alone"; exit 1; }}
/usr/bin/xattr -dr com.apple.quarantine "$NEW"
mv "$BUNDLE" "$OLD" || {{ echo "could not move the old app aside"; exit 1; }}
if ! mv "$NEW" "$BUNDLE"; then
  echo "could not move the new app into place; restoring"
  mv "$OLD" "$BUNDLE"
  exit 1
fi
rm -rf "$STAGE/backup" "$STAGE/new"
echo "updated"
"#,
        stage = sh(&stage.to_string_lossy()),
        bundle = sh(&bundle.to_string_lossy()),
        name = sh(&name),
        zip = sh(&zip.to_string_lossy()),
    );
    if relaunch {
        script.push_str(&format!("/usr/bin/open -n {}\n", sh(&bundle.to_string_lossy())));
    }
    script
}

/// The Windows PowerShell command: wait, install silently, optionally start the app again.
pub fn windows_command(pid: u32, installer: &Path, exe: &Path, relaunch: bool) -> String {
    let mut command = format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         Start-Process -Wait -FilePath {} -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER';",
        ps(&installer.to_string_lossy())
    );
    if relaunch {
        command.push_str(&format!(" Start-Process -FilePath {};", ps(&exe.to_string_lossy())));
    }
    command
}

/// Start the helper for `mode`. `file` is the verified download; `stage` a scratch directory.
pub fn spawn(
    mode: ApplyMode,
    file: &Path,
    stage: &Path,
    relaunch: bool,
) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Ubiq could not find its own program: {error}"))?;
    let pid = std::process::id();
    match mode {
        ApplyMode::Swap => spawn_swap(pid, file, stage, &exe, relaunch),
        ApplyMode::Installer => spawn_installer(pid, file, &exe, relaunch),
        ApplyMode::Manual => {
            Err("this copy of Ubiq cannot update itself; download the new version and install it by hand".to_string())
        }
    }
}

#[cfg(unix)]
fn spawn_swap(pid: u32, zip: &Path, stage: &Path, exe: &Path, relaunch: bool) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let bundle = bundle_of(exe).ok_or("Ubiq is not running from an app bundle")?;
    std::fs::create_dir_all(stage).map_err(|error| format!("could not prepare the update: {error}"))?;
    let script = stage.join("apply.sh");
    crate::atomic::write_atomic(&script, mac_script(pid, zip, stage, &bundle, relaunch).as_bytes())
        .map_err(|error| format!("could not write the update helper: {error}"))?;
    Command::new("/bin/sh")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
        .map_err(|error| format!("could not start the update helper: {error}"))
}

#[cfg(not(unix))]
fn spawn_swap(_: u32, _: &Path, _: &Path, _: &Path, _: bool) -> Result<(), String> {
    Err("swapping an app bundle is not supported on this platform".to_string())
}

#[cfg(windows)]
fn spawn_installer(pid: u32, installer: &Path, exe: &Path, relaunch: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command"])
        .arg(windows_command(pid, installer, exe, relaunch))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(drop)
        .map_err(|error| format!("could not start the installer: {error}"))
}

#[cfg(not(windows))]
fn spawn_installer(_: u32, _: &Path, _: &Path, _: bool) -> Result<(), String> {
    Err("the installer is not supported on this platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_is_three_levels_above_the_executable() {
        let exe = Path::new("/Applications/Ubiq.app/Contents/MacOS/ubiq");
        assert_eq!(bundle_of(exe).unwrap(), Path::new("/Applications/Ubiq.app"));
        assert!(bundle_of(Path::new("/usr/local/bin/ubiq")).is_none());
    }

    #[test]
    fn a_translocated_or_bare_executable_is_manual() {
        let exe = Path::new("/private/var/AppTranslocation/X/d/Ubiq.app/Contents/MacOS/ubiq");
        assert_eq!(mode_of(exe), ApplyMode::Manual);
        assert_eq!(mode_of(Path::new("/tmp/target/debug/ubiq")), ApplyMode::Manual);
    }

    #[test]
    fn the_mac_script_waits_verifies_and_restores() {
        let script = mac_script(
            42,
            Path::new("/c/u.zip"),
            Path::new("/c/stage"),
            Path::new("/Applications/Ubiq.app"),
            true,
        );
        let wait = script.find("kill -0 42").unwrap();
        let unpack = script.find("ditto -x -k").unwrap();
        let verify = script.find("codesign --verify --deep --strict").unwrap();
        let quarantine = script.find("xattr -dr com.apple.quarantine").unwrap();
        let swap = script.find("mv \"$NEW\" \"$BUNDLE\"").unwrap();
        assert!(wait < unpack && unpack < verify && verify < quarantine && quarantine < swap);
        assert!(script.contains("mv \"$OLD\" \"$BUNDLE\""));
        assert!(script.trim_end().ends_with("/usr/bin/open -n '/Applications/Ubiq.app'"));
        assert!(!mac_script(1, Path::new("z"), Path::new("s"), Path::new("/A/U.app"), false).contains("open -n"));
    }

    #[test]
    fn quoting_survives_an_apostrophe() {
        assert_eq!(sh("it's"), "'it'\\''s'");
        assert_eq!(ps("it's"), "'it''s'");
    }

    #[test]
    fn the_windows_command_waits_installs_then_relaunches() {
        let command = windows_command(7, Path::new(r"C:\c\Setup.exe"), Path::new(r"C:\A\ubiq.exe"), true);
        assert!(command.starts_with("Wait-Process -Id 7"));
        assert!(command.contains("/VERYSILENT") && command.contains("/CURRENTUSER"));
        assert!(command.ends_with(r"Start-Process -FilePath 'C:\A\ubiq.exe';"));
        assert!(!windows_command(7, Path::new("s"), Path::new("e"), false).contains("ubiq.exe"));
    }
}
