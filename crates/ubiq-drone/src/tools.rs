//! The remote-tool family (`D226`): what an agent bound to this drone asks of it, answered.
//!
//! A thin façade, as the contract says: path safety is [`ubiq_host::files::path`]'s (the same
//! boundary the file family crosses), listing and reading are `ubiq_host::files`'s, the atomic
//! write is `ubiq_host::atomic`'s, and grep shells to the tool [`crate::search::probe`] found.
//! [`answer`] is synchronous and blocks for as long as the op takes — a `Run` for as long as its
//! timeout — so the relay calls it on a thread of its own, never on the bus's.
//!
//! **Read-only roots.** The drone has no such concept yet: every root it serves is writable, and
//! [`Root::read_only`] is always `false` in `Info`. The field is on the wire for when it exists.

use std::collections::VecDeque;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ubiq_host::files::{self, path};
use ubiq_proto::remote_tools::{
    REMOTE_READ_CAP, REMOTE_RUN_TAIL_CAP, RemoteEntry, RemoteInfo, RemoteMatch, RemoteOp,
    RemoteOutput, RemoteRoot, RemoteRun,
};

/// One root this drone serves: the name a call's `root` uses, and its folder.
#[derive(Clone, Debug)]
pub struct Root {
    pub name: String,
    pub path: PathBuf,
}

/// Entries one `List` returns at most, and paths one `Glob` or matches one `Grep` returns at most.
const LIST_CAP: usize = 2_000;
const GLOB_CAP: usize = 2_000;
const GREP_CAP: usize = 1_000;

/// How long a tool's `--version` may take before it is reported absent.
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// Answer one op. `hostname` is only for `Info`.
pub fn answer(
    roots: &[Root],
    root: Option<&str>,
    hostname: &str,
    op: RemoteOp,
) -> Result<RemoteOutput, String> {
    if let RemoteOp::Info = op {
        return Ok(RemoteOutput::Info(Box::new(info(roots, hostname))));
    }
    let root = pick_root(roots, root)?;
    match op {
        RemoteOp::Info => unreachable!("answered above"),
        RemoteOp::List { path, depth } => list(root, &path, depth),
        RemoteOp::Read {
            path,
            offset,
            limit,
        } => read(root, &path, offset, limit),
        RemoteOp::Write {
            path,
            content,
            overwrite,
        } => write(root, &path, &content, overwrite),
        RemoteOp::Edit {
            path,
            old,
            new,
            replace_all,
        } => edit(root, &path, &old, &new, replace_all),
        RemoteOp::Glob { pattern, path } => glob(root, &pattern, path.as_deref()),
        RemoteOp::Grep {
            pattern,
            path,
            glob,
            case_insensitive,
            max_results,
        } => grep(
            root,
            &pattern,
            path.as_deref(),
            glob.as_deref(),
            case_insensitive,
            max_results,
        ),
        RemoteOp::Run {
            cmd,
            cwd,
            timeout_ms,
            shell,
        } => run(root, &cmd, cwd.as_deref(), timeout_ms, shell.as_deref()),
    }
}

/// `None` is the first root; a name must match one exactly.
fn pick_root<'a>(roots: &'a [Root], name: Option<&str>) -> Result<&'a Path, String> {
    let found = match name {
        None => roots.first(),
        Some(name) => roots.iter().find(|root| root.name == name),
    };
    match found {
        Some(root) => Ok(&root.path),
        None => Err(match name {
            Some(name) => format!(
                "no root named {name:?}; this drone serves: {}",
                roots
                    .iter()
                    .map(|root| root.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => "this drone serves no root".to_string(),
        }),
    }
}

fn refuse(error: impl std::fmt::Display) -> String {
    error.to_string()
}

// ── info ─────────────────────────────────────────────────────────────

fn info(roots: &[Root], hostname: &str) -> RemoteInfo {
    RemoteInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: hostname.to_string(),
        shell: Some(ubiq_host::shells::default_program()),
        roots: roots
            .iter()
            .map(|root| RemoteRoot {
                name: root.name.clone(),
                path: ubiq_host::host_path::wire_string(&root.path),
                read_only: false,
            })
            .collect(),
        tools: tool_versions().to_vec(),
    }
}

static TOOLS: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// The tools on `PATH` and their first `--version` line, probed once per process.
fn tool_versions() -> &'static [(String, String)] {
    TOOLS.get_or_init(|| {
        let names = [
            "git", "rg", "node", "python3", "python", "dotnet", "pwsh", "cargo",
        ];
        let handles: Vec<_> = names
            .iter()
            .map(|name| {
                let name = name.to_string();
                std::thread::spawn(move || probe_version(&name).map(|version| (name, version)))
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok().flatten())
            .collect()
    })
}

fn probe_version(name: &str) -> Option<String> {
    let mut child = Command::new(name)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if started.elapsed() > PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut text = String::new();
    let _ = stdout.read_to_string(&mut text);
    if text.trim().is_empty() {
        let _ = stderr.read_to_string(&mut text);
    }
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

// ── files ────────────────────────────────────────────────────────────

fn list(root: &Path, rel: &str, depth: Option<u32>) -> Result<RemoteOutput, String> {
    let depth = depth.unwrap_or(1).clamp(1, u8::MAX as u32) as u8;
    let listings = files::listing(root, rel, depth).map_err(refuse)?;
    let mut truncated = false;
    let mut entries = Vec::new();
    'all: for listing in listings {
        truncated |= listing.truncated;
        for entry in listing.entries {
            if entries.len() >= LIST_CAP {
                truncated = true;
                break 'all;
            }
            entries.push(RemoteEntry {
                path: entry.rel_path,
                kind: entry.kind,
                size: entry.size,
            });
        }
    }
    Ok(RemoteOutput::List { entries, truncated })
}

fn read(root: &Path, rel: &str, offset: u64, limit: u64) -> Result<RemoteOutput, String> {
    let file = files::contents(root, rel, None).map_err(refuse)?;
    if file.is_binary {
        return Err(format!("{rel} is a binary file"));
    }
    let text = String::from_utf8_lossy(&file.bytes);
    let mut content = String::new();
    let mut truncated = file.truncated;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if (index as u64) < offset {
            continue;
        }
        if limit > 0 && index as u64 >= offset + limit {
            truncated = true;
            break;
        }
        if content.len() + line.len() > REMOTE_READ_CAP {
            // A cut line is still progress: take what fits, on a character boundary.
            let mut end = REMOTE_READ_CAP - content.len();
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            content.push_str(&line[..end]);
            truncated = true;
            break;
        }
        content.push_str(line);
    }
    Ok(RemoteOutput::Read { content, truncated })
}

fn write(root: &Path, rel: &str, content: &str, overwrite: bool) -> Result<RemoteOutput, String> {
    // Folders above the file are made, as the agent's own write tool would.
    let target = path::resolve_for_create(root, rel).map_err(refuse)?;
    let existed = target.exists();
    if existed && !overwrite {
        return Err(format!(
            "{rel} already exists; pass overwrite to replace it"
        ));
    }
    if target.is_dir() {
        return Err(format!("{rel} is a folder"));
    }
    let mode = std::fs::metadata(&target).ok().map(|meta| meta.permissions());
    ubiq_host::atomic::write_atomic_with(&target, content.as_bytes(), mode)
        .map_err(|error| format!("{rel}: {error}"))?;
    Ok(RemoteOutput::Write {
        bytes: content.len() as u64,
        created: !existed,
    })
}

fn edit(
    root: &Path,
    rel: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<RemoteOutput, String> {
    if old.is_empty() {
        return Err("old must not be empty".to_string());
    }
    let target = path::resolve(root, rel).map_err(refuse)?;
    let meta = std::fs::metadata(&target).map_err(|error| format!("{rel}: {error}"))?;
    if !meta.is_file() {
        return Err(format!("{rel} is not a file"));
    }
    let text = std::fs::read_to_string(&target)
        .map_err(|error| format!("{rel}: not readable as text: {error}"))?;
    let count = text.matches(old).count();
    if count == 0 {
        return Err(format!("old text not found in {rel}"));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "old text occurs {count} times in {rel}; add context to make it unique or set replace_all"
        ));
    }
    let edited = text.replace(old, new);
    ubiq_host::atomic::write_atomic_with(&target, edited.as_bytes(), Some(meta.permissions()))
        .map_err(|error| format!("{rel}: {error}"))?;
    Ok(RemoteOutput::Edit {
        replacements: count as u32,
    })
}

// ── glob and grep ────────────────────────────────────────────────────

/// A search start under the root: the root itself for `None`/empty.
fn start_dir(root: &Path, rel: Option<&str>) -> Result<PathBuf, String> {
    let start = path::resolve(root, rel.unwrap_or("")).map_err(refuse)?;
    if !start.is_dir() {
        return Err(format!("{} is not a folder", rel.unwrap_or("")));
    }
    Ok(start)
}

fn rel_of(root: &Path, full: &Path) -> String {
    full.strip_prefix(root)
        .unwrap_or(full)
        .to_string_lossy()
        .replace('\\', "/")
}

fn glob_matcher(start: &Path, pattern: &str) -> Result<ignore::overrides::Override, String> {
    let mut builder = ignore::overrides::OverrideBuilder::new(start);
    builder
        .add(pattern)
        .map_err(|error| format!("bad glob {pattern:?}: {error}"))?;
    builder.build().map_err(|error| error.to_string())
}

fn glob(root: &Path, pattern: &str, rel: Option<&str>) -> Result<RemoteOutput, String> {
    let start = start_dir(root, rel)?;
    let root = path::resolve(root, "").map_err(refuse)?;
    let matcher = glob_matcher(&start, pattern)?;
    let mut paths = Vec::new();
    let mut truncated = false;
    for entry in ignore::WalkBuilder::new(&start)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
        .flatten()
    {
        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
        if entry.depth() == 0 || !matcher.matched(entry.path(), is_dir).is_whitelist() {
            continue;
        }
        if paths.len() >= GLOB_CAP {
            truncated = true;
            break;
        }
        paths.push(rel_of(&root, entry.path()));
    }
    paths.sort();
    Ok(RemoteOutput::Glob { paths, truncated })
}

fn grep(
    root: &Path,
    pattern: &str,
    rel: Option<&str>,
    glob: Option<&str>,
    case_insensitive: bool,
    max_results: u32,
) -> Result<RemoteOutput, String> {
    let start = start_dir(root, rel)?;
    let root = path::resolve(root, "").map_err(refuse)?;
    let found = crate::search::probe().ok_or("no search tool on this machine's PATH (rg, ag, grep)")?;
    let matcher = glob.map(|glob| glob_matcher(&root, glob)).transpose()?;
    let cap = if max_results == 0 {
        GREP_CAP
    } else {
        (max_results as usize).min(GREP_CAP)
    };

    let mut command = found.grep_command(pattern, case_insensitive, &start);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("search tool did not start: {error}"))?;
    let stdout = child.stdout.take().expect("piped");
    let mut matches = Vec::new();
    let mut truncated = false;
    for line in std::io::BufRead::lines(std::io::BufReader::new(stdout)) {
        let Ok(line) = line else { continue };
        let Some((file, number, text)) = crate::search::parse_grep_line(&line) else {
            continue;
        };
        let file = rel_of(&root, Path::new(&file));
        if let Some(matcher) = &matcher
            && !matcher.matched(root.join(&file), false).is_whitelist()
        {
            continue;
        }
        if matches.len() >= cap {
            truncated = true;
            let _ = child.kill();
            break;
        }
        matches.push(RemoteMatch {
            path: file,
            line: number,
            text,
        });
    }
    let _ = child.wait();
    Ok(RemoteOutput::Grep { matches, truncated })
}

// ── run ──────────────────────────────────────────────────────────────

/// The last [`REMOTE_RUN_TAIL_CAP`] bytes of a stream, filled by a reader thread.
#[derive(Default)]
struct Tail {
    bytes: VecDeque<u8>,
    cut: bool,
}

fn drain(mut stream: impl std::io::Read + Send + 'static) -> Arc<Mutex<Tail>> {
    let tail = Arc::new(Mutex::new(Tail::default()));
    let shared = tail.clone();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        while let Ok(n) = stream.read(&mut buffer) {
            if n == 0 {
                break;
            }
            let Ok(mut tail) = shared.lock() else { break };
            tail.bytes.extend(&buffer[..n]);
            let excess = tail.bytes.len().saturating_sub(REMOTE_RUN_TAIL_CAP);
            if excess > 0 {
                tail.bytes.drain(..excess);
                tail.cut = true;
            }
        }
    });
    tail
}

fn take(tail: &Arc<Mutex<Tail>>) -> (String, bool) {
    let tail = tail.lock().expect("tail lock");
    let bytes: Vec<u8> = tail.bytes.iter().copied().collect();
    (String::from_utf8_lossy(&bytes).into_owned(), tail.cut)
}

#[cfg(unix)]
fn shell_command(shell: Option<&str>, cmd: &str) -> Command {
    let mut command = Command::new(shell.unwrap_or("sh"));
    command.arg("-c").arg(cmd);
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
    command
}

#[cfg(windows)]
fn shell_command(shell: Option<&str>, cmd: &str) -> Command {
    use std::os::windows::process::CommandExt as _;
    let on_path = |name: &str| {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join(format!("{name}.exe")).is_file())
        })
    };
    let mut command = match shell {
        Some(shell) => {
            let mut command = Command::new(shell);
            command.arg("-c").arg(cmd);
            command
        }
        None => {
            let program = ["pwsh", "powershell"].into_iter().find(|name| on_path(name));
            match program {
                Some(program) => {
                    let mut command = Command::new(program);
                    command.args(["-NoProfile", "-NonInteractive", "-Command"]).arg(cmd);
                    command
                }
                None => {
                    let mut command = Command::new("cmd");
                    command.arg("/C").raw_arg(cmd);
                    command
                }
            }
        }
    };
    command.creation_flags(0x0800_0000);
    command
}

fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The shell is its own process group (see `shell_command`), so a group kill takes what it
        // started with it and the drained pipes close.
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn run(
    root: &Path,
    cmd: &str,
    cwd: Option<&str>,
    timeout_ms: u64,
    shell: Option<&str>,
) -> Result<RemoteOutput, String> {
    let dir = match cwd {
        Some(cwd) if !cwd.is_empty() => start_dir(root, Some(cwd))?,
        _ => path::resolve(root, "").map_err(refuse)?,
    };
    let mut command = shell_command(shell, cmd);
    let mut child = command
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start the shell: {error}"))?;
    let out = drain(child.stdout.take().expect("piped"));
    let err = drain(child.stderr.take().expect("piped"));

    let timeout = Duration::from_millis(timeout_ms.max(1));
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                timed_out = true;
                kill_tree(&mut child);
                break child.wait().map_err(|error| error.to_string())?;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    // Let the readers take the last bytes the pipes held.
    std::thread::sleep(Duration::from_millis(30));
    let (stdout, cut_out) = take(&out);
    let (stderr, cut_err) = take(&err);
    Ok(RemoteOutput::Run(Box::new(RemoteRun {
        exit_code: if timed_out { None } else { status.code() },
        stdout,
        stderr,
        truncated: cut_out || cut_err,
        timed_out,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Vec<Root>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let roots = vec![Root {
            name: "main".to_string(),
            path: dir.path().to_path_buf(),
        }];
        (dir, roots)
    }

    fn call(roots: &[Root], op: RemoteOp) -> Result<RemoteOutput, String> {
        answer(roots, None, "h", op)
    }

    fn edit_op(old: &str, new: &str, replace_all: bool) -> RemoteOp {
        RemoteOp::Edit {
            path: "a.txt".to_string(),
            old: old.to_string(),
            new: new.to_string(),
            replace_all,
        }
    }

    #[test]
    fn edit_needs_a_unique_old_unless_replace_all() {
        let (dir, roots) = fixture();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "one two one").unwrap();

        assert!(call(&roots, edit_op("zzz", "x", false)).unwrap_err().contains("not found"));
        assert!(call(&roots, edit_op("one", "x", false)).unwrap_err().contains("2 times"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one two one");

        assert_eq!(
            call(&roots, edit_op("two", "2", false)),
            Ok(RemoteOutput::Edit { replacements: 1 })
        );
        assert_eq!(
            call(&roots, edit_op("one", "1", true)),
            Ok(RemoteOutput::Edit { replacements: 2 })
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "1 2 1");
    }

    #[test]
    fn write_refuses_an_existing_file_without_overwrite() {
        let (dir, roots) = fixture();
        let op = |overwrite| RemoteOp::Write {
            path: "sub/b.txt".to_string(),
            content: "hi".to_string(),
            overwrite,
        };
        assert_eq!(
            call(&roots, op(false)),
            Ok(RemoteOutput::Write { bytes: 2, created: true })
        );
        assert!(call(&roots, op(false)).is_err());
        assert_eq!(
            call(&roots, op(true)),
            Ok(RemoteOutput::Write { bytes: 2, created: false })
        );
        assert_eq!(std::fs::read_to_string(dir.path().join("sub/b.txt")).unwrap(), "hi");
    }

    #[test]
    fn a_path_cannot_leave_its_root() {
        let (_dir, roots) = fixture();
        for escape in ["../x", "/etc/passwd", "a/../../x"] {
            let read = RemoteOp::Read {
                path: escape.to_string(),
                offset: 0,
                limit: 0,
            };
            assert!(call(&roots, read).is_err(), "{escape}");
            let write = RemoteOp::Write {
                path: escape.to_string(),
                content: "x".to_string(),
                overwrite: true,
            };
            assert!(call(&roots, write).is_err(), "{escape}");
        }
        let run = RemoteOp::Run {
            cmd: "true".to_string(),
            cwd: Some("..".to_string()),
            timeout_ms: 1000,
            shell: None,
        };
        assert!(call(&roots, run).is_err());
    }

    #[test]
    fn read_pages_by_line() {
        let (dir, roots) = fixture();
        std::fs::write(dir.path().join("l.txt"), "a\nb\nc\nd\n").unwrap();
        let out = call(
            &roots,
            RemoteOp::Read {
                path: "l.txt".to_string(),
                offset: 1,
                limit: 2,
            },
        );
        assert_eq!(
            out,
            Ok(RemoteOutput::Read {
                content: "b\nc\n".to_string(),
                truncated: true
            })
        );
    }

    #[test]
    fn an_unknown_root_is_named() {
        let (_dir, roots) = fixture();
        let error = answer(&roots, Some("nope"), "h", RemoteOp::List { path: String::new(), depth: None })
            .unwrap_err();
        assert!(error.contains("main"));
    }

    #[test]
    fn glob_finds_by_pattern() {
        let (dir, roots) = fixture();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        let out = call(
            &roots,
            RemoteOp::Glob {
                pattern: "**/*.rs".to_string(),
                path: None,
            },
        );
        assert_eq!(
            out,
            Ok(RemoteOutput::Glob {
                paths: vec!["src/a.rs".to_string()],
                truncated: false
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_exit_and_output() {
        let (_dir, roots) = fixture();
        let out = call(
            &roots,
            RemoteOp::Run {
                cmd: "echo out; echo err >&2; exit 3".to_string(),
                cwd: None,
                timeout_ms: 5000,
                shell: None,
            },
        );
        let Ok(RemoteOutput::Run(run)) = out else { panic!("{out:?}") };
        assert_eq!(run.exit_code, Some(3));
        assert_eq!(run.stdout, "out\n");
        assert_eq!(run.stderr, "err\n");
        assert!(!run.timed_out);
    }

    #[cfg(unix)]
    #[test]
    fn run_is_killed_at_its_timeout() {
        let (_dir, roots) = fixture();
        let started = Instant::now();
        let out = call(
            &roots,
            RemoteOp::Run {
                cmd: "echo before; sleep 30".to_string(),
                cwd: None,
                timeout_ms: 300,
                shell: None,
            },
        );
        let Ok(RemoteOutput::Run(run)) = out else { panic!("{out:?}") };
        assert!(run.timed_out);
        assert_eq!(run.exit_code, None);
        assert_eq!(run.stdout, "before\n");
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
