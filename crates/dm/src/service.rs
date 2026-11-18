//! The service: keeps the greeter running on the first console, listens to
//! it on a socket only it may open, and for a login it accepts opens the
//! user's session on the second console and shows that; when the session
//! is over the first console is shown again.

use gevrek_dm::wire::{Answer, Ask, Kind, Order};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gevrek_dm::GREETER_USER;
/// The console the greeter is on, and the one a person's session is on.
const GREETER_CONSOLE: u32 = 1;
const SESSION_CONSOLE: u32 = 2;
/// Where the service's own things are while it runs.
const RUN_FOLDER: &str = "/run/gevrek-dm";
/// A greeter that ends sooner than this did not come up.
const TOO_SOON: Duration = Duration::from_secs(20);
/// After so many greeters in a row that did not come up, the service
/// gives the console back to a text login and ends.
const TRIES: u32 = 3;

/// Where gevrek's programs are: beside this one.
fn beside(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|file| file.parent().map(|folder| folder.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// The keyboard layout the system is set to (`localectl`, which writes
/// `/etc/vconsole.conf`): the greeter has nobody's configuration, and a
/// password typed on another layout is another password.
pub fn keyboard_of(vconsole: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in vconsole.lines() {
        let Some((name, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        let variable = match name.trim() {
            "XKBLAYOUT" => "XKB_DEFAULT_LAYOUT",
            "XKBVARIANT" => "XKB_DEFAULT_VARIANT",
            "XKBMODEL" => "XKB_DEFAULT_MODEL",
            "XKBOPTIONS" => "XKB_DEFAULT_OPTIONS",
            _ => continue,
        };
        if !value.is_empty() {
            found.push((variable.to_string(), value.to_string()));
        }
    }
    found
}

/// The layout for a console keymap (`KEYMAP=trq`), where the system
/// names only that: by systemd's own table of which goes with which
/// (`kbd-model-map`: keymap, layout, model, variant, options; `-` for
/// none).
pub fn keyboard_of_keymap(vconsole: &str, table: &str) -> Vec<(String, String)> {
    let Some(keymap) = vconsole.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == "KEYMAP")
            .then(|| {
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string()
            })
            .filter(|value| !value.is_empty())
    }) else {
        return Vec::new();
    };
    let Some(row) = table
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .find(|row| row.first() == Some(&keymap.as_str()) && row.len() >= 2)
    else {
        return Vec::new();
    };
    [
        ("XKB_DEFAULT_LAYOUT", 1),
        ("XKB_DEFAULT_MODEL", 2),
        ("XKB_DEFAULT_VARIANT", 3),
        ("XKB_DEFAULT_OPTIONS", 4),
    ]
    .into_iter()
    .filter_map(|(variable, column)| {
        let value = row.get(column).copied().filter(|value| *value != "-")?;
        Some((variable.to_string(), value.to_string()))
    })
    .collect()
}

/// What the greeter's session runs: gevrek's compositor on the hardware,
/// with no configuration of anybody's, and the login in it.
fn greeter_command() -> String {
    format!(
        "{} --backend drm --safe --no-xwayland --ends-with-program -- {} --greeter",
        beside("gevrek-wl").display(),
        beside("gevrek-lock").display()
    )
}

/// What a person's session runs: the session installed for them, else
/// the system's.
fn session_command() -> String {
    // Handed to the person's own shell, which need not be one that
    // reads `if ...; then`: fish and csh do not, and such a person's
    // session ended as it began. Every shell can start `/bin/sh` with
    // one word in single quotes, and that one reads the rest.
    format!(
        "exec /bin/sh -c 'if [ -x \"$HOME/.local/bin/gevrek-session\" ]; then exec \"$HOME/.local/bin/gevrek-session\"; else exec \"{}\"; fi'",
        beside("gevrek-session").display()
    )
}

/// Start a worker with its order; its first line says whether the
/// session was opened. (What is written to it afterwards is the word
/// that a person's session may start.)
fn start_worker(order: &Order) -> anyhow::Result<(Child, Answer, std::process::ChildStdin)> {
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut to = child.stdin.take().expect("a pipe to the worker");
    writeln!(to, "{}", serde_json::to_string(order)?)?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("a pipe from the worker")).read_line(&mut line)?;
    let answer = serde_json::from_str(&line)
        .unwrap_or_else(|_| Answer::no("The session could not be opened."));
    if !answer.ok {
        let _ = child.wait();
    }
    Ok((child, answer, to))
}

/// The greeter's worker, while there is one (to ask the login's gevrek
/// to go under its veil).
static GREETER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// How long the login's gevrek is given to go under its veil before the
/// desktop's console is shown (its change takes about two thirds of a second).
const FOR_THE_VEIL: Duration = Duration::from_millis(800);

/// Keep the greeter there: started again when it ends, and given up
/// after a few that did not come up.
fn keep_greeter(socket: PathBuf) {
    let mut failures = 0;
    loop {
        let mut environment = vec![
            (
                gevrek_dm::SOCKET_VARIABLE.to_string(),
                socket.display().to_string(),
            ),
            ("GEVREK_INSTANCE".to_string(), "greeter".to_string()),
        ];
        let vconsole = std::fs::read_to_string("/etc/vconsole.conf").unwrap_or_default();
        let mut keyboard = keyboard_of(&vconsole);
        if !keyboard
            .iter()
            .any(|(name, _)| name == "XKB_DEFAULT_LAYOUT")
        {
            // (Only the console's keymap is named: the layout that goes with it.)
            keyboard = keyboard_of_keymap(
                &vconsole,
                &std::fs::read_to_string("/usr/share/systemd/kbd-model-map").unwrap_or_default(),
            );
        }
        environment.extend(keyboard);
        // What the person who logged in last has: their keyboard in
        // place of the system's, their wallpaper and accent on the login.
        if let Some(greeter) = users::get_user_by_name(GREETER_USER) {
            use users::os::unix::UserExt;
            let kept = crate::remember::environment(greeter.home_dir());
            environment.retain(|(name, _)| !kept.iter().any(|(kept, _)| kept == name));
            if kept.iter().any(|(name, _)| name == "XKB_DEFAULT_LAYOUT") {
                // (A layout of theirs with the system's variant would be no layout.)
                environment.retain(|(name, _)| !name.starts_with("XKB_DEFAULT_"));
            }
            environment.extend(kept);
        }
        let order = Order {
            kind: Kind::Greeter,
            user: GREETER_USER.to_string(),
            password: String::new(),
            console: GREETER_CONSOLE,
            command: greeter_command(),
            environment,
        };
        let started = Instant::now();
        match start_worker(&order) {
            Ok((mut worker, answer, _to)) if answer.ok => {
                log::info!("the greeter is started");
                GREETER.store(worker.id() as i32, Ordering::SeqCst);
                let _ = worker.wait();
                GREETER.store(0, Ordering::SeqCst);
            }
            Ok((_, answer, _)) => log::error!("the greeter was not started: {}", answer.why),
            Err(error) => log::error!("the greeter was not started: {error}"),
        }
        if started.elapsed() < TOO_SOON {
            failures += 1;
        } else {
            failures = 0;
        }
        if failures >= TRIES {
            // No login is coming: a boot splash goes, and the text
            // console is there again.
            crate::splash::end(false);
            // (The unit starts the text login when this service fails.)
            log::error!(
                "the greeter did not come up {TRIES} times: the console goes back to a text login"
            );
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Who is at the other end of a connection.
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: u32::MAX,
        gid: u32::MAX,
    };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the kernel fills the record, whose size it is told.
    let done = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    (done == 0).then_some(credentials.uid)
}

/// The machine as the service touches it for what the greeter asks:
/// every step that changes something is one of these. The service's own
/// is `TheMachine`; a check has one that only writes down what was done.
trait Machine: Send + Sync + 'static {
    /// A session that was opened, until it is over.
    type Session: Send + 'static;
    /// Have a worker check the password and open the session.
    fn open(&self, order: &Order) -> anyhow::Result<(Self::Session, Answer)>;
    /// The login goes under its veil, and is given the time for it.
    fn veil(&self);
    /// Show a console.
    fn show(&self, console: u32);
    /// Let go of a console.
    fn release(&self, console: u32);
    /// The session's console is shown: the session may start.
    fn go(&self, session: &mut Self::Session);
    /// Wait until the session is over.
    fn wait(&self, session: Self::Session, user: &str);
    /// Switch the machine off, or start it again; whether it took that.
    fn power(&self, restart: bool) -> bool;
    /// The login is on the display: the boot splash under it ends.
    fn shown(&self);
}

/// The machine itself.
struct TheMachine;

impl Machine for TheMachine {
    type Session = (Child, Option<std::process::ChildStdin>);

    fn open(&self, order: &Order) -> anyhow::Result<(Self::Session, Answer)> {
        let (worker, answer, go) = start_worker(order)?;
        Ok(((worker, Some(go)), answer))
    }

    fn veil(&self) {
        let greeter = GREETER.load(Ordering::SeqCst);
        if greeter > 0 {
            // SAFETY: a signal to the service's own child.
            unsafe {
                libc::kill(greeter, libc::SIGUSR1);
            }
            std::thread::sleep(FOR_THE_VEIL);
        }
    }

    fn show(&self, console: u32) {
        if let Err(error) = crate::console::show(console) {
            log::error!("console {console} was not shown: {error}");
        }
    }

    fn release(&self, console: u32) {
        crate::console::release(console);
    }

    fn go(&self, session: &mut Self::Session) {
        if let Some(mut go) = session.1.take() {
            let _ = writeln!(go, "go");
        }
    }

    fn wait(&self, mut session: Self::Session, user: &str) {
        let status = session.0.wait();
        log::info!("the session of {user} is over ({status:?})");
    }

    fn power(&self, restart: bool) -> bool {
        Command::new("systemctl")
            .arg(if restart { "reboot" } else { "poweroff" })
            .status()
            .is_ok_and(|status| status.success())
    }

    fn shown(&self) {
        crate::splash::end(true);
    }
}

/// Log a person in: the worker checks the password and opens the
/// session; then the session's console is shown, and when the session is
/// over the greeter's again.
fn login<M: Machine>(
    machine: &Arc<M>,
    user: String,
    password: String,
    running: &Arc<AtomicBool>,
) -> Answer {
    if running.swap(true, Ordering::SeqCst) {
        return Answer::no("A session is running already.");
    }
    let order = Order {
        kind: Kind::User,
        user: user.clone(),
        password,
        console: SESSION_CONSOLE,
        command: session_command(),
        environment: Vec::new(),
    };
    let (mut session, answer) = match machine.open(&order) {
        Ok(started) => started,
        Err(error) => {
            log::error!("no worker for the session: {error}");
            running.store(false, Ordering::SeqCst);
            return Answer::no("The session could not be started.");
        }
    };
    if !answer.ok {
        running.store(false, Ordering::SeqCst);
        return answer;
    }
    log::info!("a session of {user} on console {SESSION_CONSOLE}");
    // (The worker holds the session's console, so that what the greeter
    // showed last stays until the desktop's first frame.)
    // The login goes under its veil, and the desktop comes out from
    // under its own: the change of consoles is not seen as a cut.
    machine.veil();
    machine.show(SESSION_CONSOLE);
    // Its console is shown: the session may start.
    machine.go(&mut session);
    let running = Arc::clone(running);
    let machine = Arc::clone(machine);
    std::thread::spawn(move || {
        machine.wait(session, &user);
        machine.show(GREETER_CONSOLE);
        machine.release(SESSION_CONSOLE);
        running.store(false, Ordering::SeqCst);
    });
    answer
}

fn power(machine: &impl Machine, restart: bool, running: &AtomicBool) -> Answer {
    if running.load(Ordering::SeqCst) {
        return Answer::no("A session is running.");
    }
    if machine.power(restart) {
        Answer::yes()
    } else {
        Answer::no("The system did not take that.")
    }
}

/// The line the greeter wrote. (A line longer than any name and password
/// is not read to its end.)
fn line_of(reader: impl std::io::Read) -> Option<String> {
    let mut line = String::new();
    BufReader::new(std::io::Read::take(reader, 8192))
        .read_line(&mut line)
        .ok()?;
    Some(line)
}

/// What is answered to a line of the greeter's, after what it asks is done.
fn answer_to<M: Machine>(machine: &Arc<M>, line: &str, running: &Arc<AtomicBool>) -> Answer {
    match serde_json::from_str::<Ask>(line) {
        Ok(Ask::Login { user, password }) => login(machine, user, password, running),
        Ok(Ask::Power { restart }) => power(&**machine, restart, running),
        Ok(Ask::Shown) => {
            machine.shown();
            Answer::yes()
        }
        Err(_) => Answer::no("That was not understood."),
    }
}

fn serve(stream: UnixStream, greeter_uid: u32, running: &Arc<AtomicBool>) {
    // The socket's rights let only the greeter open it; who did is
    // asked of the kernel all the same.
    if peer_uid(&stream) != Some(greeter_uid) {
        log::warn!("a connection that is not the greeter's is refused");
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let Some(line) = line_of(reader) else {
        return;
    };
    let answer = answer_to(&Arc::new(TheMachine), &line, running);
    let mut stream = stream;
    let _ = writeln!(
        stream,
        "{}",
        serde_json::to_string(&answer).unwrap_or_default()
    );
}

pub fn run() -> anyhow::Result<()> {
    // SAFETY: asks who runs this.
    anyhow::ensure!(
        unsafe { libc::geteuid() } == 0,
        "gevrek-dm is a service of the system: it is started as root, by its unit"
    );
    let greeter = users::get_user_by_name(GREETER_USER).ok_or_else(|| {
        anyhow::anyhow!("there is no user {GREETER_USER} (the package makes it: systemd-sysusers)")
    })?;
    let folder = Path::new(RUN_FOLDER);
    std::fs::create_dir_all(folder)?;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755))?;
    let socket = folder.join("greeter.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    std::os::unix::fs::chown(
        &socket,
        Some(greeter.uid()),
        Some(greeter.primary_group_id()),
    )?;

    // A boot splash lets go of the display before the greeter takes it.
    crate::splash::let_go();

    let running = Arc::new(AtomicBool::new(false));
    let greeter_socket = socket.clone();
    std::thread::Builder::new()
        .name("gevrek-dm-greeter".to_string())
        .spawn(move || keep_greeter(greeter_socket))?;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let running = Arc::clone(&running);
        let uid = greeter.uid();
        std::thread::spawn(move || serve(stream, uid, &running));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_greeter_types_on_the_systems_keyboard_layout() {
        let found = keyboard_of("KEYMAP=\"trq\"\nFONT=eurlatgr\nXKBLAYOUT=\"tr,us\"\nXKBVARIANT=\",\"\nXKBOPTIONS=grp:alt_shift_toggle\nXKBMODEL=\n");
        assert_eq!(
            found,
            vec![
                ("XKB_DEFAULT_LAYOUT".to_string(), "tr,us".to_string()),
                ("XKB_DEFAULT_VARIANT".to_string(), ",".to_string()),
                (
                    "XKB_DEFAULT_OPTIONS".to_string(),
                    "grp:alt_shift_toggle".to_string()
                ),
            ]
        );
        assert!(keyboard_of("").is_empty());
    }

    #[test]
    fn a_console_keymap_names_its_layout_by_the_systems_table() {
        let table = "# Generated\nus\t\tus\tpc105+inet\t-\tterminate:ctrl_alt_bksp\ntrq\t\ttr\tpc105+inet\t-\tterminate:ctrl_alt_bksp\nde-latin1-nodeadkeys de pc105+inet nodeadkeys terminate:ctrl_alt_bksp\n";
        let found = keyboard_of_keymap("KEYMAP=\"trq\"\nFONT=eurlatgr\n", table);
        assert!(found.contains(&("XKB_DEFAULT_LAYOUT".to_string(), "tr".to_string())));
        assert!(!found.iter().any(|(name, _)| name == "XKB_DEFAULT_VARIANT"));
        let german = keyboard_of_keymap("KEYMAP=de-latin1-nodeadkeys\n", table);
        assert!(german.contains(&("XKB_DEFAULT_VARIANT".to_string(), "nodeadkeys".to_string())));
        assert!(keyboard_of_keymap("KEYMAP=no-such\n", table).is_empty());
        assert!(keyboard_of_keymap("FONT=x\n", table).is_empty());
    }

    #[test]
    fn the_greeter_is_gevrek_and_nothing_else() {
        let command = greeter_command();
        assert!(command
            .contains("gevrek-wl --backend drm --safe --no-xwayland --ends-with-program -- "));
        assert!(command.ends_with("gevrek-lock --greeter"));
        assert!(session_command().contains("gevrek-session"));
    }
}
