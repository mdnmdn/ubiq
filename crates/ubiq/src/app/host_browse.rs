//! Phase 4b: filling the shared file picker from a remote host's own filesystem, before any
//! project exists on it.
//!
//! **Why this needs a state of its own beside [`crate::state::file_picker::FilePickerState`].**
//! That type holds a forest and nothing about where it came from — see its own module doc. Which
//! host is being browsed, which absolute paths are still owed a listing, and what to do once one
//! lands are all questions only this layer can answer, so they live here rather than teaching the
//! picker what a host is.
//!
//! **The staleness guard.** [`HostBrowseState`] is replaced wholesale every time
//! [`AppState::open_remote_project_picker`] runs — reopening the picker, even against the same
//! host, drops the old `pending` sets along with it. An answer that lands late for a path the
//! *current* session never asked about — because the session that asked has since been replaced —
//! finds nothing in `pending` to match and is quietly dropped in [`AppState::apply_host_dir_listing`]
//! and [`AppState::apply_host_dir_error`]. This is `state::remote::AttemptId`'s discipline, adapted
//! to a protocol that echoes back a canonical *path* rather than a request id: since every path
//! this forest ever asks about again is itself built from a path the host already canonicalised for
//! us (see [`children_of`]), the same path landing twice within one session always means the same
//! folder, and landing for a *different* session means nothing here recognises it.

use std::collections::HashSet;

use ubiq_proto::files::EntryKind;
use ubiq_proto::files::{HostDirEntry, HostPathError};

use crate::state::file_picker::{
    Commit, PickKind, PickerCount, PickerNode, PickerOwner, PickerRequest, PickerView,
};

use super::*;

/// One open "browse a host's filesystem" session. Lives beside `AppState::file_picker` rather than
/// inside it — see the module doc — and only while that picker is a host-project one.
pub struct HostBrowseState {
    pub host: HostId,
    /// The address the picker's title and up-affordance say — `RemoteConn::label`, copied in at
    /// open rather than looked up again every frame.
    pub label: String,
    /// The absolute path currently at the top of the tree. `None` until the very first listing
    /// lands — the dialog opens on an empty forest while that is in flight.
    pub root: Option<String>,
    /// `root`'s parent, so the dialog knows whether to offer walking up. Absent exactly when the
    /// host said its root listing had no parent — the filesystem root.
    pub parent: Option<String>,
    /// The most recent root listing was cut short by the host's entry ceiling.
    pub root_truncated: bool,
    /// A listing or a folder this session is still owed. `true` while waiting on the very first
    /// answer, whatever path it turns out to name.
    awaiting_root: bool,
    /// Absolute paths asked for as a *new root* — a walk up — and not yet answered.
    pending_roots: HashSet<String>,
    /// Absolute paths asked for as a folder *within* the current tree — a twisty opened — and not
    /// yet answered.
    pending_folders: HashSet<String>,
    /// What the host last refused, worded for the dialog to show directly. Cleared by the next
    /// successful answer, on the same rule the project picker's own banner clears on an
    /// `AddProject` that succeeds.
    pub error: Option<String>,
}

/// What one arriving answer, tagged with the [`HostRef`] it came from and the path it named,
/// turned out to be for the session it landed on.
///
/// Pure and total on purpose: [`HostBrowseState::classify`] is the whole of the staleness guard —
/// see the module doc — so it is written to need nothing but the session and the arrival, which is
/// what lets this file's own `#[cfg(test)]` module pin it down without a window, a picker, or a
/// live `Context`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arrival {
    /// This session's own current root — the very first answer, or a walk up it asked for.
    Root,
    /// A folder within this session's own tree that it asked to have listed.
    Folder,
    /// Not this session's news: a different host, or a path nothing here is waiting on — either
    /// because this session never asked, or because a session it replaced did.
    Stale,
}

impl HostBrowseState {
    fn new(host: HostId, label: String) -> Self {
        Self {
            host,
            label,
            root: None,
            parent: None,
            root_truncated: false,
            awaiting_root: true,
            pending_roots: HashSet::new(),
            pending_folders: HashSet::new(),
            error: None,
        }
    }

    /// Work out what an arriving path is to this session, consuming it from whichever `pending`
    /// set named it so the same path answered twice is not mistaken for a second live request.
    fn classify(&mut self, host: HostRef, path: Option<&str>) -> Arrival {
        if HostRef::Remote(self.host) != host {
            return Arrival::Stale;
        }
        if self.awaiting_root {
            self.awaiting_root = false;
            return Arrival::Root;
        }
        let Some(path) = path else {
            return Arrival::Stale;
        };
        if self.pending_roots.remove(path) {
            return Arrival::Root;
        }
        if self.pending_folders.remove(path) {
            return Arrival::Folder;
        }
        Arrival::Stale
    }
}

/// One [`HostDirEntry`] as a forest node, rooted under `parent` — the canonical path the listing
/// carrying it answered for, so a child's own path is itself something this session can later ask
/// the host about and recognise the answer to.
///
/// The separator follows the parent: the host may be Windows (`C:\…`) or Unix (`/…`) while this
/// window runs on either, so joining a verbatim parent with `/` — `\\?\C:\foo/bar` — must never
/// happen here. Whatever the host sent is what is extended.
fn child_path(parent: &str, name: &str) -> String {
    if parent.ends_with('/') || parent.ends_with('\\') {
        format!("{parent}{name}")
    } else if parent.contains('\\') {
        format!("{parent}\\{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// A listing's entries as picker nodes, in the order the host sent them — already directories
/// first and case-insensitive by name, on [`Message::HostDirListing`]'s own contract, so this never
/// re-sorts.
fn children_of(parent: &str, entries: Vec<HostDirEntry>) -> Vec<PickerNode> {
    entries
        .into_iter()
        .map(|entry| {
            let path = child_path(parent, &entry.name);
            match entry.kind {
                EntryKind::Dir => {
                    PickerNode::dir_unfetched(&entry.name, &path, entry.readable, entry.hidden)
                }
                // A file this dialog never lets anyone pick — `PickKind::Folders` hides it — but
                // still worth carrying faithfully rather than dropping, on the same "a tree with
                // rows missing is a tree that lies" reasoning `state::explorer` gives `Other`.
                EntryKind::File | EntryKind::Other => {
                    let mut node = PickerNode::file(&entry.name, &path, 0);
                    node.readable = entry.readable;
                    node.hidden = entry.hidden;
                    node
                }
            }
        })
        .collect()
}

impl AppState {
    /// Every remote host this window is attached to, for `ui::project_menu`'s "Open remote
    /// project…" row — the only reader, and the reason this returns owned strings rather than the
    /// borrowed `&str` [`Bus::remotes`] itself hands back: a menu row is built from `&AppState`
    /// and holds no borrow of it once drawn.
    /// Which host a message naming neither a pane nor a project currently reaches — for the Hosts
    /// section's dropdown to show as selected, and nothing else: painting a control from `Bus`
    /// state is not the same as `Bus` itself crossing into UI code, which stays exactly as
    /// forbidden as ever.
    pub fn active_host(&self) -> HostRef {
        self.bus.active()
    }

    pub fn remote_hosts(&self) -> Vec<(HostId, String)> {
        self.bus
            .remotes()
            .map(|(id, label)| (id, label.to_string()))
            .collect()
    }

    /// Which attached remote "Open remote project…" should offer. See
    /// [`crate::app::hosts::preferred_remote`], the pure function this delegates to, for the
    /// reasoning; this is only the plumbing that reads `Bus` state for it.
    pub fn preferred_remote_host(&self) -> Option<(HostId, String)> {
        crate::app::hosts::preferred_remote(self.active_host(), &self.remote_hosts())
    }

    /// Raise the folder picker over a remote host's own filesystem, to open whatever folder is
    /// chosen as a project there.
    ///
    /// `host` must be one [`Bus::remotes`] still names — see
    /// `ui::project_menu`'s "Open remote project…" row, the only caller, for how it gets one.
    pub fn open_remote_project_picker(
        &mut self,
        host: HostId,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.host_browse = Some(HostBrowseState::new(host, label.clone()));
        self.bus
            .send_to(HostRef::Remote(host), Message::BrowseHostDir { path: None });

        let request = PickerRequest::new(
            PickerOwner::HostProject,
            format!("Open a project on {label}"),
        )
        .kind(PickKind::Folders)
        .count(PickerCount::Single)
        .commit(Commit::OnButton);
        self.open_file_picker(request, Vec::new(), PickerView::Tree, window, cx);
    }

    /// Walk the tree's root up to its parent, re-fetching from there. A no-op with nothing to walk
    /// up to — the caller draws no such affordance at the filesystem root in the first place, this
    /// is just the same guard kept on the acting side too.
    pub fn walk_picker_up(&mut self, cx: &mut Context<Self>) {
        let Some(browse) = &mut self.host_browse else {
            return;
        };
        let Some(parent) = browse.parent.clone() else {
            return;
        };
        browse.pending_roots.insert(parent.clone());
        browse.error = None;
        self.bus.send_to(
            HostRef::Remote(browse.host),
            Message::BrowseHostDir { path: Some(parent) },
        );
        cx.notify();
    }

    /// After anything that may have expanded a folder — a twisty click, an arrow key stepping in,
    /// `enter` on a shut folder — ask the host for whatever the picker now needs and has not
    /// already asked for.
    ///
    /// A no-op outside a host-project picker: nothing here reads or writes `file_picker`'s forest
    /// directly, so this is safe to call after every interaction with the dialog rather than only
    /// the ones that could matter, which is simpler than each call site knowing which those are.
    pub(super) fn sync_host_browse(&mut self, _cx: &mut Context<Self>) {
        let (Some(browse), Some(picker)) = (&mut self.host_browse, &self.file_picker) else {
            return;
        };
        let asks: Vec<String> = picker
            .expanded_needing_load()
            .into_iter()
            .filter(|path| !browse.pending_folders.contains(path))
            .collect();
        if asks.is_empty() {
            return;
        }
        for path in &asks {
            browse.pending_folders.insert(path.clone());
        }
        let host = browse.host;
        for path in asks {
            self.bus.send_to(
                HostRef::Remote(host),
                Message::BrowseHostDir { path: Some(path) },
            );
        }
    }

    /// `Message::HostDirListing`'s landing, called from `wire.rs`'s host-browse family handler.
    ///
    /// Discarded outright when it is not this window's current session's news — a different host,
    /// or a path nothing in the current session's `pending` sets is waiting on. See the module doc
    /// for why a path match is the whole of the guard the wire affords.
    pub(super) fn apply_host_dir_listing(
        &mut self,
        host: HostRef,
        path: String,
        parent: Option<String>,
        entries: Vec<HostDirEntry>,
        truncated: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(browse) = &mut self.host_browse else {
            return;
        };

        match browse.classify(host, Some(&path)) {
            Arrival::Root => {
                browse.root = Some(path.clone());
                browse.parent = parent;
                browse.root_truncated = truncated;
                browse.error = None;
                if let Some(picker) = &mut self.file_picker {
                    picker.set_forest(children_of(&path, entries));
                }
                cx.notify();
            }
            Arrival::Folder => {
                browse.error = None;
                if let Some(picker) = &mut self.file_picker {
                    let children = children_of(&path, entries);
                    picker.fill_node(&path, children, truncated);
                }
                cx.notify();
            }
            // A listing this session never asked about, or asked about and already got — stale,
            // and silently dropped.
            Arrival::Stale => {}
        }
    }

    /// `Message::HostDirError`'s landing, on the same terms as
    /// [`Self::apply_host_dir_listing`].
    pub(super) fn apply_host_dir_error(
        &mut self,
        host: HostRef,
        path: Option<String>,
        error: HostPathError,
        cx: &mut Context<Self>,
    ) {
        let Some(browse) = &mut self.host_browse else {
            return;
        };
        if browse.classify(host, path.as_deref()) == Arrival::Stale {
            return;
        }
        browse.error = Some(error.to_string());
        cx.notify();
    }

    /// Take the browse session down. Called alongside the picker's own cancel and commit, since
    /// neither leaves anything here for a stale answer to land on afterwards.
    pub(super) fn close_host_browse(&mut self) {
        self.host_browse = None;
    }

    /// The host browse family: `BrowseHostDir`'s two answers.
    ///
    /// Answers with the message when it belongs to another family, on the same convention every
    /// other `receive_*` in `wire.rs` follows.
    pub(super) fn receive_host_browse(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::HostDirListing {
                path,
                parent,
                entries,
                truncated,
            } => self.apply_host_dir_listing(host, path, parent, entries, truncated, cx),
            Message::HostDirError { path, error } => {
                self.apply_host_dir_error(host, path, error, cx)
            }
            other => return Some(other),
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two distinct hosts, the way `Bus::register_remote` mints them — `HostId`'s field is private
    /// even to this module, so a real `Bus` is the only way to get one to test against.
    fn two_hosts() -> (HostId, HostId) {
        let (local, _local_end) = ubiq_proto::bus::detached();
        let mut bus = Bus::new(local);
        let (a, _) = bus.register_remote(ubiq_proto::bus::detached().0, "host-a".to_string());
        let (b, _) = bus.register_remote(ubiq_proto::bus::detached().0, "host-b".to_string());
        (a, b)
    }

    /// The very first answer is the root, whatever path it turns out to name — the whole reason
    /// `BrowseHostDir { path: None }` is answerable at all.
    #[test]
    fn the_first_answer_is_the_root_however_it_is_pathed() {
        let (host, _) = two_hosts();
        let mut browse = HostBrowseState::new(host, "example".to_string());

        assert_eq!(
            browse.classify(HostRef::Remote(host), Some("/home/mdn")),
            Arrival::Root
        );
        // Answered once: a second listing for the same path is no longer "the first answer" and
        // matches nothing else either.
        assert_eq!(
            browse.classify(HostRef::Remote(host), Some("/home/mdn")),
            Arrival::Stale
        );
    }

    /// An answer tagged with a host this session is not browsing is discarded before anything
    /// about its path is even looked at — the exact race the module doc calls out: host B's late
    /// answer must not land on a picker now pointed at host A.
    #[test]
    fn an_answer_from_the_wrong_host_is_always_stale() {
        let (a, b) = two_hosts();
        let mut browse = HostBrowseState::new(a, "example".to_string());
        browse.classify(HostRef::Remote(a), Some("/home"));

        assert_eq!(
            browse.classify(HostRef::Remote(b), Some("/home")),
            Arrival::Stale
        );
    }

    /// A folder asked about by its twisty is recognised once, and a repeat of the same answer —
    /// or one for a folder never asked about at all — is not.
    #[test]
    fn a_folder_is_recognised_once_and_only_once() {
        let (host, _) = two_hosts();
        let mut browse = HostBrowseState::new(host, "example".to_string());
        browse.classify(HostRef::Remote(host), Some("/home/mdn"));
        browse
            .pending_folders
            .insert("/home/mdn/projects".to_string());

        assert_eq!(
            browse.classify(HostRef::Remote(host), Some("/home/mdn/projects")),
            Arrival::Folder
        );
        assert_eq!(
            browse.classify(HostRef::Remote(host), Some("/home/mdn/projects")),
            Arrival::Stale
        );
        assert_eq!(
            browse.classify(HostRef::Remote(host), Some("/etc")),
            Arrival::Stale
        );
    }

    /// Walking up asks about a specific path too, and is told apart from a folder going deeper
    /// only by which `pending` set named it — both answer `Arrival::Root` /
    /// `Arrival::Folder` on exactly the same shape of call.
    #[test]
    fn walking_up_is_a_root_arrival_not_a_folder_one() {
        let (host, _) = two_hosts();
        let mut browse = HostBrowseState::new(host, "example".to_string());
        browse.classify(HostRef::Remote(host), Some("/home/mdn"));
        browse.pending_roots.insert("/home".to_string());

        assert_eq!(
            browse.classify(HostRef::Remote(host), Some("/home")),
            Arrival::Root
        );
    }

    /// A host-side failure with no path at all — the host could not even find a default place to
    /// start — is recognised only while the very first answer is still owed.
    #[test]
    fn a_pathless_failure_is_recognised_only_as_the_opening_answer() {
        let (host, _) = two_hosts();
        let mut fresh = HostBrowseState::new(host, "example".to_string());
        assert_eq!(fresh.classify(HostRef::Remote(host), None), Arrival::Root);

        let mut answered = HostBrowseState::new(host, "example".to_string());
        answered.classify(HostRef::Remote(host), Some("/home"));
        assert_eq!(
            answered.classify(HostRef::Remote(host), None),
            Arrival::Stale
        );
    }

    /// Unix parents join with `/`, without doubling the root's own.
    #[test]
    fn a_unix_child_joins_with_a_forward_slash() {
        assert_eq!(child_path("/home/mdn", "works"), "/home/mdn/works");
        assert_eq!(child_path("/", "home"), "/home");
    }

    /// Windows parents join with `\` — the host may be Windows while this window is not, so the
    /// separator follows the parent rather than the local platform. Joining with `/` is what
    /// produced `\\?\C:\foo/bar` and os error 123.
    #[test]
    fn a_windows_child_joins_with_a_backslash() {
        assert_eq!(child_path(r"C:\works", "ubiq"), r"C:\works\ubiq");
        assert_eq!(child_path(r"C:\", "works"), r"C:\works");
    }
}
