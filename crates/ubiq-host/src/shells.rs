//! Which shells this machine actually has, and how one is started.
//!
//! The interface may not look for itself — a program on disk is a local fact, and no path crosses
//! into UI code — so the list is made here and answered over the bus as
//! [`Message::ShellList`](ubiq_proto::messages::Message::ShellList).
//!
//! It is a bounded search, not a launcher for anything on the machine. On Unix that bound is a
//! fixed set of known names checked for existence; on Windows it is the same idea applied to a
//! known install layout — PowerShell's own version directories under `%ProgramFiles%`, the Store
//! alias, and whatever `PATH` names — so a machine with several PowerShell builds shows a row for
//! each instead of just the first one found. Either way the menu stays a handful of shells, and an
//! open-ended scan of everything the machine could run would be a configuration feature nobody
//! asked for.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::OnceLock;

use ubiq_proto::messages::ShellInfo;

/// The shells offered, in the order the menu shows them.
#[cfg(unix)]
const CANDIDATES: &[&str] = &["zsh", "bash", "fish", "sh"];
#[cfg(windows)]
const CANDIDATES: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe"];

/// Where a shell is looked for besides `PATH`.
///
/// Ubiq started from Finder or a desktop launcher inherits a thin `PATH` — the very reason a pane's
/// shell has to source the user's own login files — so the usual homes are checked as well rather
/// than trusting the environment the application happens to have.
#[cfg(unix)]
const EXTRA_DIRS: &[&str] = &[
    "/bin",
    "/usr/bin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/run/current-system/sw/bin",
];
#[cfg(windows)]
const EXTRA_DIRS: &[&str] = &[];

/// What a session starts when it is not told what to start: the user's own shell.
#[cfg(unix)]
pub fn default_program() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// What a session starts when it is not told what to start. Windows has no `$SHELL`, so the
/// newest PowerShell install on the machine is the default — ahead of one merely found on `PATH`
/// with no version attached — falling back to the inbox `powershell.exe`, then to the command
/// processor the system names, as it is for every other terminal there.
#[cfg(windows)]
pub fn default_program() -> String {
    let shells = windows_shells();
    if let Some(shell) = shells
        .iter()
        .find(|shell| shell.label.starts_with("PowerShell"))
    {
        return shell.program.to_string_lossy().into_owned();
    }
    if let Some(shell) = shells
        .iter()
        .find(|shell| shell.label == "Windows PowerShell 5.1")
    {
        return shell.program.to_string_lossy().into_owned();
    }
    std::env::var("COMSPEC")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "cmd.exe".to_string())
}

/// Whether two shell file names name the same shell. Windows file lookups are
/// case-insensitive — `COMSPEC` may spell `CMD.EXE` any way it likes — while Unix names compare
/// exactly.
fn names_equal(a: &str, b: &str) -> bool {
    #[cfg(unix)]
    {
        a == b
    }
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
}

/// Every shell this machine has, in menu order, with the default one marked.
///
/// A candidate that is not installed is left out — the menu offers what can actually be started.
/// The default is always in the list, even when it is a shell this module has never heard of: what
/// the new-pane control already starts has to be something the menu can name.
#[cfg(unix)]
pub fn available() -> Vec<ShellInfo> {
    let default = default_program();
    let default_name = basename(&default);

    let mut shells: Vec<ShellInfo> = Vec::new();
    if !CANDIDATES
        .iter()
        .any(|name| names_equal(name, &default_name))
    {
        shells.push(ShellInfo {
            label: label_of(&default_name),
            program: default.clone(),
            is_default: true,
        });
    }
    for name in CANDIDATES {
        let is_default = names_equal(name, &default_name);
        // The default's row is the default's own program, not whichever copy of that name the
        // probe found first: the row and a bare click on "+" have to start the same thing.
        let program = if is_default {
            Some(PathBuf::from(&default))
        } else {
            locate(name)
        };
        if let Some(program) = program {
            shells.push(ShellInfo {
                label: label_of(name),
                program: program.to_string_lossy().into_owned(),
                is_default,
            });
        }
    }
    shells
}

/// Every shell this machine has, in menu order, with the default one marked.
///
/// [`CANDIDATES`] is a fixed three names, but the menu is not: a machine can have several
/// `pwsh.exe` builds installed side by side, so [`windows_shells`] does the real enumeration and
/// this only turns its answer into rows, marking the one [`default_program`] already starts. That
/// row carries the default's own program string, not whichever copy of the same install
/// [`windows_shells`] happened to reach first, so the row and a bare click on "+" agree on what
/// they start.
#[cfg(windows)]
pub fn available() -> Vec<ShellInfo> {
    let default = default_program();
    let shells = windows_shells();

    let mut rows: Vec<ShellInfo> = Vec::new();
    let mut found_default = false;
    for shell in shells {
        let is_default = paths_equal(&shell.program, &default);
        found_default |= is_default;
        let program = if is_default {
            default.clone()
        } else {
            shell.program.to_string_lossy().into_owned()
        };
        rows.push(ShellInfo {
            label: shell.label,
            program,
            is_default,
        });
    }
    if !found_default {
        rows.insert(
            0,
            ShellInfo {
                label: label_from_path(Path::new(&default)),
                program: default,
                is_default: true,
            },
        );
    }
    rows
}

/// One shell [`windows_shells`] found, before it becomes the row [`available`] returns.
#[cfg(windows)]
struct WindowsShell {
    label: String,
    program: PathBuf,
}

/// Every shell this machine has, gathered and de-duplicated, in the order the menu wants them:
/// PowerShell installs newest major first (a preview after the release of the same major), then
/// any `pwsh` found with no version attached, then Windows PowerShell, then the command processor.
///
/// Two of these can name the same file — a version added to `PATH` for convenience, the Store
/// alias sitting next to an MSI install — so the list is de-duplicated by resolved path,
/// case-insensitively, keeping the first (and so the most informative) label found for it.
#[cfg(windows)]
fn windows_shells() -> Vec<WindowsShell> {
    let mut pwsh = powershell_installs();

    // The Store alias and whatever `pwsh.exe` `locate` finds on `PATH` are the same product with
    // no install directory to read a version from, so both get the bare "PowerShell" label.
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let alias = PathBuf::from(local_app_data)
            .join("Microsoft")
            .join("WindowsApps")
            .join("pwsh.exe");
        if is_executable(&alias) {
            pwsh.push(WindowsShell {
                label: label_from_path(&alias),
                program: alias,
            });
        }
    }
    if let Some(program) = locate("pwsh.exe") {
        pwsh.push(WindowsShell {
            label: label_from_path(&program),
            program,
        });
    }
    let mut shells = dedup_by_path(pwsh);

    if let Some(program) = locate("powershell.exe") {
        shells.push(WindowsShell {
            label: label_from_path(&program),
            program,
        });
    }

    let cmd = std::env::var("COMSPEC")
        .ok()
        .filter(|comspec| !comspec.is_empty())
        .map(PathBuf::from)
        .or_else(|| locate("cmd.exe"));
    if let Some(program) = cmd {
        shells.push(WindowsShell {
            label: label_from_path(&program),
            program,
        });
    }

    shells
}

/// Keeps the first of any duplicate path — comparing the way Windows does, without case, since a
/// version already found under `%ProgramFiles%` and the same file reached again through `PATH` are
/// one shell, not two rows.
#[cfg(windows)]
fn dedup_by_path(shells: Vec<WindowsShell>) -> Vec<WindowsShell> {
    let mut seen = std::collections::HashSet::new();
    shells
        .into_iter()
        .filter(|shell| seen.insert(shell.program.to_string_lossy().to_ascii_lowercase()))
        .collect()
}

/// Whether a path and a program string name the same file, the way Windows does — by spelling,
/// case-insensitively. This is a menu, not a filesystem canonicalisation: no symlink is resolved.
#[cfg(windows)]
fn paths_equal(a: &Path, b: &str) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(b)
}

/// Every `pwsh.exe` install under `%ProgramFiles%\PowerShell\`, newest major first and a preview
/// after the release of the same major.
///
/// `%ProgramFiles(x86)%` and `SysWOW64` are not read: this application does not offer the 32-bit
/// builds.
#[cfg(windows)]
fn powershell_installs() -> Vec<WindowsShell> {
    let Some(program_files) = std::env::var_os("ProgramFiles") else {
        return Vec::new();
    };
    let root = PathBuf::from(program_files).join("PowerShell");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut installs: Vec<(u32, bool, WindowsShell)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let program = entry.path().join("pwsh.exe");
        let Some((major, preview)) = powershell_install_version(&program) else {
            continue;
        };
        if !is_executable(&program) {
            continue;
        }
        let label = powershell_install_label(major, preview);
        installs.push((major, preview, WindowsShell { label, program }));
    }
    installs.sort_by_key(|(major, preview, _)| powershell_install_order(*major, *preview));
    installs.into_iter().map(|(_, _, shell)| shell).collect()
}

/// Sort key for a PowerShell install: newest major first, and within a major, the release before
/// its preview.
#[cfg(windows)]
fn powershell_install_order(major: u32, preview: bool) -> (std::cmp::Reverse<u32>, bool) {
    (std::cmp::Reverse(major), preview)
}

/// The label for a PowerShell install of this major version.
#[cfg(windows)]
fn powershell_install_label(major: u32, preview: bool) -> String {
    if preview {
        format!("PowerShell {major} (preview)")
    } else {
        format!("PowerShell {major}")
    }
}

/// The version a `pwsh.exe` path names, when it sits directly under an install directory —
/// `...\PowerShell\<version>\pwsh.exe` — and `None` for anything else: `PATH`, the Store alias, a
/// copy the user put somewhere of their own.
#[cfg(windows)]
fn powershell_install_version(program: &Path) -> Option<(u32, bool)> {
    let version_dir = program.parent()?;
    let install_root = version_dir.parent()?;
    if !install_root
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("PowerShell"))
    {
        return None;
    }
    powershell_version_dir(version_dir.file_name()?.to_str()?)
}

/// Whether a directory name under `...\PowerShell\` names a version install, and if so its major
/// version and whether it is a preview build.
///
/// The installer names a version directory by the release it holds — `6`, `7`, `7-preview` — and
/// drops one unversioned directory beside them, `Scripts`, which a leading-digit check rejects. The
/// exact minor version is not read: the directory never carries more than the major, and that is
/// all this module claims to know.
#[cfg(windows)]
fn powershell_version_dir(dir_name: &str) -> Option<(u32, bool)> {
    let (major, preview) = match dir_name.strip_suffix("-preview") {
        Some(rest) => (rest, true),
        None => (dir_name, false),
    };
    if major.is_empty() || !major.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    major.parse().ok().map(|major| (major, preview))
}

/// A Windows shell's row label, worked out from the full path the probe found it at.
///
/// `pwsh.exe` under a version directory is named for the release that directory names; found any
/// other way — `PATH`, the Store alias — it is only "PowerShell", because nothing at that path
/// says which build it is. `powershell.exe` is always "Windows PowerShell 5.1": 5.1 is the only
/// version of Windows PowerShell that ships on any Windows this application supports, and the
/// `v1.0` in its own install path is a historical lie. Anything else offered here is `cmd.exe`,
/// "Command Prompt".
#[cfg(windows)]
fn label_from_path(program: &Path) -> String {
    let name = basename(&program.to_string_lossy());
    if names_equal(&name, "pwsh.exe") {
        return match powershell_install_version(program) {
            Some((major, preview)) => powershell_install_label(major, preview),
            None => "PowerShell".to_string(),
        };
    }
    if names_equal(&name, "powershell.exe") {
        return "Windows PowerShell 5.1".to_string();
    }
    "Command Prompt".to_string()
}

/// Whether `program` is a shell, and so should be started the way a terminal application starts
/// one — see [`crate::pty::spawn`], which is what the answer changes.
pub fn is_shell(program: &str) -> bool {
    let name = basename(program);
    CANDIDATES
        .iter()
        .any(|candidate| names_equal(candidate, &name))
        || names_equal(&name, &basename(&default_program()))
}

/// A shell's row label on Unix: its own bare name.
#[cfg(unix)]
fn label_of(name: &str) -> String {
    name.strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name)
        .to_string()
}

/// The program's file name, which is how a shell is recognised — `$SHELL` is a path and a
/// candidate is a name.
fn basename(program: &str) -> String {
    Path::new(program)
        .file_name()
        .unwrap_or_else(|| OsStr::new(program))
        .to_string_lossy()
        .into_owned()
}

/// The `PATH` the user's own login shell has, which is not the one a desktop-launched Ubiq
/// inherits. Asked of the shell once per process: it costs a subprocess, and a machine does not
/// grow a new toolchain directory while a window is open.
#[cfg(unix)]
fn login_path() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let shell = default_program();
        // `-lic` rather than `-c`: the directories a toolchain installer adds are written into the
        // login and interactive files, not into a non-interactive shell's environment.
        let Ok(out) = std::process::Command::new(&shell)
            .args(["-lic", "printf %s \"$PATH\""])
            .output()
        else {
            return Vec::new();
        };
        let path = String::from_utf8_lossy(&out.stdout);
        std::env::split_paths(path.trim()).collect()
    })
}

/// Windows has no login shell to ask, so there is nothing beyond `PATH` and the usual homes.
#[cfg(windows)]
fn login_path() -> &'static [PathBuf] {
    &[]
}

/// Where a named program is on this machine: `PATH` first, then the login shell's own `PATH`, then
/// the usual homes.
///
/// The login shell's `PATH` is what finds a harness installed under the user's home — `~/.local/bin`,
/// a node prefix — which the environment a desktop launcher hands Ubiq does not name at all.
pub(crate) fn locate(name: &str) -> Option<PathBuf> {
    let names = file_names(name);
    let path = std::env::var_os("PATH");
    let from_path = path
        .as_deref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten();
    from_path
        .chain(login_path().iter().cloned())
        .chain(EXTRA_DIRS.iter().map(PathBuf::from))
        .find_map(|dir| {
            names
                .iter()
                .map(|file| dir.join(file))
                .find(|candidate| is_executable(candidate))
        })
}

/// The filenames a bare program name can have in a directory, in the order they are tried.
///
/// Unix has exactly one — the name as written, with the execute bit answering the rest.
#[cfg(unix)]
fn file_names(name: &str) -> Vec<OsString> {
    vec![OsString::from(name)]
}

/// The filenames a bare program name can have in a directory, in the order Windows itself tries
/// them: the name as written, then the name with each suffix in `PATHEXT`.
///
/// Windows has no execute bit — what makes a file a program is its extension — so a literal join
/// finds a harness only where something happened to drop an extensionless file beside it. npm's
/// global bin does exactly that (a POSIX shim next to `<name>.cmd`), which is why an npm-installed
/// harness was found and a native installer's `claude.exe` was not. A name that already carries an
/// extension is looked up as written, so the shell [`CANDIDATES`] keep resolving to themselves.
#[cfg(windows)]
fn file_names(name: &str) -> Vec<OsString> {
    let mut names = vec![OsString::from(name)];
    if Path::new(name).extension().is_some() {
        return names;
    }
    let pathext = std::env::var("PATHEXT").unwrap_or_default();
    let suffixes: Vec<String> = pathext
        .split(';')
        .map(str::trim)
        .filter(|suffix| suffix.starts_with('.') && suffix.len() > 1)
        .map(str::to_string)
        .collect();
    let suffixes = if suffixes.is_empty() {
        DEFAULT_PATHEXT.iter().map(|s| (*s).to_string()).collect()
    } else {
        suffixes
    };
    names.extend(suffixes.into_iter().map(|suffix| {
        let mut file = OsString::from(name);
        file.push(suffix);
        file
    }));
    names
}

/// What `PATHEXT` is taken to be when the environment does not name it — the four suffixes a
/// program is actually started by, in the order the system's own default lists them.
#[cfg(windows)]
const DEFAULT_PATHEXT: &[&str] = &[".COM", ".EXE", ".BAT", ".CMD"];

/// The `PATH` this process should really be running with: what it already has, then the login
/// shell's own `PATH`, then the usual homes ([`EXTRA_DIRS`]) — de-duplicated, order preserved.
///
/// Pure and side-effect-free so it can be tested without a login shell; [`repair_path`] is the
/// only caller that feeds it the real environment and writes the result back.
fn effective_path(current: &OsStr, login: &[PathBuf], extra: &[&str]) -> OsString {
    let mut seen = std::collections::HashSet::new();
    let dirs: Vec<PathBuf> = std::env::split_paths(current)
        .chain(login.iter().cloned())
        .chain(extra.iter().map(PathBuf::from))
        .filter(|dir| seen.insert(dir.clone()))
        .collect();
    std::env::join_paths(dirs).unwrap_or_else(|_| current.to_os_string())
}

/// Repair this process's own `PATH` from the login shell, once, at startup.
///
/// A desktop launcher (Finder, the dock) hands Ubiq a thin `PATH` — see the module doc — and
/// nothing before this point has done anything about it, so every bare-name spawn inside
/// `agent-manager` (`Command::new("claude")` and its siblings: `discover_models`,
/// `discover_thinking`, `version()`, every harness launch) fails under exactly that launch even
/// though the same binaries are found fine from a terminal.
///
/// # Safety / soundness
/// Must be called exactly once, before any other thread is spawned in this process. Mutating the
/// environment ([`std::env::set_var`] is `unsafe` since Rust 2024) races any concurrent reader —
/// the only window in which it is sound is before a second thread exists to read it. The sole
/// call site is the very start of `ubiq_app::run`, ahead of the coordinator thread, ahead of the
/// GPUI event loop, ahead of anything else this process spawns.
pub fn repair_path() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let composed = effective_path(&current, login_path(), EXTRA_DIRS);
    if composed == current {
        return;
    }
    // SAFETY: see above — this runs once, at process startup, before any other thread exists.
    unsafe { std::env::set_var("PATH", &composed) };
    tracing::debug!(path = %composed.to_string_lossy(), "repaired process PATH");
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shell_is_always_offered_and_marked() {
        let shells = available();
        let default: Vec<_> = shells.iter().filter(|shell| shell.is_default).collect();
        assert_eq!(
            default.len(),
            1,
            "exactly one row is the default: {shells:?}"
        );
        assert_eq!(default[0].program, default_program());
    }

    #[test]
    fn every_offered_shell_is_a_shell() {
        for shell in available() {
            assert!(
                is_shell(&shell.program),
                "{} is not recognised",
                shell.program
            );
            assert!(!shell.label.contains('/'), "a label is a name, not a path");
        }
    }

    /// What the login shell's `PATH` holds is this machine's business, so the assertion is the
    /// invariant instead: the probe still finds a shell, and the answer is computed once.
    #[test]
    #[cfg(unix)]
    fn the_login_path_is_asked_for_once_and_locate_still_finds_a_shell() {
        assert!(locate("sh").is_some(), "every unix machine has sh");
        assert!(
            std::ptr::eq(login_path(), login_path()),
            "the login shell is asked once per process"
        );
    }

    #[test]
    fn a_program_that_is_not_a_shell_is_not_started_as_one() {
        assert!(!is_shell("/usr/local/bin/claude"));
        assert!(!is_shell("codex"));
    }

    /// PowerShell outranks the command processor: whatever `windows_shells` finds first on this
    /// machine is the default, and `COMSPEC` is only the fallback when no PowerShell — installed
    /// or merely on `PATH` — is here at all.
    #[test]
    #[cfg(windows)]
    fn the_default_is_the_newest_powershell_on_the_machine() {
        let shells = windows_shells();
        match shells
            .iter()
            .find(|shell| shell.label.starts_with("PowerShell"))
        {
            Some(pwsh) => assert_eq!(default_program(), pwsh.program.to_string_lossy()),
            None => match shells
                .iter()
                .find(|shell| shell.label == "Windows PowerShell 5.1")
            {
                Some(powershell) => {
                    assert_eq!(default_program(), powershell.program.to_string_lossy())
                }
                None => assert_eq!(
                    default_program(),
                    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
                ),
            },
        }
    }

    /// `COMSPEC` may spell `CMD.EXE` in any case; a shell is still recognised.
    #[test]
    #[cfg(windows)]
    fn shell_names_compare_case_insensitively() {
        assert!(names_equal("cmd.exe", "CMD.EXE"));
        assert!(!names_equal("cmd.exe", "pwsh.exe"));
        assert!(is_shell("C:\\WINDOWS\\system32\\CMD.EXE"));
        assert!(is_shell(
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\PowerShell.exe"
        ));
    }

    /// A harness names its program without an extension, and a native installer drops a `.exe`
    /// with no extensionless companion — so the bare name has to be tried with `PATHEXT` or the
    /// harness is reported as not installed while it runs fine from any shell.
    #[test]
    #[cfg(windows)]
    fn a_bare_name_is_tried_with_every_executable_suffix() {
        let names = file_names("claude");
        assert_eq!(
            names[0],
            OsString::from("claude"),
            "the name as written first"
        );
        for suffix in [".EXE", ".CMD", ".BAT", ".COM"] {
            let mut wanted = OsString::from("claude");
            wanted.push(suffix);
            assert!(
                names
                    .iter()
                    .any(|name| names_equal(&name.to_string_lossy(), &wanted.to_string_lossy())),
                "{suffix} is tried: {names:?}"
            );
        }
    }

    /// A name that already carries an extension — every shell in [`CANDIDATES`] — is looked up as
    /// written, so nothing resolves to `cmd.exe.exe`.
    #[test]
    #[cfg(windows)]
    fn a_name_with_an_extension_is_looked_up_as_written() {
        assert_eq!(file_names("cmd.exe"), vec![OsString::from("cmd.exe")]);
    }

    /// The same question asked of the real machine: a bare name resolves to the program a shell
    /// would start.
    #[test]
    #[cfg(windows)]
    fn locate_resolves_a_bare_name_against_the_machine() {
        let found = locate("cmd").expect("every windows machine has a command processor");
        assert!(is_shell(&found.to_string_lossy()), "resolved to {found:?}");
    }

    /// `pwsh.exe` sitting right under a version directory is named for the release that directory
    /// names, and a preview directory says so in the label too.
    #[test]
    #[cfg(windows)]
    fn label_from_path_names_a_versioned_powershell_install() {
        assert_eq!(
            label_from_path(Path::new(r"C:\Program Files\PowerShell\7\pwsh.exe")),
            "PowerShell 7"
        );
        assert_eq!(
            label_from_path(Path::new(r"C:\Program Files\PowerShell\6\pwsh.exe")),
            "PowerShell 6"
        );
        assert_eq!(
            label_from_path(Path::new(r"C:\Program Files\PowerShell\7-preview\pwsh.exe")),
            "PowerShell 7 (preview)"
        );
    }

    /// A `pwsh.exe` reached any other way — the Store alias, a copy sitting on `PATH` — carries no
    /// version, so the label says only what it is.
    #[test]
    #[cfg(windows)]
    fn label_from_path_names_an_unversioned_pwsh_just_powershell() {
        assert_eq!(
            label_from_path(Path::new(
                r"C:\Users\marco\AppData\Local\Microsoft\WindowsApps\pwsh.exe"
            )),
            "PowerShell"
        );
        assert_eq!(
            label_from_path(Path::new(r"C:\tools\pwsh.exe")),
            "PowerShell"
        );
    }

    /// `powershell.exe` is always "Windows PowerShell 5.1", whatever its own path claims.
    #[test]
    #[cfg(windows)]
    fn label_from_path_names_windows_powershell_regardless_of_its_own_v1_path() {
        assert_eq!(
            label_from_path(Path::new(
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
            )),
            "Windows PowerShell 5.1"
        );
    }

    /// Anything else offered is `cmd.exe`, and its label is "Command Prompt".
    #[test]
    #[cfg(windows)]
    fn label_from_path_names_the_command_processor() {
        assert_eq!(
            label_from_path(Path::new(r"C:\Windows\System32\cmd.exe")),
            "Command Prompt"
        );
    }

    /// A version directory has a leading digit; the installer's own `Scripts` directory, and
    /// anything else that isn't a version, are rejected.
    #[test]
    #[cfg(windows)]
    fn powershell_version_dir_accepts_a_release_and_a_preview() {
        assert_eq!(powershell_version_dir("7"), Some((7, false)));
        assert_eq!(powershell_version_dir("6"), Some((6, false)));
        assert_eq!(powershell_version_dir("7-preview"), Some((7, true)));
    }

    #[test]
    #[cfg(windows)]
    fn powershell_version_dir_rejects_a_non_version_directory() {
        assert_eq!(powershell_version_dir("Scripts"), None);
        assert_eq!(powershell_version_dir(""), None);
        assert_eq!(powershell_version_dir("preview"), None);
    }

    /// Newest major first, and a preview sits right after the release of the same major rather
    /// than before it or off with some other major entirely.
    #[test]
    #[cfg(windows)]
    fn powershell_installs_sort_newest_major_first_preview_after_its_release() {
        let mut keys = vec![
            powershell_install_order(6, false),
            powershell_install_order(7, true),
            powershell_install_order(7, false),
        ];
        keys.sort();
        assert_eq!(
            keys,
            vec![
                powershell_install_order(7, false),
                powershell_install_order(7, true),
                powershell_install_order(6, false),
            ]
        );
    }

    /// A version already found under `%ProgramFiles%` and the same file reached again through
    /// `PATH` are one shell, not two rows — the first (most informative) label found wins.
    #[test]
    #[cfg(windows)]
    fn dedup_by_path_keeps_the_first_of_a_duplicate() {
        let versioned = WindowsShell {
            label: "PowerShell 7".to_string(),
            program: PathBuf::from(r"C:\Program Files\PowerShell\7\pwsh.exe"),
        };
        let same_file_different_case = WindowsShell {
            label: "PowerShell".to_string(),
            program: PathBuf::from(r"c:\program files\powershell\7\pwsh.exe"),
        };
        let deduped = dedup_by_path(vec![versioned, same_file_different_case]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].label, "PowerShell 7");
    }

    #[cfg(unix)]
    fn path(dirs: &[&str]) -> OsString {
        std::env::join_paths(dirs.iter().map(PathBuf::from)).unwrap()
    }

    #[cfg(unix)]
    fn paths(dirs: &[&str]) -> Vec<PathBuf> {
        dirs.iter().map(PathBuf::from).collect()
    }

    /// A directory the login shell adds that `PATH` doesn't have yet is appended, after what was
    /// already there.
    #[test]
    #[cfg(unix)]
    fn effective_path_appends_new_login_directories_after_the_existing_ones() {
        let current = path(&["/usr/bin", "/bin"]);
        let login = paths(&["/usr/bin", "/opt/homebrew/bin"]);
        let composed = effective_path(&current, &login, &[]);
        assert_eq!(
            std::env::split_paths(&composed).collect::<Vec<_>>(),
            paths(&["/usr/bin", "/bin", "/opt/homebrew/bin"])
        );
    }

    /// A login shell that names nothing new, and no extra dirs either, leaves `PATH` untouched —
    /// not merely equal, but the very same value, so [`repair_path`] can skip the write.
    #[test]
    #[cfg(unix)]
    fn effective_path_is_a_no_op_when_the_login_shell_adds_nothing_new() {
        let current = path(&["/usr/bin", "/bin"]);
        let login = paths(&["/bin", "/usr/bin"]);
        let composed = effective_path(&current, &login, &[]);
        assert_eq!(composed, current);
    }

    /// Existing entries always come first, whatever order the login shell and the extra homes
    /// name their own duplicates in.
    #[test]
    #[cfg(unix)]
    fn effective_path_keeps_existing_entries_first_and_drops_duplicates() {
        let current = path(&["/opt/homebrew/bin", "/bin"]);
        let login = paths(&["/bin", "/opt/homebrew/bin", "/usr/local/bin"]);
        let composed = effective_path(
            &current,
            &login,
            &["/usr/local/bin", "/run/current-system/sw/bin"],
        );
        assert_eq!(
            std::env::split_paths(&composed).collect::<Vec<_>>(),
            paths(&[
                "/opt/homebrew/bin",
                "/bin",
                "/usr/local/bin",
                "/run/current-system/sw/bin",
            ])
        );
    }
}
