//! A destination as text, and text as a destination.
//!
//! ```text
//! ubiq://<project-id>/<view>[/<item>][#<locus>]
//! ```
//!
//! **The view slug decides the arity, not the number of segments.** That is the whole trick: a
//! file path is many segments and a pane id is one, and only the slug in front of them says which
//! to expect — so `ide/crates/ubiq/src/app/nav.rs` is unambiguous without escaping a single `/`.
//!
//! **The project is written as its id.** A name is not stable and a bookmark from three weeks ago
//! needs one that is; the name is what gets *drawn*, never what gets written.
//!
//! **Parsing is total.** A string that is not a link is [`NotALink`] and nothing else happens —
//! no error, no toast, no partial arrival at the project it happened to name.

use std::fmt;
use std::path::{Component, Path};
use std::str::FromStr;

use ubiq_proto::ids::{PaneId, ProjectId, SessionId, TaskId};
use ubiq_proto::work::AgentId;

use super::{Destination, Locus, View};
use crate::state::dock::ChatId;
use crate::state::orchestration::{InspectorTab, Selection};

const SCHEME: &str = "ubiq://";

/// This text does not name a place. The only failure the form has, because every call site is an
/// `.ok()` and none of them has anything to say about *why*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotALink;

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}/", self.project)?;
        match &self.view {
            View::Control => f.write_str("control")?,
            View::Kb => f.write_str("kb")?,
            View::Git => f.write_str("git")?,
            View::Logs => f.write_str("logs")?,
            View::Ide { key } => write!(f, "ide/{}", encode(key))?,
            View::Explorer { path } => write!(f, "explorer/{}", encode(path))?,
            View::Terminal { pane } => write!(f, "terminal/{pane}")?,
            View::Graph { selection, tab } => {
                let (kind, id) = match selection {
                    Selection::Session(id) => ("s", id.to_string()),
                    Selection::Agent(id) => ("a", id.to_string()),
                };
                let tab = match tab {
                    InspectorTab::Chat => "chat",
                    InspectorTab::Tasks => "tasks",
                };
                write!(f, "graph/{kind}:{id}/{tab}")?;
            }
            View::Agents { agent } => write!(f, "agents/{agent}")?,
            View::Tasks { task } => write!(f, "tasks/{task}")?,
            View::Chat { chat } => write!(f, "chat/{chat}")?,
        }
        if let Some(locus) = &self.locus {
            write!(f, "#{}", print_locus(locus))?;
        }
        Ok(())
    }
}

impl FromStr for Destination {
    type Err = NotALink;

    fn from_str(text: &str) -> Result<Self, NotALink> {
        let rest = text.trim().strip_prefix(SCHEME).ok_or(NotALink)?;
        // A literal `#` is always the fragment mark: one inside a path is written `%23`.
        let (head, frag) = match rest.find('#') {
            Some(cut) => (&rest[..cut], Some(&rest[cut + 1..])),
            None => (rest, None),
        };
        let mut parts = head.splitn(3, '/');
        let project: ProjectId = parts
            .next()
            .ok_or(NotALink)?
            .parse()
            .map_err(|_| NotALink)?;
        let slug = parts.next().ok_or(NotALink)?;
        let item = parts.next();
        let view = parse_view(slug, item)?;
        let locus = match frag {
            Some(frag) => parse_locus(&decode(frag)?)?,
            None => None,
        };
        Ok(Destination {
            project,
            view,
            locus,
        })
    }
}

/// The view a slug names, with whatever followed it — which the slug alone says how to read.
fn parse_view(slug: &str, item: Option<&str>) -> Result<View, NotALink> {
    // The screens that name nothing. A trailing segment is not a refinement of them, it is a
    // different string, so it is not a link.
    let none = |view: View| match item {
        None => Ok(view),
        Some(_) => Err(NotALink),
    };
    // Exactly one segment, for the arms whose item is a single id.
    let one = || match item {
        Some(text) if !text.is_empty() && !text.contains('/') => Ok(text),
        _ => Err(NotALink),
    };
    match slug {
        "control" => none(View::Control),
        "kb" => none(View::Kb),
        "git" => none(View::Git),
        "logs" => none(View::Logs),
        // Everything up to the fragment, however many segments that is.
        "ide" => {
            let key = decode(item.ok_or(NotALink)?)?;
            path_ok(&crate::state::editor::from_tab_key(&key).0)?;
            Ok(View::Ide { key })
        }
        "explorer" => {
            let path = decode(item.ok_or(NotALink)?)?;
            path_ok(&path)?;
            Ok(View::Explorer { path })
        }
        "terminal" => Ok(View::Terminal {
            pane: one()?.parse::<PaneId>().map_err(|_| NotALink)?,
        }),
        "tasks" => Ok(View::Tasks {
            task: one()?.parse::<TaskId>().map_err(|_| NotALink)?,
        }),
        "chat" => Ok(View::Chat {
            chat: one()?.parse::<ChatId>().map_err(|_| NotALink)?,
        }),
        "agents" => Ok(View::Agents {
            agent: one()?.parse::<AgentId>().map_err(|_| NotALink)?,
        }),
        // A selection, then optionally which half of the inspector is up.
        "graph" => {
            let item = item.ok_or(NotALink)?;
            let (sel, tab) = match item.split_once('/') {
                Some((sel, tab)) => (sel, Some(tab)),
                None => (item, None),
            };
            // The `s:`/`a:` prefix is forced: both ids are 26-character ULIDs and the text alone
            // cannot tell a session from an agent.
            let selection = match sel.split_once(':') {
                Some(("s", id)) => {
                    Selection::Session(id.parse::<SessionId>().map_err(|_| NotALink)?)
                }
                Some(("a", id)) => Selection::Agent(id.parse::<AgentId>().map_err(|_| NotALink)?),
                _ => return Err(NotALink),
            };
            let tab = match tab {
                None | Some("chat") => InspectorTab::Chat,
                Some("tasks") => InspectorTab::Tasks,
                Some(_) => return Err(NotALink),
            };
            Ok(View::Graph { selection, tab })
        }
        _ => Err(NotALink),
    }
}

fn print_locus(locus: &Locus) -> String {
    match locus {
        Locus::Line { line } => format!("L{line}"),
        Locus::Span { from, to } => format!("L{from}-{to}"),
        Locus::Viewport { x, y, scale } => format!("v={x},{y},{scale}"),
        Locus::Node { key } => format!("n={}", encode(key)),
        // A slug that would read back as a line, a viewport, a node — or as another slug, which
        // is what a leading `a=` does — has to say it is a slug, or the round trip changes it.
        Locus::Anchor { slug } => match parse_locus(slug) {
            Ok(Some(Locus::Anchor { slug: read })) if &read == slug => encode(slug),
            _ => format!("a={}", encode(slug)),
        },
    }
}

/// A fragment, already unescaped. Tried in order, with the anchor as the catch-all.
fn parse_locus(frag: &str) -> Result<Option<Locus>, NotALink> {
    if frag.is_empty() {
        return Ok(None);
    }
    if let Some(rest) = frag.strip_prefix('L')
        && rest.starts_with(|c: char| c.is_ascii_digit())
    {
        let locus = match rest.split_once('-') {
            None => Locus::Line { line: line(rest)? },
            Some((from, to)) => {
                let (from, to) = (line(from)?, line(to)?);
                // Written backwards means the same two lines.
                Locus::Span {
                    from: from.min(to),
                    to: from.max(to),
                }
            }
        };
        return Ok(Some(locus));
    }
    if let Some(rest) = frag.strip_prefix("v=") {
        let mut numbers = rest.split(',');
        let mut next = || -> Result<f32, NotALink> {
            numbers
                .next()
                .ok_or(NotALink)?
                .parse::<f32>()
                .map_err(|_| NotALink)
        };
        let (x, y, scale) = (next()?, next()?, next()?);
        if numbers.next().is_some() {
            return Err(NotALink);
        }
        return Ok(Some(Locus::Viewport { x, y, scale }));
    }
    if let Some(key) = frag.strip_prefix("n=") {
        return Ok(Some(Locus::Node {
            key: key.to_string(),
        }));
    }
    let slug = frag.strip_prefix("a=").unwrap_or(frag);
    Ok(Some(Locus::Anchor {
        slug: slug.to_string(),
    }))
}

/// A one-based line number. Line zero is not a line.
fn line(text: &str) -> Result<u32, NotALink> {
    match text.parse::<u32>() {
        Ok(0) | Err(_) => Err(NotALink),
        Ok(line) => Ok(line),
    }
}

/// The interface's mirror of `ubiq-host`'s `files::path::components`: no `..`, nothing rooted or
/// prefixed, no NUL. The host stays the actual boundary — this only keeps a nonsense link from
/// being offered as a place to go.
fn path_ok(path: &str) -> Result<(), NotALink> {
    if path.contains('\0') {
        return Err(NotALink);
    }
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(NotALink);
            }
        }
    }
    Ok(())
}

/// Escape exactly what the grammar would otherwise read: the escape mark, the fragment mark, the
/// space that ends a link in running prose, and the control bytes. **UTF-8 passes through raw**,
/// because a link to `_docs/café.md` is meant to be read by a person.
fn encode(text: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'%' | b'#' | b' ' | 0x00..=0x1f | 0x7f => {
                out.extend_from_slice(format!("%{byte:02X}").as_bytes())
            }
            _ => out.push(byte),
        }
    }
    // Every byte kept is one of the input's own, so what comes out is the input's UTF-8.
    String::from_utf8(out).unwrap_or_default()
}

/// Undo [`encode`]. A malformed escape fails the whole parse rather than surviving as itself:
/// half a link is worse than none.
fn decode(text: &str) -> Result<String, NotALink> {
    let raw = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut at = 0;
    while at < raw.len() {
        if raw[at] == b'%' {
            let hex = raw.get(at + 1..at + 3).ok_or(NotALink)?;
            let hex = std::str::from_utf8(hex).map_err(|_| NotALink)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| NotALink)?);
            at += 3;
        } else {
            out.push(raw[at]);
            at += 1;
        }
    }
    String::from_utf8(out).map_err(|_| NotALink)
}

/// A link written *inside a document*, resolved against that document.
///
/// **`..` pops here, where the host refuses it.** That is not an inconsistency: the host only ever
/// holds paths it handed out, so a `..` reaching it is a probe and gets refused; a document holds
/// paths a human wrote, where `../src/app.rs` is the ordinary way to name a sibling folder. A pop
/// with nothing left on the stack is the escapes-the-project-root rejection, and that is where the
/// two rules meet again.
pub fn resolve_relative(project: ProjectId, doc_path: &str, target: &str) -> Option<Destination> {
    let target = target.trim();
    if target.starts_with(SCHEME) {
        return target.parse().ok();
    }
    let lower = target.to_ascii_lowercase();
    // Somewhere else entirely. The caller hands these to the operating system.
    if ["http:", "https:", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return None;
    }
    if let Some(frag) = target.strip_prefix('#') {
        return Some(Destination {
            project,
            view: View::Ide {
                key: doc_path.to_string(),
            },
            locus: parse_locus(&decode(frag).ok()?).ok()?,
        });
    }

    let (path, frag) = match target.find('#') {
        Some(cut) => (&target[..cut], Some(&target[cut + 1..])),
        None => (target, None),
    };
    let path = decode(path).ok()?;
    if path.is_empty() || path.contains('\0') {
        return None;
    }

    // From the document's folder, not the document.
    let mut stack: Vec<String> = Vec::new();
    for component in Path::new(doc_path).components() {
        match component {
            Component::Normal(name) => stack.push(name.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    stack.pop();

    for component in Path::new(&path).components() {
        match component {
            Component::Normal(name) => stack.push(name.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                stack.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if stack.is_empty() {
        return None;
    }
    Some(Destination {
        project,
        view: View::Ide {
            key: stack.join("/"),
        },
        locus: match frag {
            Some(frag) => parse_locus(&decode(frag).ok()?).ok()?,
            None => None,
        },
    })
}
