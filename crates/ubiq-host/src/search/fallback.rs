//! An external tool a search may fall back to.
//!
//! Invoked from exactly one place: the `Err` arm of `RegexMatcherBuilder::build` in
//! [`super::worker`]. That is the only built-in failure an external tool can rescue — a missing
//! project root is [`ubiq_proto::search::SearchError::Root`], and a bad glob is refused before any
//! tool runs. `ripgrep`'s regex engine is stricter than the PCRE-ish dialects `grep -E` and `ag`
//! accept, so a pattern a user copied from elsewhere is exactly the case this rescues.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ubiq_proto::search::{FileHit, LineHit};

use super::Job;
use super::ceiling;

/// One external tool, found on this machine.
pub struct Chosen {
    pub tool: String,
    pub program: PathBuf,
}

/// The first configured tool that exists, through [`crate::shells::locate`] — `PATH`, the login
/// shell's `PATH`, then the usual homes.
///
/// `find` and `fd` match file *names*, not contents, and cannot answer a content search: if either
/// is configured, it is skipped with a log line rather than run and its output misread as hits.
pub fn pick(order: &[String]) -> Option<Chosen> {
    for tool in order {
        if tool == "find" || tool == "fd" {
            tracing::debug!(
                tool,
                "matches file names, not contents; skipped as a search fallback"
            );
            continue;
        }
        if let Some(program) = crate::shells::locate(tool) {
            return Some(Chosen {
                tool: tool.clone(),
                program,
            });
        }
    }
    None
}

/// Run `chosen` over `start` and answer the hits it printed, bounded by the same ceilings as the
/// built-in path.
///
/// Killed if it has not finished by `deadline` — the one place in this crate worth more than
/// `.output()`, since a wedged external tool would otherwise wedge the search worker permanently.
pub fn run(
    chosen: &Chosen,
    job: &Job,
    start: &Path,
    deadline: Duration,
) -> Result<(Vec<FileHit>, usize), String> {
    let mut command = Command::new(&chosen.program);
    match chosen.tool.as_str() {
        "ag" => {
            command.arg("--numbers").arg("--nocolor").arg("--nogroup");
            if !job.query.case_sensitive {
                command.arg("-i");
            }
            if job.query.whole_word {
                command.arg("-w");
            }
            command.arg(&job.query.text).arg(start);
        }
        // grep, and anything else configured that speaks its dialect.
        _ => {
            command.arg("-rInE").arg("--binary-files=without-match");
            if !job.query.case_sensitive {
                command.arg("-i");
            }
            if job.query.whole_word {
                command.arg("-w");
            }
            command.arg("-e").arg(&job.query.text).arg(start);
        }
    }

    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{} did not start: {error}", chosen.tool))?;
    let child = Arc::new(Mutex::new(child));

    let stdout = child
        .lock()
        .unwrap()
        .stdout
        .take()
        .expect("stdout was piped");

    // A watchdog, not a timeout on the read: `BufRead::lines` blocks on the child, so the only way
    // to bound it is to kill the child out from under it. Killing an already-finished child is a
    // harmless no-op, so this thread is never joined — it is left to run its course.
    {
        let child = Arc::clone(&child);
        thread::spawn(move || {
            thread::sleep(deadline);
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
        });
    }

    let started = Instant::now();
    let mut files: Vec<FileHit> = Vec::new();
    let mut current: Option<(String, Vec<LineHit>)> = None;
    let mut total_hits = 0usize;
    let mut files_with_hits = 0usize;

    for line in std::io::BufReader::new(stdout).lines() {
        let Ok(line) = line else { continue };
        let Some((path, file_line, text)) = parse_hit(&line) else {
            continue;
        };
        let rel_path = strip_root(path, &job.root);

        // No matcher was compiled for this search — that is exactly why a fallback ran — so the
        // matched byte range within the line is not knowable. The whole line is highlighted
        // instead of inventing a range that would light up the wrong bytes.
        let hit = LineHit {
            line: file_line,
            ranges: vec![(0, text.len() as u32)],
            text: text.to_string(),
        };

        match &mut current {
            Some((current_path, lines)) if *current_path == rel_path => {
                if lines.len() < ceiling::HITS_PER_FILE {
                    lines.push(hit);
                }
                total_hits += 1;
            }
            _ => {
                if let Some((rel_path, lines)) = current.take() {
                    let truncated = lines.len() >= ceiling::HITS_PER_FILE;
                    files.push(FileHit {
                        rel_path,
                        lines,
                        truncated,
                    });
                    files_with_hits += 1;
                }
                if files_with_hits >= ceiling::FILES_WITH_HITS || total_hits >= ceiling::TOTAL_HITS
                {
                    let _ = child.lock().unwrap().kill();
                    break;
                }
                total_hits += 1;
                current = Some((rel_path, vec![hit]));
            }
        }
    }
    if let Some((rel_path, lines)) = current.take() {
        let truncated = lines.len() >= ceiling::HITS_PER_FILE;
        files.push(FileHit {
            rel_path,
            lines,
            truncated,
        });
    }

    let elapsed = started.elapsed();
    let _ = child.lock().unwrap().wait();
    if elapsed >= deadline {
        return Err(format!(
            "{} did not finish within {deadline:?}",
            chosen.tool
        ));
    }

    Ok((files, total_hits))
}

/// Split `path:line:text` on the first two colons only — the matched text itself may hold colons.
fn parse_hit(line: &str) -> Option<(&str, u32, &str)> {
    let (path, rest) = line.split_once(':')?;
    let (line_no, text) = rest.split_once(':')?;
    Some((path, line_no.parse().ok()?, text))
}

/// Project-relative, the same convention the built-in walk uses.
fn strip_root(path: &str, root: &Path) -> String {
    Path::new(path)
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(path))
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_name_nothing_on_this_machine_answers_no_chosen() {
        assert!(pick(&["definitely-not-a-real-search-tool".to_string()]).is_none());
    }

    #[test]
    fn find_and_fd_are_skipped_even_if_installed() {
        // Neither answers matches by content, so neither is ever chosen, whether or not it is on
        // this machine.
        assert!(pick(&["find".to_string(), "fd".to_string()]).is_none());
    }

    #[test]
    fn parses_path_line_text_keeping_colons_in_the_text() {
        assert_eq!(
            parse_hit("src/main.rs:12:let x: u32 = 1;"),
            Some(("src/main.rs", 12, "let x: u32 = 1;"))
        );
        assert!(parse_hit("not a hit line").is_none());
    }
}
