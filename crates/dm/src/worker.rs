//! One session's worker. The service starts one for the greeter and one
//! for each login; it is the process PAM and logind know the session by.
//!
//! It reads its order from its standard input (one line), checks the
//! password where the order is a person's, says on its standard output
//! whether that held, opens the session (logind then makes it a session
//! of the seat on the order's console), starts the session's program as
//! the user, and stays until that ends: then the session is closed and
//! the worker ends with the program's own result.

use gevrek_dm::wire::{Answer, Kind, Order};
use std::io::{BufRead, Write};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use users::os::unix::UserExt;

/// The PAM service a session of this kind goes through.
fn service_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Greeter => "gevrek-greeter",
        Kind::User => "gevrek-dm",
    }
}

/// What a session is told of itself: logind reads these when PAM opens
/// the session (which seat, which console, what kind), and the session's
/// programs read them afterwards.
pub fn session_environment(order: &Order, uid: u32) -> Vec<(String, String)> {
    let mut environment = vec![
        ("XDG_SEAT".to_string(), "seat0".to_string()),
        ("XDG_VTNR".to_string(), order.console.to_string()),
        ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        (
            "XDG_SESSION_CLASS".to_string(),
            match order.kind {
                Kind::Greeter => "greeter",
                Kind::User => "user",
            }
            .to_string(),
        ),
        ("XDG_SESSION_DESKTOP".to_string(), "gevrek".to_string()),
        ("XDG_CURRENT_DESKTOP".to_string(), "gevrek".to_string()),
        ("XDG_RUNTIME_DIR".to_string(), format!("/run/user/{uid}")),
        // gevrek leaves the display as it finds it until its first
        // frame: no switching off between the login and the desktop.
        ("GEVREK_KEEP_DISPLAY".to_string(), "1".to_string()),
    ];
    environment.extend(order.environment.iter().cloned());
    environment
}

fn say(answer: &Answer) {
    let mut out = std::io::stdout().lock();
    if let Err(error) = writeln!(out, "{}", serde_json::to_string(answer).unwrap_or_default())
        .and_then(|()| out.flush())
    {
        log::debug!("the service was not told whether the login held: {error}");
    }
}

/// The session's number in what `/proc/self/cgroup` says
/// (`…/session-12.scope`).
pub fn session_of(cgroup: &str) -> Option<String> {
    let after = cgroup.rsplit_once("/session-")?.1;
    let id = after.split_once(".scope")?.0;
    (!id.is_empty() && id.chars().all(|letter| letter.is_ascii_alphanumeric()))
        .then(|| id.to_string())
}

/// The process of the session (the leader of its group), for the
/// handler of the signal that ends the worker.
static SESSION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

extern "C" fn pass_on(_: libc::c_int) {
    let session = SESSION.load(std::sync::atomic::Ordering::SeqCst);
    if session > 0 {
        // SAFETY: sending a signal is allowed in a handler.
        unsafe {
            libc::kill(-session, libc::SIGTERM);
        }
    }
}

/// The service asks the login's gevrek to go under its veil (the
/// desktop's console is about to be shown): handed on to it alone.
extern "C" fn pass_on_leave(_: libc::c_int) {
    let session = SESSION.load(std::sync::atomic::Ordering::SeqCst);
    if session > 0 {
        // SAFETY: sending a signal is allowed in a handler.
        unsafe {
            libc::kill(session, libc::SIGUSR1);
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let mut order: Order = serde_json::from_str(&line)
        .map_err(|error| anyhow::anyhow!("the order was not understood: {error}"))?;
    // (The order carries the password: only its parts that are not are
    // said.)
    log::debug!(
        "a worker for a {:?} session of {} on console {}",
        order.kind,
        order.user,
        order.console
    );
    if !gevrek_dm::wire::is_a_user_name(&order.user) {
        say(&Answer::no("There is no such user."));
        std::process::exit(1);
    }
    let Some(account) = users::get_user_by_name(&order.user) else {
        log::debug!("there is no account {}", order.user);
        // (Said as a wrong password is: who has an account is not told.)
        say(&Answer::no("The name or the password is not right."));
        std::process::exit(1);
    };
    // Root does not log into a desktop.
    if order.kind == Kind::User && account.uid() == 0 {
        log::debug!(
            "{} is not let in: root does not log into a desktop",
            order.user
        );
        say(&Answer::no("The name or the password is not right."));
        std::process::exit(1);
    }
    let environment = session_environment(&order, account.uid());
    // logind reads the session's seat, console and kind from PAM's
    // environment, or from that of the process that opens the session.
    for (name, value) in &environment {
        std::env::set_var(name, value);
    }
    let mut client =
        crate::pam_session::Session::start(service_of(order.kind), &order.user, &order.password)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    log::debug!(
        "PAM checks {} through {}",
        order.user,
        service_of(order.kind)
    );
    if let Err(error) = client.authenticate() {
        log::info!("{} was not let in: {error:?}", order.user);
        say(&Answer::no("The name or the password is not right."));
        std::process::exit(1);
    }
    log::debug!("PAM let {} in: the session is opened", order.user);
    if let Err(error) = client.open() {
        log::error!("no session for {}: {error:?}", order.user);
        say(&Answer::no("The session could not be opened."));
        std::process::exit(1);
    }
    log::debug!("PAM opened the session of {}", order.user);
    // The password has done its work: none of its copies is kept.
    client.forget_password();
    for text in [&mut line, &mut order.password] {
        // SAFETY: zero bytes are valid UTF-8, and the length is unchanged.
        unsafe { text.as_bytes_mut() }.fill(0);
        text.clear();
    }
    // What PAM's modules want the session to have: logind's own word on
    // the session, `pam_env`'s variables, a keyring's socket.
    let from_pam = client.environment();
    log::debug!(
        "PAM's modules gave the session {} variables",
        from_pam.len()
    );
    let environment = crate::pam_session::merged(&environment, &from_pam);
    // The session's console is taken before anybody is told that the
    // session is there (the service shows the console when it hears yes).
    let held = match crate::console::Held::take(order.console) {
        Ok(held) => {
            log::debug!("console {} is taken for the session", order.console);
            Some(held)
        }
        Err(error) => {
            log::warn!("console {} was not taken: {error}", order.console);
            None
        }
    };
    say(&Answer::yes());
    // A session starts when the service says that its console is the
    // one shown (gevrek is given the display only there), the greeter's
    // as a person's; a service that is gone says nothing, and nothing is
    // started.
    {
        let mut go = String::new();
        log::debug!("the session waits for its console to be shown");
        if std::io::stdin().lock().read_line(&mut go).unwrap_or(0) == 0 {
            log::debug!("the service went before it said to begin: the session is not started");
            drop(held);
            drop(client);
            std::process::exit(1);
        }
    }

    // The login remembers this person's wallpaper, accent and keyboard
    // for the next time it is shown: as they are now, and again as they
    // are when the session ends (what was changed while it ran is on the
    // login that comes after it, with no login in between).
    let remember = || {
        if order.kind != Kind::User {
            return;
        }
        if let Some(greeter) = users::get_user_by_name(gevrek_dm::GREETER_USER) {
            crate::remember::keep(
                &crate::remember::Reader {
                    name: &order.user,
                    uid: account.uid(),
                    gid: account.primary_group_id(),
                },
                account.home_dir(),
                greeter.home_dir(),
                (greeter.uid(), greeter.primary_group_id()),
            );
        }
    };
    remember();

    // The session's program, as the user: in the user's home, by the
    // user's shell (as a login shell, so that the profile is read), with
    // an environment made here and not inherited from the service.
    let home = account.home_dir().to_path_buf();
    let shell = account.shell().to_path_buf();
    let (uid, gid) = (account.uid(), account.primary_group_id());
    let name = std::ffi::CString::new(order.user.clone())?;
    let mut command = match order.kind {
        // (The greeter's account has no shell to log in with.)
        Kind::Greeter => {
            let mut command = std::process::Command::new("/bin/sh");
            command.arg("-c").arg(format!("exec {}", order.command));
            command
        }
        Kind::User => {
            let mut command = std::process::Command::new(&shell);
            command.arg("-l").arg("-c").arg(&order.command);
            command
        }
    };
    // What the session writes goes where the service's own words go (the
    // journal), not into the pipe the verdict went through.
    match std::io::stderr().as_fd().try_clone_to_owned() {
        Ok(journal) => {
            command.stdout(std::process::Stdio::from(journal));
        }
        Err(error) => log::debug!("what the session writes goes nowhere: {error}"),
    }
    command
        .env_clear()
        .env("HOME", &home)
        .env("USER", &order.user)
        .env("LOGNAME", &order.user)
        .env("SHELL", &shell)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .envs(environment.iter().cloned())
        .current_dir(if home.is_dir() {
            home.as_path()
        } else {
            std::path::Path::new("/")
        })
        .stdin(std::process::Stdio::null());
    // The session's number, as logind gave it (this process was put into
    // the session's group when the session was opened).
    match session_of(&std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default()) {
        Some(id) => {
            log::debug!("logind's session {id}");
            command.env("XDG_SESSION_ID", id);
        }
        None => log::debug!("the worker is in no session of logind's"),
    }
    // SAFETY: only calls that are safe between fork and exec: the groups
    // of the user, then the user's own identity, in that order (after the
    // identity is dropped the groups could not be set any more).
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                // (Already a leader of a session: nothing to do.)
            }
            if libc::initgroups(name.as_ptr(), gid) != 0
                || libc::setgid(gid) != 0
                || libc::setuid(uid) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            log::error!("the session of {} did not start: {error}", order.user);
            drop(held);
            drop(client);
            std::process::exit(1);
        }
    };
    // When the service ends, or asks this worker to end, the session's
    // programs are asked to end; the worker stays to close the session.
    log::debug!(
        "the session of {} is started (process {})",
        order.user,
        child.id()
    );
    SESSION.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
    // SAFETY: the handler only sends a signal on, which may be done there.
    unsafe {
        libc::signal(libc::SIGTERM, pass_on as *const () as libc::sighandler_t);
        if order.kind == Kind::Greeter {
            libc::signal(
                libc::SIGUSR1,
                pass_on_leave as *const () as libc::sighandler_t,
            );
        }
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
    let status = child.wait()?;
    log::debug!("the session of {} ended: {status}", order.user);
    remember();
    drop(held);
    // The session is closed with the client (PAM tells logind).
    drop(client);
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_told_its_seat_its_console_and_its_kind() {
        let order = Order {
            kind: Kind::User,
            user: "ada".to_string(),
            password: String::new(),
            console: 2,
            command: "gevrek-session".to_string(),
            environment: vec![("GEVREK_X".to_string(), "1".to_string())],
        };
        let environment = session_environment(&order, 1000);
        let of = |name: &str| {
            environment
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(of("XDG_VTNR"), Some("2"));
        assert_eq!(of("XDG_SEAT"), Some("seat0"));
        assert_eq!(of("XDG_SESSION_CLASS"), Some("user"));
        assert_eq!(of("XDG_SESSION_TYPE"), Some("wayland"));
        assert_eq!(of("XDG_RUNTIME_DIR"), Some("/run/user/1000"));
        assert_eq!(of("GEVREK_X"), Some("1"));
        let greeter = Order {
            kind: Kind::Greeter,
            console: 1,
            ..order
        };
        let environment = session_environment(&greeter, 970);
        assert!(environment.contains(&("XDG_SESSION_CLASS".to_string(), "greeter".to_string())));
        assert_eq!(service_of(Kind::Greeter), "gevrek-greeter");
        assert_eq!(service_of(Kind::User), "gevrek-dm");
        assert_eq!(
            session_of("0::/user.slice/user-1000.slice/session-12.scope\n").as_deref(),
            Some("12")
        );
        assert_eq!(session_of("0::/system.slice/gevrek-dm.service\n"), None);
    }
}
