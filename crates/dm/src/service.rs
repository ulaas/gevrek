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

/// Start a program from the one thread that starts them all and lives
/// as long as the service does. A worker asks to be ended when its
/// parent goes (`PR_SET_PDEATHSIG`), and its parent, to the system, is
/// the thread that started it: started from the thread of one
/// connection, which ends when it has answered, a worker could be told
/// to end, and its session with it, a moment after the login.
fn started_by_the_keeper(command: Command) -> std::io::Result<Child> {
    type Asked = (Command, std::sync::mpsc::Sender<std::io::Result<Child>>);
    static KEEPER: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Asked>>> =
        std::sync::OnceLock::new();
    let keeper = KEEPER.get_or_init(|| {
        let (ask, asked) = std::sync::mpsc::channel::<Asked>();
        let started = std::thread::Builder::new()
            .name("gevrek-dm-keeper".to_string())
            .spawn(move || {
                for (mut command, answer) in asked {
                    let _ = answer.send(command.spawn());
                }
            });
        if let Err(error) = started {
            log::error!("the thread that starts the workers was not started: {error}");
        }
        std::sync::Mutex::new(ask)
    });
    let (answer, answered) = std::sync::mpsc::channel();
    let gone = || std::io::Error::other("the thread that starts the workers is gone");
    keeper
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .send((command, answer))
        .map_err(|_| gone())?;
    answered.recv().map_err(|_| gone())?
}

/// Start a worker with its order; its first line says whether the
/// session was opened. (What is written to it afterwards is the word
/// that the session may start.)
fn start_worker(order: &Order) -> anyhow::Result<(Child, Answer, std::process::ChildStdin)> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = started_by_the_keeper(command)?;
    log::debug!(
        "a worker (process {}) for the {:?} session of {}",
        child.id(),
        order.kind,
        order.user
    );
    let mut to = child.stdin.take().expect("a pipe to the worker");
    writeln!(to, "{}", serde_json::to_string(order)?)?;
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("a pipe from the worker")).read_line(&mut line)?;
    let answer: Answer = serde_json::from_str(&line).unwrap_or_else(|error| {
        log::debug!("the worker's answer was not understood: {error}");
        Answer::no("The session could not be opened.")
    });
    log::debug!(
        "the worker answered: {}",
        if answer.ok { "yes" } else { "no" }
    );
    if !answer.ok {
        match child.wait() {
            Ok(status) => log::debug!("the worker that said no ended: {status}"),
            Err(error) => log::debug!("the worker that said no was not waited for: {error}"),
        }
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
    // The service's first greeter has its console shown: a service that
    // is started again while another console is the one shown (a
    // session's, a text login's) had a greeter that logind gave no
    // display to, and no login came up. (A greeter started again later
    // takes no console from a session that is running.)
    let mut first = true;
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
            Ok((mut worker, answer, mut to)) if answer.ok => {
                // (The worker holds the console by now: logind starts no
                // text login on it when it is shown.)
                if first {
                    match crate::console::show(GREETER_CONSOLE) {
                        Ok(()) => log::debug!("console {GREETER_CONSOLE} is shown for the greeter"),
                        Err(error) => {
                            log::warn!("console {GREETER_CONSOLE} was not shown: {error}")
                        }
                    }
                    first = false;
                }
                match writeln!(to, "go") {
                    Ok(()) => log::debug!("the greeter's worker is told to begin"),
                    Err(error) => log::warn!("the greeter's worker was not told to begin: {error}"),
                }
                log::info!("the greeter is started");
                GREETER.store(worker.id() as i32, Ordering::SeqCst);
                match worker.wait() {
                    Ok(status) => log::info!("the greeter ended: {status}"),
                    Err(error) => log::warn!("the greeter was not waited for: {error}"),
                }
                GREETER.store(0, Ordering::SeqCst);
            }
            Ok((_, answer, _)) => log::error!("the greeter was not started: {}", answer.why),
            Err(error) => log::error!("the greeter was not started: {error}"),
        }
        if started.elapsed() < TOO_SOON {
            failures += 1;
            log::debug!("the greeter did not stay: {failures} of {TRIES} tries");
            // (A boot splash that still has the display keeps the
            // greeter from it.)
            if failures < TRIES {
                crate::splash::let_go_again();
            }
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
            log::debug!("the login is asked to go under its veil");
            // SAFETY: a signal to the service's own child.
            unsafe {
                libc::kill(greeter, libc::SIGUSR1);
            }
            std::thread::sleep(FOR_THE_VEIL);
        } else {
            log::debug!("no greeter to go under its veil");
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
            match writeln!(go, "go") {
                Ok(()) => log::debug!("the session's worker is told to begin"),
                Err(error) => log::warn!("the session's worker was not told to begin: {error}"),
            }
        }
    }

    fn wait(&self, mut session: Self::Session, user: &str) {
        let status = session.0.wait();
        log::info!("the session of {user} is over ({status:?})");
    }

    fn power(&self, restart: bool) -> bool {
        let asked = if restart { "reboot" } else { "poweroff" };
        match Command::new("systemctl").arg(asked).status() {
            Ok(status) => {
                log::info!("systemctl {asked}: {status}");
                status.success()
            }
            Err(error) => {
                log::warn!("systemctl {asked} did not start: {error}");
                false
            }
        }
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
        log::debug!("no login for {user}: a session is running already");
        return Answer::no("A session is running already.");
    }
    log::debug!("the greeter asks for a session of {user}");
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
        log::debug!("{user} has no session: {}", answer.why);
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
        log::debug!("back to the greeter on console {GREETER_CONSOLE}");
        machine.show(GREETER_CONSOLE);
        machine.release(SESSION_CONSOLE);
        running.store(false, Ordering::SeqCst);
    });
    answer
}

fn power(machine: &impl Machine, restart: bool, running: &AtomicBool) -> Answer {
    if running.load(Ordering::SeqCst) {
        log::debug!("the machine is not switched off: a session is running");
        return Answer::no("A session is running.");
    }
    log::debug!(
        "the greeter asks to {}",
        if restart { "restart" } else { "switch off" }
    );
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
            log::debug!("the greeter has drawn its first picture");
            machine.shown();
            Answer::yes()
        }
        Err(error) => {
            // (Not what it said, nor the parser's words, which quote it:
            // it could hold a password.)
            log::debug!(
                "the greeter's line was not understood ({} bytes, {:?} at column {})",
                line.len(),
                error.classify(),
                error.column()
            );
            Answer::no("That was not understood.")
        }
    }
}

fn serve(stream: UnixStream, greeter_uid: u32, running: &Arc<AtomicBool>) {
    // The socket's rights let only the greeter open it; who did is
    // asked of the kernel all the same.
    if peer_uid(&stream) != Some(greeter_uid) {
        log::warn!("a connection that is not the greeter's is refused");
        return;
    }
    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(10))) {
        log::debug!("the greeter's connection has no time limit: {error}");
    }
    let reader = match stream.try_clone() {
        Ok(reader) => reader,
        Err(error) => {
            log::debug!("the greeter's connection was not read: {error}");
            return;
        }
    };
    let Some(line) = line_of(reader) else {
        log::debug!("the greeter's connection ended with nothing read");
        return;
    };
    let answer = answer_to(&Arc::new(TheMachine), &line, running);
    let mut stream = stream;
    if let Err(error) = writeln!(
        stream,
        "{}",
        serde_json::to_string(&answer).unwrap_or_default()
    ) {
        log::debug!("the greeter was not answered: {error}");
    }
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
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                log::debug!("a connection to the service was not taken: {error}");
                continue;
            }
        };
        let running = Arc::clone(&running);
        let uid = greeter.uid();
        std::thread::spawn(move || serve(stream, uid, &running));
    }
    Ok(())
}

/// (What spoils what is read from outside: the engine's, taken in as it is.)
#[cfg(test)]
#[path = "../../engine/src/spoil.rs"]
mod spoil;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Mutex;

    #[test]
    fn a_program_started_for_a_thread_that_ends_is_not_ended_with_it() {
        // Asked for from a thread that ends at once, as the thread of a
        // connection does: the program is the keeper's, asks to be ended
        // when its parent goes, and lives on. (Started by that thread
        // itself it was told to end as the thread did.)
        let started = std::thread::spawn(|| {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 0.4; exit 7"]);
            // SAFETY: only a call that is safe between fork and exec.
            unsafe {
                use std::os::unix::process::CommandExt;
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            started_by_the_keeper(command)
        })
        .join()
        .expect("the thread that asked");
        let mut child = started.expect("started");
        // It ran to its own end, with its own answer, and was not ended
        // by a signal.
        assert_eq!(child.wait().expect("waited for").code(), Some(7));
    }

    /// What a machine was asked to do.
    #[derive(Debug, Clone, PartialEq)]
    enum Did {
        Open(Order),
        Veil,
        Show(u32),
        Release(u32),
        Go,
        Over,
        Power(bool),
        Shown,
    }

    /// A machine that does nothing and writes down what it was asked.
    struct Written {
        did: Mutex<Vec<Did>>,
        /// Whether the worker lets a person in.
        lets_in: bool,
        /// A session is over when a word comes here, or nobody is left
        /// to say one.
        over: Mutex<Receiver<()>>,
    }

    impl Written {
        fn new(lets_in: bool) -> (Arc<Self>, Sender<()>) {
            let (end, over) = channel();
            let machine = Self {
                did: Mutex::new(Vec::new()),
                lets_in,
                over: Mutex::new(over),
            };
            (Arc::new(machine), end)
        }

        fn note(&self, did: Did) {
            self.did.lock().unwrap().push(did);
        }

        fn did(&self) -> Vec<Did> {
            self.did.lock().unwrap().clone()
        }
    }

    impl Machine for Written {
        type Session = ();

        fn open(&self, order: &Order) -> anyhow::Result<((), Answer)> {
            self.note(Did::Open(order.clone()));
            Ok(if self.lets_in {
                ((), Answer::yes())
            } else {
                ((), Answer::no("The name or the password is not right."))
            })
        }

        fn veil(&self) {
            self.note(Did::Veil);
        }

        fn show(&self, console: u32) {
            self.note(Did::Show(console));
        }

        fn release(&self, console: u32) {
            self.note(Did::Release(console));
        }

        fn go(&self, _: &mut ()) {
            self.note(Did::Go);
        }

        fn wait(&self, _: (), _: &str) {
            let _ = self.over.lock().unwrap().recv();
            self.note(Did::Over);
        }

        fn power(&self, restart: bool) -> bool {
            self.note(Did::Power(restart));
            true
        }

        fn shown(&self) {
            self.note(Did::Shown);
        }
    }

    /// Until no session runs (the session's end is another thread's).
    fn until_over(running: &AtomicBool) {
        for _ in 0..2000 {
            if !running.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the session never was over");
    }

    const LOGIN: &str = r#"{"ask": "login", "user": "ada", "password": "s3cret"}"#;

    #[test]
    fn a_login_is_a_persons_session_on_its_console_until_it_is_over() {
        let (machine, end) = Written::new(true);
        let running = Arc::new(AtomicBool::new(false));
        assert_eq!(answer_to(&machine, LOGIN, &running), Answer::yes());
        let order = Order {
            kind: Kind::User,
            user: "ada".to_string(),
            password: "s3cret".to_string(),
            console: SESSION_CONSOLE,
            command: session_command(),
            environment: Vec::new(),
        };
        let first = [
            Did::Open(order),
            Did::Veil,
            Did::Show(SESSION_CONSOLE),
            Did::Go,
        ];
        assert_eq!(machine.did(), first);

        // While it runs: no second session, and the machine stays on.
        assert!(!answer_to(&machine, LOGIN, &running).ok);
        assert!(!answer_to(&machine, r#"{"ask": "power", "restart": false}"#, &running).ok);
        assert_eq!(machine.did(), first);

        // It is over: the login's console again, and the session's let go.
        end.send(()).unwrap();
        until_over(&running);
        assert_eq!(
            machine.did()[first.len()..],
            [
                Did::Over,
                Did::Show(GREETER_CONSOLE),
                Did::Release(SESSION_CONSOLE)
            ]
        );
        assert!(answer_to(&machine, r#"{"ask": "power", "restart": true}"#, &running).ok);
        assert_eq!(machine.did().last(), Some(&Did::Power(true)));
    }

    #[test]
    fn who_is_not_let_in_has_no_session_and_may_try_again() {
        let (machine, _end) = Written::new(false);
        let running = Arc::new(AtomicBool::new(false));
        for _ in 0..2 {
            let answer = answer_to(&machine, LOGIN, &running);
            assert!(!answer.ok);
            assert!(!running.load(Ordering::SeqCst));
        }
        assert!(machine.did().iter().all(|did| matches!(did, Did::Open(_))));
        assert_eq!(machine.did().len(), 2);
    }

    #[test]
    fn a_line_longer_than_a_name_and_a_password_is_not_understood() {
        let (machine, _end) = Written::new(true);
        let running = Arc::new(AtomicBool::new(false));
        let long = format!(
            r#"{{"ask": "login", "user": "ada", "password": "{}"}}"#,
            "x".repeat(9000)
        );
        let line = line_of(long.as_bytes()).expect("what was read of it");
        assert!(line.len() <= 8192);
        assert!(!answer_to(&machine, &line, &running).ok);
        assert!(machine.did().is_empty());
        // (What is no text is no line.)
        assert_eq!(line_of(&[0xff, 0xfe, b'\n'][..]), None);
        assert_eq!(line_of(&b""[..]).as_deref(), Some(""));
    }

    /// The service runs as root and reads what the login's program
    /// writes: whatever is written, the service comes to its end with an
    /// answer, opens nothing but a person's session on the session's
    /// console, one at a time, and does not switch off under a session.
    #[test]
    fn no_asking_from_outside_ends_the_service_or_takes_it_further() {
        use super::spoil::{harder, lived, places, spoil, Chance};
        let whole = serde_json::json!([
            {"ask": "login", "user": "ada", "password": "s3cret"},
            {"ask": "power", "restart": false},
            {"ask": "shown"},
            {"ask": "login", "user": "root", "password": ""},
            {"ask": "power", "restart": true},
        ]);
        let mut all = Vec::new();
        places(&whole, &mut Vec::new(), &mut all);

        let (times, seed) = harder();
        let mut chance = Chance(0x3c6e_f372_fe94_f82b ^ seed.rotate_left(5) | 1);
        let mut wrong: Vec<String> = Vec::new();
        for round in 0..600 * times {
            let mut document = whole.clone();
            let mut done = Vec::new();
            for _ in 0..chance.below(4) {
                done.push(spoil(&mut document, &all, &mut chance));
            }
            let mut lines: Vec<String> = match &document {
                serde_json::Value::Array(asks) => asks.iter().map(|ask| ask.to_string()).collect(),
                other => vec![other.to_string()],
            };
            // (Sometimes cut short.)
            if chance.below(6) == 0 {
                if let Some(last) = lines.last_mut() {
                    let mut end = chance.below(last.len() + 1);
                    while !last.is_char_boundary(end) {
                        end -= 1;
                    }
                    last.truncate(end);
                }
            }
            // A session that ends at once, one that ends at the end, and
            // a worker that lets nobody in.
            let (machine, end) = Written::new(round % 3 != 2);
            let stays = round % 3 == 0;
            let mut end = Some(end);
            if !stays {
                end = None;
            }
            let running = Arc::new(AtomicBool::new(false));
            let (told, flag) = (Arc::clone(&machine), Arc::clone(&running));
            let run = lived(move || {
                for line in &lines {
                    let Some(line) = line_of(line.as_bytes()) else {
                        continue;
                    };
                    let answer = answer_to(&told, &line, &flag);
                    assert!(!serde_json::to_string(&answer).unwrap().contains('\n'));
                }
            });
            drop(end);
            until_over(&running);
            let mut open = false;
            let mut beyond = None;
            for did in machine.did() {
                match &did {
                    Did::Open(order) => {
                        let as_it_must = order.kind == Kind::User
                            && order.console == SESSION_CONSOLE
                            && order.command == session_command()
                            && order.environment.is_empty();
                        if open || !as_it_must {
                            beyond = Some(format!("{did:?}"));
                        }
                        open = machine.lets_in;
                    }
                    Did::Over => open = false,
                    Did::Power(_) if open => beyond = Some("off under a session".to_string()),
                    Did::Show(console) | Did::Release(console)
                        if ![GREETER_CONSOLE, SESSION_CONSOLE].contains(console) =>
                    {
                        beyond = Some(format!("{did:?}"));
                    }
                    _ => {}
                }
            }
            if let Some(why) = run.err().or(beyond) {
                wrong.push(format!("with {}: {why}", done.join(", ")));
            }
        }
        wrong.truncate(20);
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

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
