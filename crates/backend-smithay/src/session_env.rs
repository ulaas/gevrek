//! gevrek as the session a user logs into: what it tells the programs it
//! starts, the service manager and D-Bus.
//!
//! Every program started in the session must know the display (the
//! Wayland socket, and the X11 display for X11 programs) and which desktop
//! it is in. Programs gevrek starts itself inherit that from its own
//! environment (`set_own`). Programs started for the session by systemd or
//! by D-Bus activation (portals, the polkit agent, notification daemons,
//! autostart entries) only know what the user's service manager and the
//! D-Bus daemon were told: with `--session` gevrek tells them (`export`)
//! once its socket is up, says that it is ready (`notify`), and takes the
//! variables back when it ends (`unexport`).
//!
//! Only a gevrek started as the session does the last three: a nested or
//! headless one must not rewrite the environment of the session it runs in.

use std::process::Command;

/// What the session is called (`XDG_CURRENT_DESKTOP`, the name of the
/// portal configuration `gevrek-portals.conf`).
pub const DESKTOP_NAME: &str = "gevrek";

/// The variables the service manager and D-Bus are told.
pub const EXPORTED: [&str; 7] = [
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "XDG_SESSION_TYPE",
    "XCURSOR_THEME",
    "XCURSOR_SIZE",
];

/// Set, in gevrek's own environment, what every program it starts
/// inherits: the desktop's name, the session type, and the variables of
/// the configuration (`desktop.session.environment`; an empty value
/// removes one).
pub fn set_own(session: &gevrek_model::session::SessionConfig) {
    std::env::set_var("XDG_CURRENT_DESKTOP", DESKTOP_NAME);
    std::env::set_var("XDG_SESSION_DESKTOP", DESKTOP_NAME);
    std::env::set_var("XDG_SESSION_TYPE", "wayland");
    for (name, value) in &session.environment {
        if value.is_empty() {
            std::env::remove_var(name);
        } else {
            std::env::set_var(name, value);
        }
    }
}

/// The names to export: the fixed ones that are set, and those of the
/// configuration.
pub fn exported_names(session: &gevrek_model::session::SessionConfig) -> Vec<String> {
    EXPORTED
        .iter()
        .map(|name| name.to_string())
        .chain(session.environment.keys().cloned())
        .filter(|name| std::env::var_os(name).is_some())
        .collect()
}

/// The commands that tell the service manager and D-Bus the variables.
pub fn export_commands(names: &[String]) -> Vec<Vec<String>> {
    if names.is_empty() {
        return Vec::new();
    }
    let mut systemd = vec![
        "systemctl".to_string(),
        "--user".to_string(),
        "import-environment".to_string(),
    ];
    systemd.extend(names.iter().cloned());
    let mut dbus = vec!["dbus-update-activation-environment".to_string()];
    dbus.extend(names.iter().cloned());
    vec![systemd, dbus]
}

/// The command that takes them back when the session ends.
pub fn unexport_command(names: &[String]) -> Vec<String> {
    let mut command = vec![
        "systemctl".to_string(),
        "--user".to_string(),
        "unset-environment".to_string(),
    ];
    command.extend(names.iter().cloned());
    command
}

fn run(command: &[String]) {
    let Some((program, arguments)) = command.split_first() else {
        return;
    };
    match Command::new(program).args(arguments).status() {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!("{} ended with {status}", command.join(" ")),
        // Not every system has both (no systemd, no D-Bus tool): the
        // programs gevrek starts itself are told anyway.
        Err(error) => log::debug!("{program}: {error}"),
    }
}

/// Tell the service manager and D-Bus the session's variables.
pub fn export(session: &gevrek_model::session::SessionConfig) {
    let names = exported_names(session);
    log::info!(
        "Session environment for systemd and D-Bus: {}",
        names.join(" ")
    );
    for command in export_commands(&names) {
        run(&command);
    }
}

/// Take the variables back (the next session must not inherit them).
pub fn unexport(session: &gevrek_model::session::SessionConfig) {
    let mut names: Vec<String> = EXPORTED.iter().map(|name| name.to_string()).collect();
    names.extend(session.environment.keys().cloned());
    run(&unexport_command(&names));
}

/// Tell the service manager something about this service (`READY=1`,
/// `STOPPING=1`), the way `sd_notify` does: a datagram to the socket named
/// in `NOTIFY_SOCKET`. Nothing happens when gevrek is not run as a service.
pub fn notify(state: &str) {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    let path = path.to_string_lossy().into_owned();
    let result = (|| -> std::io::Result<()> {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::net::{SocketAddr, UnixDatagram};
        let socket = UnixDatagram::unbound()?;
        // A name that starts with @ is in the abstract namespace.
        let address = match path.strip_prefix('@') {
            Some(name) => SocketAddr::from_abstract_name(name.as_bytes())?,
            None => SocketAddr::from_pathname(&path)?,
        };
        socket.send_to_addr(state.as_bytes(), &address)?;
        Ok(())
    })();
    if let Err(error) = result {
        log::warn!("The service manager was not told {state}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_name_every_variable_once() {
        let names = vec!["WAYLAND_DISPLAY".to_string(), "TERMINAL".to_string()];
        let commands = export_commands(&names);
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0],
            [
                "systemctl",
                "--user",
                "import-environment",
                "WAYLAND_DISPLAY",
                "TERMINAL"
            ]
        );
        assert_eq!(
            commands[1],
            [
                "dbus-update-activation-environment",
                "WAYLAND_DISPLAY",
                "TERMINAL"
            ]
        );
        assert_eq!(
            unexport_command(&names),
            [
                "systemctl",
                "--user",
                "unset-environment",
                "WAYLAND_DISPLAY",
                "TERMINAL"
            ]
        );
        assert!(export_commands(&[]).is_empty());
    }
}
