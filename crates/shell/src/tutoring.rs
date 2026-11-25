//! The tutorial as the shell carries it: what needs a program or the
//! line to the window manager. The practice windows are opened and
//! closed here, the tutorial's workspace is tidied away at its end (and
//! after a gevrek that stopped in the middle), and the welcome is
//! opened. Everything else of a tutorial that is taken (its scene, its
//! steps, the ring's place) is `gevrek_tutorial::tutoring`, the same on
//! every system; each system's half opens the card's surface and hands
//! the commands on.

pub use gevrek_tutorial::tutoring::*;
use gevrek_tutorial::{Facts, Progress};
use std::time::Duration;

/// The names the practice windows are opened by, in turn.
const PRACTICE: [&str; 6] = ["Notes", "Browser", "Music", "Mail", "Files", "Photos"];

/// Open this many practice windows more: the settings program beside
/// this one is a practice window when it is asked to be.
pub fn open_practice(windows: &mut Vec<std::process::Child>, count: usize) {
    let program = std::env::current_exe().ok().and_then(|own| {
        Some(
            own.parent()?
                .join(format!("gevrek-studio{}", std::env::consts::EXE_SUFFIX)),
        )
    });
    let Some(program) = program.filter(|program| program.is_file()) else {
        log::warn!("no practice window: gevrek-studio is not beside the shell");
        return;
    };
    for _ in 0..count {
        let name = PRACTICE[windows.len() % PRACTICE.len()];
        // Held by a pipe: a practice window goes when whoever opened it
        // is gone, however that came (`HELD`).
        match std::process::Command::new(&program)
            .args(["--practice", name])
            .env(gevrek_tutorial::HELD, "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(window) => windows.push(window),
            Err(error) => log::warn!("a practice window was not opened: {error}"),
        }
    }
}

/// The tutorial has ended: its practice windows are closed, its
/// workspace goes once they are gone (a moment), and what was shown
/// before (`before`: a workspace's name) is shown again. `own`: the
/// person's own windows on the tutorial's workspace, which go there too.
/// `beside`: the tutorial's second workspace is there, and goes as well.
pub fn tidy(
    before: String,
    windows: Vec<std::process::Child>,
    own: Vec<u64>,
    beside: bool,
    tell: impl Fn(gevrek_link::Request) + Send + 'static,
) {
    for mut window in windows {
        let _ = window.kill();
        let _ = window.wait();
    }
    std::thread::spawn(move || {
        let dispatch =
            |command: serde_json::Value| tell(gevrek_link::Request::Dispatch(command.to_string()));
        std::thread::sleep(Duration::from_millis(600));
        for command in tidy_commands(&before, &own, beside) {
            dispatch(command);
        }
    });
}

/// A tutorial that did not end by itself (the shell was stopped, the
/// session ended) has left its workspace behind: it is closed, once,
/// when the window manager first says how the desktop stands. Its
/// practice windows went with the shell that held them.
pub fn tidy_after_a_stop(
    wm: &serde_json::Value,
    asked: &mut bool,
    tell: impl Fn(gevrek_link::Request) + Send + 'static,
) {
    if std::mem::replace(asked, true) {
        return;
    }
    let file = gevrek_tutorial::progress::file();
    let mut progress = Progress::load(&file);
    if !progress.running {
        return;
    }
    progress.running = false;
    progress.save(&file);
    if let Some(before) = left_behind(wm) {
        log::info!("a tutorial that was stopped is tidied up after");
        // (Its practice windows went with the shell that held them: what
        // is on its workspace now, where that is the one shown, is the
        // person's.)
        let facts = Facts::of(wm);
        let beside = facts
            .workspaces
            .iter()
            .any(|name| name == gevrek_tutorial::WORKSPACE_BESIDE);
        tidy(before, Vec::new(), own_windows(&facts), beside, tell);
    }
}

/// Open the welcome where a person is to be welcomed (a first launch).
/// Asked once for each run of the shell, with the configuration that
/// runs.
pub fn welcome_if_due(
    running: &gevrek_model::Config,
    asked: &mut bool,
    tell: impl Fn(gevrek_link::Request) + Send + 'static,
) {
    if !std::mem::replace(asked, true) && gevrek_tutorial::welcome_due(running) {
        log::info!("the first launch: the welcome is opened");
        open_welcome(tell);
    }
}

/// Open the welcome: the settings program beside this one shows it. When
/// it ends with the tour asked for, the tour is begun: after the
/// welcome's program is gone, and its window with it.
pub fn open_welcome(tell: impl Fn(gevrek_link::Request) + Send + 'static) {
    let program = std::env::current_exe().ok().and_then(|own| {
        Some(
            own.parent()?
                .join(format!("gevrek-studio{}", std::env::consts::EXE_SUFFIX)),
        )
    });
    let Some(program) = program.filter(|program| program.is_file()) else {
        log::warn!("no welcome: gevrek-studio is not beside the shell");
        return;
    };
    let welcome = std::process::Command::new(&program)
        .arg("--welcome")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let mut welcome = match welcome {
        Ok(welcome) => welcome,
        Err(error) => {
            log::warn!("the welcome was not opened: {error}");
            return;
        }
    };
    std::thread::spawn(move || {
        let asked = welcome
            .wait()
            .is_ok_and(|ended| ended.code() == Some(gevrek_tutorial::TOUR_ASKED));
        if asked {
            // (A moment: the desktop takes in that the window is gone.)
            std::thread::sleep(Duration::from_millis(600));
            tell(gevrek_link::Request::Dispatch(
                serde_json::json!({"command": "shell", "action": "tutorial"}).to_string(),
            ));
        }
    });
}
