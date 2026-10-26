//! The system `ssh`/`scp` argv an [`SshProfile`] stands for — shared by every caller that shells to
//! the system client: the interface's drone dial and deployer (`crates/ubiq/src/app/ssh_connect.rs`)
//! and the host's database tunnel (`crates/ubiq-host/src/db/tunnel.rs`).
//!
//! It lives in the contract crate only because that is the one crate both halves depend on, and
//! the profile it reads is defined here too ([`crate::settings`]). It builds a [`Command`] and
//! starts nothing: no process handle, pipe or wait ever comes out of this module.

use std::process::Command;

use crate::settings::{SshAuth, SshProfile};

/// How long `ssh` may spend on its own connection before giving up. A wrong address must not
/// hang a caller for whatever the platform's own ceiling is.
pub const CONNECT_TIMEOUT_SECS: u64 = 6;

/// The environment variable that puts the binary into askpass mode, and names which profile's
/// secret is wanted. Read by `ubiq_app::askpass`; a reference, never material.
pub const ASKPASS_PROFILE_VAR: &str = "UBIQ_ASKPASS_PROFILE";

/// Which config root the helper opens its secret store under. Absent, the helper resolves its own
/// the way any other launch does.
pub const ASKPASS_ROOT_VAR: &str = "UBIQ_ASKPASS_ROOT";

/// What `ssh` is pointed at: the alias for a `ConfigAlias` profile, the host otherwise. The user
/// rides as `-l` rather than as `user@host`, so a name with an `@` in it is not ambiguous.
pub fn target(profile: &SshProfile) -> String {
    profile.host.trim().to_string()
}

/// [`target`], or `None` when it cannot be put in an argv: empty, read by `ssh` as an option of
/// its own (a leading `-`), or carrying a control character.
pub fn typable_target(profile: &SshProfile) -> Option<String> {
    let target = target(profile);
    let refused =
        target.is_empty() || target.starts_with('-') || target.chars().any(|c| c.is_control());
    (!refused).then_some(target)
}

/// Which program [`apply_connection_options`] is building argv for. `ssh` and `scp` read
/// identical `-o` options but spell the port and the user differently — `-p`/`-l` against
/// `-P`/`-o User=`, `scp` having no `-l` of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionKind {
    Ssh,
    Scp,
}

/// The profile fields `ssh` and `scp` read the same way: port, user, identity file, connect
/// timeout and batch mode — everything [`SshAuth::ConfigAlias`] defers to `~/.ssh/config` instead.
/// One reading of the profile for every caller, so a dial, an upload and a tunnel never drift.
pub fn apply_connection_options(command: &mut Command, profile: &SshProfile, kind: ConnectionKind) {
    if matches!(profile.auth, SshAuth::ConfigAlias) {
        return;
    }
    match kind {
        ConnectionKind::Ssh => {
            command.arg("-p").arg(profile.port.to_string());
            if !profile.user.trim().is_empty() {
                command.arg("-l").arg(profile.user.trim());
            }
        }
        ConnectionKind::Scp => {
            command.arg("-P").arg(profile.port.to_string());
            if !profile.user.trim().is_empty() {
                command
                    .arg("-o")
                    .arg(format!("User={}", profile.user.trim()));
            }
        }
    }
    if let SshAuth::KeyFile { path, .. } = &profile.auth
        && !path.trim().is_empty()
    {
        command.arg("-i").arg(path.trim());
    }
    command
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"));
    // Batch mode where there is nothing to answer a prompt with: an agent key or an unencrypted
    // key file either works or does not, and failing at once beats blocking on a prompt no
    // terminal will ever see. Where a secret *is* filed, batch mode would refuse the askpass
    // helper that supplies it, so it is left off.
    if !profile.auth.has_secret() {
        command.arg("-o").arg("BatchMode=yes");
    }
}

/// Point `ssh` at Ubiq's own binary for the password prompt, when the profile has a secret filed.
///
/// **The material is never in `argv`, never in a file, and never on the bus.** What crosses here
/// is the profile *id* in the child's environment. The helper — this same executable in the mode
/// `ubiq_app::askpass` answers — opens the host's secret store under the config root and writes the
/// material on its own standard output, which is where OpenSSH reads an askpass answer from (`D124`).
pub fn apply_askpass(command: &mut Command, profile: &SshProfile, config_root: Option<&str>) {
    if !profile.auth.has_secret() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        // Without a path to this binary there is no helper to point at. `ssh` then prompts, finds
        // no terminal, and fails with its own sentence — which is the right failure to show.
        tracing::warn!("no path to this executable: ssh will have no askpass helper");
        return;
    };
    command.env("SSH_ASKPASS", exe);
    // OpenSSH 8.4 and later: use the helper whatever the terminal and `DISPLAY` say. Older
    // clients consult `DISPLAY` instead, which is why one is set below when there is none.
    command.env("SSH_ASKPASS_REQUIRE", "force");
    if std::env::var_os("DISPLAY").is_none() {
        command.env("DISPLAY", ":0");
    }
    command.env(ASKPASS_PROFILE_VAR, profile.id.to_string());
    if let Some(root) = config_root {
        command.env(ASKPASS_ROOT_VAR, root);
    }
}

/// A session of its own, so an interrupt aimed at the terminal Ubiq was launched from does not
/// take the child with it. `SSH_ASKPASS_REQUIRE=force` is what actually keeps the password off a
/// terminal; this only keeps the child out of the window's job control.
pub fn put_in_own_session(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// `ssh`/`scp`/`taskkill` here all talk over pipes or exit codes, never a screen. Windows would
/// still pop a console window for a console-subsystem child like these: the app has already freed
/// its own console (`ubiq_app::detach_console`), and a child with none to inherit gets a new one.
#[cfg(windows)]
pub fn no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}
#[cfg(not(windows))]
pub fn no_window(_command: &mut Command) {}

/// The argv of a local port forward: `ssh … -N -T -L 127.0.0.1:<local_port>:<remote_host>:<remote_port>
/// <target>`, over the same [`apply_connection_options`] and [`apply_askpass`] a dial reads.
///
/// `-N` runs no remote command; `ExitOnForwardFailure` makes a port that will not bind an exit
/// rather than a silently dead tunnel. A [`SshAuth::ConfigAlias`] profile gets `BatchMode=yes`
/// here as well — it can have no secret filed, so a prompt is one nothing will ever answer.
/// `remote_host` is named as the far machine resolves it, so `localhost` is *its* loopback. Streams
/// are left for the caller to pipe. `None` when the target or the remote host cannot be put in an
/// argv.
pub fn tunnel_command(
    profile: &SshProfile,
    config_root: Option<&str>,
    local_port: u16,
    remote_host: &str,
    remote_port: u16,
) -> Option<Command> {
    let target = typable_target(profile)?;
    let remote_host = remote_host.trim();
    if remote_host.is_empty() || remote_host.chars().any(|c| c.is_control() || c == '/') {
        return None;
    }
    // An IPv6 literal is bracketed, or its colons read as the forward's own separators.
    let remote_host = match remote_host.contains(':') && !remote_host.starts_with('[') {
        true => format!("[{remote_host}]"),
        false => remote_host.to_string(),
    };
    let mut command = Command::new("ssh");
    apply_connection_options(&mut command, profile, ConnectionKind::Ssh);
    if matches!(profile.auth, SshAuth::ConfigAlias) {
        command.arg("-o").arg("BatchMode=yes");
    }
    command
        .arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{local_port}:{remote_host}:{remote_port}"
        ))
        .arg(target);
    apply_askpass(&mut command, profile, config_root);
    put_in_own_session(&mut command);
    no_window(&mut command);
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SshProfileId;

    fn profile(auth: SshAuth) -> SshProfile {
        SshProfile {
            id: SshProfileId::generate(),
            name: "bastion".into(),
            host: "bastion.example".into(),
            port: 2222,
            user: "ops".into(),
            auth,
        }
    }

    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_tunnel_forwards_a_loopback_port_through_the_profile() {
        let command = tunnel_command(&profile(SshAuth::Agent), None, 40123, "db.internal", 5432)
            .expect("a typable profile builds a tunnel");
        let args = args(&command);
        assert_eq!(command.get_program(), "ssh");
        for expected in [
            "-p",
            "2222",
            "-l",
            "ops",
            "BatchMode=yes",
            "-N",
            "ExitOnForwardFailure=yes",
            "127.0.0.1:40123:db.internal:5432",
        ] {
            assert!(
                args.contains(&expected.to_string()),
                "{expected} in {args:?}"
            );
        }
        assert_eq!(args.last().map(String::as_str), Some("bastion.example"));
    }

    #[test]
    fn a_tunnel_brackets_ipv6_and_refuses_what_is_not_a_host() {
        let ipv6 = tunnel_command(&profile(SshAuth::Agent), None, 1, "::1", 3306).unwrap();
        assert!(args(&ipv6).contains(&"127.0.0.1:1:[::1]:3306".to_string()));
        // A Postgres socket directory is not a TCP host.
        assert!(tunnel_command(&profile(SshAuth::Agent), None, 1, "/var/run/pg", 5432).is_none());
        let mut dashed = profile(SshAuth::Agent);
        dashed.host = "-oProxyCommand=x".into();
        assert!(tunnel_command(&dashed, None, 1, "db", 5432).is_none());
    }

    #[test]
    fn a_config_alias_tunnel_is_still_batch_and_a_filed_secret_is_not() {
        let alias = tunnel_command(&profile(SshAuth::ConfigAlias), None, 1, "db", 5432).unwrap();
        let alias = args(&alias);
        assert!(alias.contains(&"BatchMode=yes".to_string()));
        assert!(
            !alias.contains(&"-p".to_string()),
            "the alias governs the port"
        );
        let secret = tunnel_command(
            &profile(SshAuth::Password { has_password: true }),
            Some("/config/root"),
            1,
            "db",
            5432,
        )
        .unwrap();
        assert!(!args(&secret).contains(&"BatchMode=yes".to_string()));
        assert!(
            secret
                .get_envs()
                .any(|(key, _)| key == std::ffi::OsStr::new(ASKPASS_PROFILE_VAR))
        );
    }
}
