//! *Open in Ubiq* in the desktop's file-manager context menu.
//!
//! On Windows that is three per-user verbs under `HKEY_CURRENT_USER\Software\Classes` — one for
//! any file (`*`), one for a folder (`Directory`) and one for the empty space inside an open folder
//! (`Directory\Background`) — each naming the running executable, so a pick in Explorer launches
//! `ubiq <path>`, and `ubiq-app`'s handoff hands that path to the window already up. Per user
//! because the current user is who asked: nothing here needs elevation, and nothing reaches another
//! account's menu.
//!
//! **The machine is the record.** Nothing is stored about whether the entries are installed: every
//! answer reads the registry back, so a key deleted by hand, or written by another build, is drawn
//! as it is. Each entry carries a marker value naming the executable it launches, which answers
//! both questions this module has — *did Ubiq write this key*, so *Remove* never deletes an entry of
//! the user's own, and *which build does it launch*, which is what makes an entry left behind by a
//! moved or replaced application `stale` rather than mysterious. `cli_shortcut`'s `ubiq-target:`
//! line is the same idea in a file.
//!
//! Every key and every path belongs to the host. The interface asks for one of three actions and is
//! told what was found. On any other platform the answer is `supported: false` and nothing changes.

#[cfg(not(windows))]
use ubiq_proto::messages::{Message, ShellIntegrationAction};

/// Look, write or delete, and answer what is there afterwards either way.
#[cfg(not(windows))]
pub fn handle(_action: ShellIntegrationAction) -> Message {
    Message::ShellIntegrationState {
        supported: false,
        installed: false,
        stale: false,
        command: None,
        error: None,
    }
}

#[cfg(windows)]
pub use self::registry::handle;

#[cfg(windows)]
mod registry {
    use std::io;

    use ubiq_proto::messages::{Message, ShellIntegrationAction};
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    /// What the menu says.
    pub(super) const LABEL: &str = "Open in Ubiq";

    /// The value every entry Ubiq wrote carries: the executable it launches.
    pub(super) const MARKER: &str = "UbiqTarget";

    /// Where each entry lives under `Software\Classes`, and the placeholder its command passes.
    ///
    /// `%1` is the item that was clicked; a folder's background has no item, and `%V` is the folder
    /// it is the background of.
    pub(super) const ENTRIES: [(&str, &str); 3] = [
        (r"*\shell\Ubiq", "%1"),
        (r"Directory\shell\Ubiq", "%1"),
        (r"Directory\Background\shell\Ubiq", "%V"),
    ];

    /// Look, write or delete, and answer what is there afterwards either way.
    pub fn handle(action: ShellIntegrationAction) -> Message {
        let exe = target();
        let classes = classes();
        let error = match action {
            ShellIntegrationAction::Query => None,
            ShellIntegrationAction::Install => match (&classes, &exe) {
                (Ok(classes), Ok(exe)) => install(classes, exe).err(),
                (Err(error), _) | (_, Err(error)) => Some(error.clone()),
            },
            ShellIntegrationAction::Remove => match &classes {
                Ok(classes) => remove(classes).err(),
                Err(error) => Some(error.clone()),
            },
        };
        if action != ShellIntegrationAction::Query {
            notify_shell();
        }
        match &classes {
            Ok(classes) => state(classes, exe.as_deref().ok(), error),
            Err(cannot) => Message::ShellIntegrationState {
                supported: true,
                installed: false,
                stale: false,
                command: exe.as_deref().ok().map(|exe| command(exe, "%1")),
                error: error.or_else(|| Some(cannot.clone())),
            },
        }
    }

    /// The running executable, as the wire spells a path — no `\\?\` prefix, which neither
    /// Explorer nor a reader of the settings page wants to see.
    fn target() -> Result<String, String> {
        std::env::current_exe()
            .map(|exe| crate::host_path::wire_string(&exe))
            .map_err(|error| format!("no executable path: {error}"))
    }

    /// The current user's `Software\Classes`, opened for writing. It exists on every account, so
    /// creating it only ever opens it.
    fn classes() -> Result<RegKey, String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(r"Software\Classes")
            .map(|(key, _)| key)
            .map_err(|error| {
                format!("could not open HKEY_CURRENT_USER\\Software\\Classes: {error}")
            })
    }

    /// The command line an entry runs: the executable, and the item quoted so a path with spaces
    /// stays one argument.
    pub(super) fn command(exe: &str, placeholder: &str) -> String {
        format!("\"{exe}\" \"{placeholder}\"")
    }

    /// The executable an entry's marker names, if the key is there and Ubiq wrote it.
    pub(super) fn marked(classes: &RegKey, path: &str) -> Option<String> {
        classes
            .open_subkey(path)
            .ok()?
            .get_value::<String, _>(MARKER)
            .ok()
    }

    /// Write all three entries for `exe`, replacing any Ubiq wrote before. The half that takes its
    /// root rather than finding it, so the tests run against a throwaway key.
    pub(super) fn install(classes: &RegKey, exe: &str) -> Result<(), String> {
        for (path, placeholder) in ENTRIES {
            write_entry(classes, path, exe, placeholder)
                .map_err(|error| format!("could not write {path}: {error}"))?;
        }
        Ok(())
    }

    fn write_entry(classes: &RegKey, path: &str, exe: &str, placeholder: &str) -> io::Result<()> {
        let (key, _) = classes.create_subkey(path)?;
        key.set_value("", &LABEL)?;
        key.set_value("Icon", &exe)?;
        key.set_value(MARKER, &exe)?;
        let (run, _) = key.create_subkey("command")?;
        run.set_value("", &command(exe, placeholder))
    }

    /// Delete every entry carrying the marker. One without it is the user's, and stays.
    pub(super) fn remove(classes: &RegKey) -> Result<(), String> {
        for (path, _) in ENTRIES {
            if marked(classes, path).is_none() {
                continue;
            }
            classes
                .delete_subkey_all(path)
                .map_err(|error| format!("could not remove {path}: {error}"))?;
        }
        Ok(())
    }

    /// What is there, read back rather than remembered.
    ///
    /// Installed when any entry carries the marker; stale when one of them launches another build,
    /// or one of the three is missing — either way *Update* is what puts it right. Paths compare
    /// without case, as Windows does.
    pub(super) fn state(classes: &RegKey, exe: Option<&str>, error: Option<String>) -> Message {
        let marks: Vec<Option<String>> = ENTRIES
            .iter()
            .map(|(path, _)| marked(classes, path))
            .collect();
        let installed = marks.iter().any(Option::is_some);
        let stale = installed
            && marks.iter().any(|mark| match (mark, exe) {
                (Some(mark), Some(exe)) => !mark.eq_ignore_ascii_case(exe),
                (Some(_), None) => false,
                (None, _) => true,
            });
        Message::ShellIntegrationState {
            supported: true,
            installed,
            stale,
            command: exe.map(|exe| command(exe, "%1")),
            error,
        }
    }

    /// Tell Explorer the associations changed, so an open window's menu does not wait for a
    /// restart. Linked straight from `shell32`, on `detach_console`'s pattern in `ubiq-app`; the
    /// call has no result to fail with.
    fn notify_shell() {
        use std::ffi::c_void;

        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
        }
        const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
        const SHCNF_IDLIST: u32 = 0;
        unsafe {
            SHChangeNotify(
                SHCNE_ASSOCCHANGED,
                SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            );
        }
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn another_platform_is_unsupported_and_changes_nothing() {
        for action in [
            ShellIntegrationAction::Query,
            ShellIntegrationAction::Install,
            ShellIntegrationAction::Remove,
        ] {
            assert!(matches!(
                handle(action),
                Message::ShellIntegrationState {
                    supported: false,
                    installed: false,
                    stale: false,
                    command: None,
                    error: None,
                }
            ));
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::registry::*;
    use ubiq_proto::messages::Message;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const EXE: &str = r"C:\Program Files\Ubiq\ubiq.exe";
    const OTHER: &str = r"D:\builds\target\debug\ubiq.exe";

    /// A throwaway key standing in for `Software\Classes`, deleted with everything under it when
    /// the test ends — pass or fail.
    struct Root {
        path: String,
        key: RegKey,
    }

    impl Root {
        fn new(name: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = format!(
                r"Software\UbiqTests\shell-integration-{name}-{}-{nanos}",
                std::process::id()
            );
            let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(&path)
                .unwrap();
            Root { path, key }
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(&self.path);
            // Only goes when empty, so a test running beside this one keeps its own.
            let _ = hkcu.delete_subkey(r"Software\UbiqTests");
        }
    }

    fn read(root: &Root, path: &str, value: &str) -> String {
        root.key
            .open_subkey(path)
            .unwrap()
            .get_value::<String, _>(value)
            .unwrap()
    }

    fn installed_and_stale(message: Message) -> (bool, bool) {
        match message {
            Message::ShellIntegrationState {
                supported,
                installed,
                stale,
                ..
            } => {
                assert!(supported);
                (installed, stale)
            }
            other => panic!("answered with something else: {other:?}"),
        }
    }

    #[test]
    fn installs_three_marked_entries_with_their_commands() {
        let root = Root::new("install");
        install(&root.key, EXE).unwrap();

        for (path, placeholder) in ENTRIES {
            assert_eq!(read(&root, path, ""), LABEL);
            assert_eq!(read(&root, path, "Icon"), EXE);
            assert_eq!(read(&root, path, MARKER), EXE);
            assert_eq!(
                read(&root, &format!(r"{path}\command"), ""),
                format!("\"{EXE}\" \"{placeholder}\"")
            );
        }
        assert_eq!(
            read(&root, r"Directory\Background\shell\Ubiq\command", ""),
            format!("\"{EXE}\" \"%V\"")
        );
        assert_eq!(
            installed_and_stale(state(&root.key, Some(EXE), None)),
            (true, false)
        );
    }

    #[test]
    fn nothing_written_reads_as_not_installed() {
        let root = Root::new("empty");
        let message = state(&root.key, Some(EXE), None);
        if let Message::ShellIntegrationState { command, .. } = &message {
            assert_eq!(
                command.as_deref(),
                Some(format!("\"{EXE}\" \"%1\"").as_str())
            );
        }
        assert_eq!(installed_and_stale(message), (false, false));
    }

    #[test]
    fn entries_for_another_build_are_stale_until_updated() {
        let root = Root::new("stale");
        install(&root.key, OTHER).unwrap();
        assert_eq!(
            installed_and_stale(state(&root.key, Some(EXE), None)),
            (true, true)
        );

        install(&root.key, EXE).unwrap();
        assert_eq!(
            installed_and_stale(state(&root.key, Some(EXE), None)),
            (true, false)
        );
        // Windows paths are not case-sensitive, and neither is the comparison.
        assert_eq!(
            installed_and_stale(state(&root.key, Some(&EXE.to_uppercase()), None)),
            (true, false)
        );
    }

    #[test]
    fn a_missing_entry_is_stale() {
        let root = Root::new("partial");
        install(&root.key, EXE).unwrap();
        root.key
            .delete_subkey_all(r"Directory\Background\shell\Ubiq")
            .unwrap();
        assert_eq!(
            installed_and_stale(state(&root.key, Some(EXE), None)),
            (true, true)
        );
    }

    #[test]
    fn remove_takes_ours_and_leaves_theirs() {
        let root = Root::new("remove");
        install(&root.key, EXE).unwrap();
        remove(&root.key).unwrap();
        for (path, _) in ENTRIES {
            assert!(root.key.open_subkey(path).is_err(), "{path} is gone");
        }
        assert_eq!(
            installed_and_stale(state(&root.key, Some(EXE), None)),
            (false, false)
        );

        // A `Ubiq` verb the user wrote themselves carries no marker, and is not ours to delete.
        let (theirs, _) = root.key.create_subkey(r"*\shell\Ubiq").unwrap();
        theirs.set_value("", &"Mine").unwrap();
        remove(&root.key).unwrap();
        assert_eq!(read(&root, r"*\shell\Ubiq", ""), "Mine");
        assert!(marked(&root.key, r"*\shell\Ubiq").is_none());
    }
}
