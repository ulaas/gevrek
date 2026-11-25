//! What the panel's later modules show, looked at beside the shell and
//! only while a panel shows it: how busy the machine is (`load`), what a
//! program of the person's own writes (`custom`), and when to ask what
//! is playing (`media`).
//!
//! One thread, started when the panels are known and ended when they are
//! others. It tells what it found when that is other than it was; a
//! panel that shows none of these starts nothing.

use gevrek_model::settings::{CustomModule, PanelModule, PanelProfile};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the machine's load and what is playing are looked at.
const EVERY: Duration = Duration::from_secs(2);

/// What a line of a program's is cut off at, in letters: a panel is no
/// place for more.
const LINE_MOST: usize = 120;

/// What the thread tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum News {
    /// How busy the machine is (`None`: nothing says).
    Load(Option<gevrek_system::status::Load>),
    /// It is time to ask what is playing.
    MediaDue,
    /// What the program of a panel's `custom` wrote: the panel's id, and
    /// its line (`None`: it wrote none, or did not start).
    Custom(String, Option<String>),
}

/// What is wanted of the thread, by the panels as they are.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Wanted {
    pub load: bool,
    pub media: bool,
    /// The panels whose `custom` has a program to ask: each one's id.
    pub custom: Vec<(String, CustomModule)>,
}

impl Wanted {
    /// What the panels that are switched on show of this.
    pub fn of(panels: &[PanelProfile]) -> Self {
        let mut wanted = Self::default();
        for panel in panels.iter().filter(|panel| panel.enabled) {
            wanted.load |= panel.shows(PanelModule::Load);
            wanted.media |= panel.shows(PanelModule::Media);
            if panel.shows(PanelModule::Custom) && !panel.custom.command.is_empty() {
                wanted.custom.push((panel.id.clone(), panel.custom.clone()));
            }
        }
        wanted
    }

    pub fn is_nothing(&self) -> bool {
        !self.load && !self.media && self.custom.is_empty()
    }
}

/// The first line a program writes, without what is around it, and not
/// longer than a panel has room for. `None`: no program, it did not
/// start, or it wrote nothing. (Started as it is given: no shell reads
/// the line.)
pub fn first_line(command: &[String]) -> Option<String> {
    let (program, arguments) = command.split_first()?;
    let output = std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|error| log::debug!("the panel's program {program:?} did not start: {error}"))
        .ok()?;
    line_of(&String::from_utf8_lossy(&output.stdout))
}

/// The first line of a text that has anything on it, as a panel shows it.
pub fn line_of(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    // (What cannot be drawn in a line is left out.)
    let clean: String = line
        .chars()
        .filter(|letter| !letter.is_control())
        .take(LINE_MOST)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// Start a program without waiting for it (a press on a panel's
/// `custom`). Whether it started.
pub fn start(command: &[String]) -> bool {
    let Some((program, arguments)) = command.split_first() else {
        return false;
    };
    match std::process::Command::new(program)
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            // (Waited for beside the shell, so that nothing of it stays
            // behind when it ends.)
            let _ = std::thread::Builder::new()
                .name("gevrek-panel-press".to_string())
                .spawn(move || {
                    let _ = child.wait();
                });
            true
        }
        Err(error) => {
            log::warn!("the panel's program {program:?} did not start: {error}");
            false
        }
    }
}

/// The thread, for as long as this lives.
pub struct Watch {
    stop: Arc<AtomicBool>,
    wanted: Wanted,
}

impl Watch {
    /// What it was started for.
    pub fn wanted(&self) -> &Wanted {
        &self.wanted
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// Look at what is wanted, beside the shell, and tell (on that thread)
/// what was found when it is other than it was. `None`: nothing is
/// wanted, or the thread did not start.
pub fn watch(wanted: Wanted, mut tell: impl FnMut(News) + Send + 'static) -> Option<Watch> {
    if wanted.is_nothing() {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (ended, asked) = (Arc::clone(&stop), wanted.clone());
    let started = std::thread::Builder::new()
        .name("gevrek-panel-extras".to_string())
        .spawn(move || {
            let mut worked = None;
            let mut load_said: Option<Option<gevrek_system::status::Load>> = None;
            // When each program is asked next, and what it said.
            let mut programs: Vec<(Option<Instant>, Option<Option<String>>)> =
                vec![(Some(Instant::now()), None); asked.custom.len()];
            while !ended.load(Ordering::Acquire) {
                if asked.load {
                    let load = gevrek_system::status::load(&mut worked);
                    if load_said != Some(load) {
                        load_said = Some(load);
                        tell(News::Load(load));
                    }
                }
                if asked.media {
                    tell(News::MediaDue);
                }
                let now = Instant::now();
                for ((id, custom), (due, said)) in asked.custom.iter().zip(programs.iter_mut()) {
                    if due.is_none_or(|due| due > now) {
                        continue;
                    }
                    let line = first_line(&custom.command);
                    if said.as_ref() != Some(&line) {
                        *said = Some(line.clone());
                        tell(News::Custom(id.clone(), line));
                    }
                    // (Asked once where it is not to be asked again.)
                    *due = custom.every().map(|every| Instant::now() + every);
                }
                // (In short steps: ended when the panels are others, the
                // thread is gone within a moment, not after its wait.)
                let until = Instant::now() + EVERY;
                while Instant::now() < until && !ended.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        });
    match started {
        Ok(_) => Some(Watch { stop, wanted }),
        Err(error) => {
            log::warn!("nothing looks at what the panel's modules show: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_the_first_that_says_something_and_no_longer_than_a_panel() {
        assert_eq!(
            line_of("\n  \n  21 °C, clear \nmore\n").as_deref(),
            Some("21 °C, clear")
        );
        assert_eq!(line_of(""), None);
        assert_eq!(line_of("\n\t\n"), None);
        assert_eq!(line_of("a\u{7}b").as_deref(), Some("ab"));
        let long = "x".repeat(500);
        assert_eq!(
            line_of(&long).map(|line| line.chars().count()),
            Some(LINE_MOST)
        );
        // No program: no line, and nothing started.
        assert_eq!(first_line(&[]), None);
        assert!(!start(&[]));
        assert_eq!(
            first_line(&["gevrek-no-such-program-there-is".to_string()]),
            None
        );
    }

    #[test]
    fn only_what_a_panel_that_is_on_shows_is_looked_at() {
        let mut panel = PanelProfile::default();
        assert!(Wanted::of(std::slice::from_ref(&panel)).is_nothing());
        assert!(watch(Wanted::default(), |_| {}).is_none());
        panel.right.push(PanelModule::Load);
        panel.right.push(PanelModule::Custom);
        // (Words alone need no program asked.)
        panel.custom.text = "hello".to_string();
        let wanted = Wanted::of(std::slice::from_ref(&panel));
        assert!(wanted.load && !wanted.media && wanted.custom.is_empty());
        panel.custom.command = vec!["date".to_string()];
        panel.left.push(PanelModule::Media);
        let wanted = Wanted::of(std::slice::from_ref(&panel));
        assert!(wanted.media);
        assert_eq!(wanted.custom.len(), 1);
        assert_eq!(wanted.custom[0].0, "main");
        // Switched off: nothing of it.
        panel.enabled = false;
        assert!(Wanted::of(&[panel]).is_nothing());
    }

    #[test]
    fn the_load_is_told_when_it_is_asked_for_and_the_thread_ends_with_its_watch() {
        let (told, news) = std::sync::mpsc::channel();
        let watch = watch(
            Wanted {
                load: true,
                ..Wanted::default()
            },
            move |found| {
                let _ = told.send(found);
            },
        )
        .expect("started");
        let first = news
            .recv_timeout(Duration::from_secs(5))
            .expect("told once at the start");
        assert!(matches!(first, News::Load(_)));
        drop(watch);
        // (The other end goes with the thread: nothing more comes for good.)
        let ended = Instant::now() + Duration::from_secs(5);
        loop {
            match news.recv_timeout(Duration::from_millis(100)) {
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                _ if Instant::now() > ended => panic!("the thread did not end"),
                _ => {}
            }
        }
    }
}
