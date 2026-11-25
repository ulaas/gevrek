//! The helper for windows of programs that run as an administrator.
//!
//! Windows lets a program change the windows of programs with no more
//! rights than its own. A window of a program that runs elevated can be
//! seen by a gevrek that does not (where it is, what it is called, that it
//! opened, moved or closed), and cannot be moved, hidden, given the
//! keyboard or closed by it. Such windows were left alone altogether
//! (`elevated.rs`).
//!
//! With the helper they are gevrek's windows as any other. The helper
//! (`gevrek-elevated`) is a small program that runs elevated and does, for
//! the window manager, the few things the window manager may not do
//! itself:
//!
//! ```text
//!  gevrek-wm (not elevated)                 gevrek-elevated (elevated)
//!  decides everything                       decides nothing
//!    │  place / show / foreground / close     │
//!    ├──── one line of JSON for each ────────►│ SetWindowPos, ShowWindow,
//!    │◄─── one line of JSON as the answer ────┤ SetForegroundWindow, WM_CLOSE
//! ```
//!
//! - **What it may be asked** is the whole of its power, and it is little:
//!   put a top-level window somewhere, show or hide it, minimise, maximise
//!   or restore it, give it the keyboard, ask it to close. It starts no
//!   program, reads and writes no file, and changes no window that is not
//!   a top-level window of another program. So the worst a program
//!   without rights can do through it is what it can already do to every
//!   window that is not elevated.
//! - **Who may ask** is the person the helper runs for: the pipe is theirs
//!   alone, and the program at its other end must be the `gevrek-wm` that
//!   lies beside the helper.
//! - **A window manager that falls over** leaves no elevated window
//!   hidden: the helper shows again what it was asked to hide when the
//!   window manager goes.
//! - **Everything else stays where it was.** The window manager reads what
//!   it reads of every window, hears of every window through its own
//!   hooks, and asks the helper only where a window is beyond its reach
//!   (`dwm::reach`). Without a helper nothing is different from before:
//!   such windows are left alone.
//!
//! The helper ends when the window manager it serves does. It is started
//! with Windows' own question (`gevrek elevated start`), or without one by
//! a task that is registered once (`gevrek elevated install`), which the
//! window manager runs when it starts.
//!
//! **The keyboard and the mouse.** Windows keeps the keys and presses that
//! go to an elevated program's window from every program that is not
//! elevated: gevrek's own hooks hear nothing while such a window has the
//! keyboard. So the helper has the same two hooks (`hotkey.rs`), and
//! answers what goes to elevated windows, by the bindings the window
//! manager tells it. What it hears it tells the window manager over a
//! second pipe, and it tells little: a key or a button that a binding
//! names, and its release; never what is only typed. And a binding must
//! be one that types nothing (`hotkey::may_be_heard_by_helper`), so that
//! naming every letter as a "binding" is no way to read what is typed
//! into an elevated program.
//!
//! **A window's frame and look** (the boxes of its title bar, the title
//! bar itself, how solid it is, its colours and corners) are changed by
//! the helper too, each within what gevrek changes of any window: a few
//! bits of a window's style, how solid it is, and a handful of what the
//! compositor is told of a window.
//!
//! Not done for elevated windows: cutting a window at a display's edge
//! (`cut_columns_at_displays`), and the gestures of a touchpad while an
//! elevated window has the keyboard.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowLongW, GetWindowThreadProcessId, IsWindow, PostMessageW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongW, SetWindowPos, ShowWindow,
    ShowWindowAsync, GA_ROOT, GWL_EXSTYLE, GWL_STYLE, LWA_ALPHA, SET_WINDOW_POS_FLAGS,
    SHOW_WINDOW_CMD, WM_CLOSE,
};

/// What the two speak; a helper of another version is not used.
pub const VERSION: u32 = 3;

/// The bits of a window's style gevrek changes: the title bar and its
/// boxes, the frame to resize it by (taken away for fullscreen).
const STYLE_BITS: i32 = 0x00C0_0000 // WS_CAPTION
    | 0x0008_0000 // WS_SYSMENU
    | 0x0004_0000 // WS_THICKFRAME
    | 0x0002_0000 // WS_MINIMIZEBOX
    | 0x0001_0000; // WS_MAXIMIZEBOX
/// And of its extended style: that it can be less than solid, and the
/// edges a frame has (taken away for fullscreen).
const EXTENDED_BITS: i32 = 0x0008_0000 // WS_EX_LAYERED
    | 0x0000_0001 // WS_EX_DLGMODALFRAME
    | 0x0000_0100 // WS_EX_WINDOWEDGE
    | 0x0000_0200 // WS_EX_CLIENTEDGE
    | 0x0002_0000; // WS_EX_STATICEDGE
/// What the compositor is told of a window by gevrek: who draws its
/// frame, its transitions, the cloak, its corners, the colours of its
/// border and title bar, its backdrop.
const DWM_ATTRIBUTES: [u32; 7] = [2, 3, 13, 33, 34, 35, 38];

/// What the helper tells the window manager, on the second pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Heard {
    /// A key or a button that a binding names, or its release.
    Input(gevrek_engine::input::RawInputEvent),
    /// A window was taken with the Windows key and the mouse, or let go.
    DragStarted(isize),
    DragEnded(isize),
}

/// The name of the second pipe, by the first's.
pub fn heard_pipe(pipe: &str) -> String {
    format!("{pipe}-heard")
}

/// Where a window is put among the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum After {
    Top,
    Bottom,
    Topmost,
    NotTopmost,
    /// Below this window.
    Below(isize),
}

impl After {
    /// From what `SetWindowPos` takes as the window to insert after.
    pub fn of(after: Option<HWND>) -> Self {
        match after.map_or(0, |after| after.0 as isize) {
            0 => Self::Top,
            1 => Self::Bottom,
            -1 => Self::Topmost,
            -2 => Self::NotTopmost,
            window => Self::Below(window),
        }
    }

    fn hwnd(self) -> HWND {
        HWND(match self {
            Self::Top => 0,
            Self::Bottom => 1,
            Self::Topmost => -1,
            Self::NotTopmost => -2,
            Self::Below(window) => window,
        } as *mut _)
    }
}

/// One window of [`Ask::PlaceMany`]: where it is put and how large.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub window: isize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// What the window manager asks of the helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ask {
    Hello {
        version: u32,
    },
    /// `SetWindowPos` for several windows, each where it stands among the
    /// others, with the same `flags`: the windows of one frame of a
    /// motion, asked for at once. (Each used to be a question and an
    /// answer of its own, sixty times a second.) One that cannot be
    /// placed does not keep the others from it; the answer is the first
    /// refusal.
    PlaceMany {
        places: Vec<Placed>,
        flags: u32,
    },
    /// `SetWindowPos`.
    Place {
        window: isize,
        after: After,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    },
    /// `ShowWindow`, or `ShowWindowAsync` where `posted`.
    Show {
        window: isize,
        command: i32,
        posted: bool,
    },
    Foreground {
        window: isize,
    },
    /// Ask the window to close (`WM_CLOSE`).
    Close {
        window: isize,
    },
    /// The bits of a window's style (or its extended style) that gevrek
    /// changes, as `value` has them; every other bit stays.
    Style {
        window: isize,
        extended: bool,
        value: i32,
    },
    /// How solid a window is (it must be one that can be less than solid).
    Alpha {
        window: isize,
        alpha: u8,
    },
    /// One of the things the compositor is told of a window.
    Dwm {
        window: isize,
        attribute: u32,
        value: u32,
    },
    /// What the helper's hooks go by: the bindings, whether they rest,
    /// and whose the Windows key is.
    Input {
        registrations: Vec<(i32, gevrek_engine::input::Hotkey)>,
        suspended: bool,
        own_windows_key: bool,
        drag_with_windows_key: bool,
    },
}

/// What the helper answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// `hears`: the helper has its hooks, and tells what they hear.
    Hello {
        version: u32,
        hears: bool,
    },
    Done,
    /// Not done, and why.
    Refused(String),
}

/// The flags of `SetWindowPos` the helper passes on: where, how large,
/// where among the others, and how. Not the ones that show or hide a
/// window (that is `Show`, which the helper keeps count of).
const PLACE_FLAGS: u32 = 0x0001 // SWP_NOSIZE
    | 0x0002 // SWP_NOMOVE
    | 0x0004 // SWP_NOZORDER
    | 0x0010 // SWP_NOACTIVATE
    | 0x0020 // SWP_FRAMECHANGED
    | 0x0200 // SWP_NOOWNERZORDER
    | 0x0400 // SWP_NOSENDCHANGING
    | 0x4000; // SWP_ASYNCWINDOWPOS

const SW_HIDE: i32 = 0;
/// The last of the commands of `ShowWindow` that only show, hide,
/// minimise, maximise and restore (`SW_RESTORE`).
const SHOW_LAST: i32 = 9;

/// The name of the helper's pipe for an instance of gevrek and a person.
pub fn pipe_name(instance: &str) -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    let plain = |text: &str| -> String {
        text.chars()
            .map(|letter| {
                if letter.is_ascii_alphanumeric() {
                    letter
                } else {
                    '-'
                }
            })
            .collect()
    };
    format!(
        r"\\.\pipe\gevrek-elevated-{}-{}",
        plain(&user),
        plain(instance)
    )
}

/// The desktop as the helper touches it: every call that changes
/// something is one of these. The helper's own is `TheDesk`; a check has
/// one that only writes down what it was asked.
pub trait Desk {
    /// Whether a window is one the helper may be asked about: a top-level
    /// window that is there and is not the helper's own.
    fn may_be_asked_of(&mut self, window: isize) -> Result<(), String>;
    /// `SetWindowPos`.
    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        window: isize,
        after: After,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> Result<(), String>;
    /// `ShowWindow`, or `ShowWindowAsync` where `posted`.
    fn show(&mut self, window: isize, command: i32, posted: bool);
    fn foreground(&mut self, window: isize);
    /// Ask the window to close (`WM_CLOSE`).
    fn close(&mut self, window: isize) -> Result<(), String>;
    /// A window's style, or its extended style.
    fn style(&mut self, window: isize, extended: bool) -> i32;
    fn set_style(&mut self, window: isize, extended: bool, value: i32);
    fn alpha(&mut self, window: isize, alpha: u8) -> Result<(), String>;
    fn dwm(&mut self, window: isize, attribute: u32, value: u32) -> Result<(), String>;
    /// Whether a window is there (still).
    fn is_there(&mut self, window: isize) -> bool;
    /// Whether the helper's hooks run.
    fn hears(&self) -> bool;
    /// What the helper's hooks go by from now on.
    fn hear_by(
        &mut self,
        registrations: Vec<(i32, gevrek_engine::input::Hotkey)>,
        suspended: bool,
        own_windows_key: bool,
        drag_with_windows_key: bool,
    );
}

/// The desktop itself.
pub struct TheDesk;

fn hwnd(window: isize) -> HWND {
    HWND(window as *mut _)
}

impl Desk for TheDesk {
    fn may_be_asked_of(&mut self, window: isize) -> Result<(), String> {
        let hwnd = hwnd(window);
        // SAFETY: plain questions about a window that may be gone.
        unsafe {
            if !IsWindow(hwnd).as_bool() {
                return Err("no such window".to_string());
            }
            if GetAncestor(hwnd, GA_ROOT) != hwnd {
                return Err("not a top-level window".to_string());
            }
            let mut process = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut process));
            if process == std::process::id() {
                return Err("a window of the helper's own".to_string());
            }
        }
        Ok(())
    }

    fn place(
        &mut self,
        window: isize,
        after: After,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> Result<(), String> {
        // SAFETY: a plain call on a window that may be gone meanwhile.
        unsafe {
            SetWindowPos(
                hwnd(window),
                after.hwnd(),
                x,
                y,
                width,
                height,
                SET_WINDOW_POS_FLAGS(flags),
            )
        }
        .map_err(|error| error.to_string())
    }

    fn show(&mut self, window: isize, command: i32, posted: bool) {
        // SAFETY: plain calls on a window that may be gone meanwhile.
        unsafe {
            if posted {
                let _ = ShowWindowAsync(hwnd(window), SHOW_WINDOW_CMD(command));
            } else {
                let _ = ShowWindow(hwnd(window), SHOW_WINDOW_CMD(command));
            }
        }
    }

    fn foreground(&mut self, window: isize) {
        let hwnd = hwnd(window);
        // SAFETY: plain calls; an input of nothing makes this the
        // program that had the last input, which Windows asks for.
        unsafe {
            use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_MOUSE};
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
                return;
            }
            let nothing = INPUT {
                r#type: INPUT_MOUSE,
                ..Default::default()
            };
            SendInput(&[nothing], std::mem::size_of::<INPUT>() as i32);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    fn close(&mut self, window: isize) -> Result<(), String> {
        // SAFETY: posted, never sent: a program that hangs is not waited for.
        unsafe { PostMessageW(hwnd(window), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .map_err(|error| error.to_string())
    }

    fn style(&mut self, window: isize, extended: bool) -> i32 {
        let which = if extended { GWL_EXSTYLE } else { GWL_STYLE };
        // SAFETY: a plain call on a window that may be gone meanwhile.
        unsafe { GetWindowLongW(hwnd(window), which) }
    }

    fn set_style(&mut self, window: isize, extended: bool, value: i32) {
        let which = if extended { GWL_EXSTYLE } else { GWL_STYLE };
        // SAFETY: a plain call on a window that may be gone meanwhile.
        unsafe {
            SetWindowLongW(hwnd(window), which, value);
        }
    }

    fn alpha(&mut self, window: isize, alpha: u8) -> Result<(), String> {
        // SAFETY: a plain call on a window that may be gone meanwhile.
        unsafe {
            SetLayeredWindowAttributes(
                hwnd(window),
                windows::Win32::Foundation::COLORREF(0),
                alpha,
                LWA_ALPHA,
            )
        }
        .map_err(|error| error.to_string())
    }

    fn dwm(&mut self, window: isize, attribute: u32, value: u32) -> Result<(), String> {
        // SAFETY: the value lives until the call returns, and is of
        // the size every one of these attributes has.
        unsafe {
            windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(
                hwnd(window),
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(attribute as i32),
                &value as *const u32 as *const core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            )
        }
        .map_err(|error| error.to_string())
    }

    fn is_there(&mut self, window: isize) -> bool {
        // SAFETY: a plain question about a window that may be gone.
        unsafe { IsWindow(hwnd(window)).as_bool() }
    }

    fn hears(&self) -> bool {
        HEARS.load(std::sync::atomic::Ordering::Acquire)
    }

    fn hear_by(
        &mut self,
        registrations: Vec<(i32, gevrek_engine::input::Hotkey)>,
        suspended: bool,
        own_windows_key: bool,
        drag_with_windows_key: bool,
    ) {
        crate::hotkey::set_helper_table(registrations.into_iter().collect(), suspended);
        crate::windows_key::configure(own_windows_key, drag_with_windows_key);
    }
}

/// Do what was asked. `hidden`: the windows hidden on the asker's behalf
/// (shown again when the asker goes: `show_hidden`).
pub fn carry_out(ask: &Ask, hidden: &mut HashSet<isize>) -> Answer {
    carry_out_on(&mut TheDesk, ask, hidden)
}

/// As `carry_out`, on the desktop that is given.
pub fn carry_out_on(desk: &mut impl Desk, ask: &Ask, hidden: &mut HashSet<isize>) -> Answer {
    let done = |result: Result<(), String>| match result {
        Ok(()) => Answer::Done,
        Err(why) => Answer::Refused(why),
    };
    match *ask {
        Ask::Hello { version } => {
            if version == VERSION {
                Answer::Hello {
                    version: VERSION,
                    hears: desk.hears(),
                }
            } else {
                Answer::Refused(format!("this helper speaks version {VERSION}"))
            }
        }
        Ask::PlaceMany { ref places, flags } => {
            // (Not among the others: `SWP_NOZORDER`, whatever was asked.)
            const SWP_NOZORDER: u32 = 0x0004;
            let flags = (flags & PLACE_FLAGS) | SWP_NOZORDER;
            let mut refused = None;
            for placed in places {
                let done = desk.may_be_asked_of(placed.window).and_then(|()| {
                    desk.place(
                        placed.window,
                        After::Top,
                        placed.x,
                        placed.y,
                        placed.width,
                        placed.height,
                        flags,
                    )
                });
                if let Err(why) = done {
                    refused.get_or_insert(why);
                }
            }
            done(refused.map_or(Ok(()), Err))
        }
        Ask::Place {
            window,
            after,
            x,
            y,
            width,
            height,
            flags,
        } => done(desk.may_be_asked_of(window).and_then(|()| {
            if let After::Below(other) = after {
                desk.may_be_asked_of(other)?;
            }
            desk.place(window, after, x, y, width, height, flags & PLACE_FLAGS)
        })),
        Ask::Show {
            window,
            command,
            posted,
        } => done(desk.may_be_asked_of(window).and_then(|()| {
            if !(SW_HIDE..=SHOW_LAST).contains(&command) {
                return Err(format!("{command} is no way to show a window"));
            }
            if command == SW_HIDE {
                hidden.insert(window);
            } else {
                hidden.remove(&window);
            }
            desk.show(window, command, posted);
            Ok(())
        })),
        Ask::Foreground { window } => done(
            desk.may_be_asked_of(window)
                .map(|()| desk.foreground(window)),
        ),
        Ask::Close { window } => done(
            desk.may_be_asked_of(window)
                .and_then(|()| desk.close(window)),
        ),
        Ask::Style {
            window,
            extended,
            value,
        } => done(desk.may_be_asked_of(window).map(|()| {
            let bits = if extended { EXTENDED_BITS } else { STYLE_BITS };
            let now = desk.style(window, extended);
            let wanted = (now & !bits) | (value & bits);
            if wanted != now {
                desk.set_style(window, extended, wanted);
            }
        })),
        Ask::Alpha { window, alpha } => done(
            desk.may_be_asked_of(window)
                .and_then(|()| desk.alpha(window, alpha)),
        ),
        Ask::Dwm {
            window,
            attribute,
            value,
        } => done(desk.may_be_asked_of(window).and_then(|()| {
            if !DWM_ATTRIBUTES.contains(&attribute) {
                return Err(format!(
                    "{attribute} is nothing gevrek tells the compositor"
                ));
            }
            // (13: cloaked. Remembered, to be undone when the asker goes.)
            if attribute == 13 {
                let mut cloaked = cloaked();
                if value != 0 {
                    cloaked.insert(window);
                } else {
                    cloaked.remove(&window);
                }
            }
            desk.dwm(window, attribute, value)
        })),
        Ask::Input {
            ref registrations,
            suspended,
            own_windows_key,
            drag_with_windows_key,
        } => {
            // (Only bindings that type nothing: see the top of this file.)
            let (heard, left_out): (Vec<_>, Vec<_>) = registrations
                .iter()
                .cloned()
                .partition(|(_, hotkey)| crate::hotkey::may_be_heard_by_helper(hotkey));
            if !left_out.is_empty() {
                log::info!(
                    "{} binding(s) of a plain key are not heard over elevated windows",
                    left_out.len()
                );
            }
            desk.hear_by(heard, suspended, own_windows_key, drag_with_windows_key);
            Answer::Done
        }
    }
}

/// Whether the helper's hooks run (the helper's own side).
static HEARS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The windows cloaked on the asker's behalf.
fn cloaked() -> std::sync::MutexGuard<'static, HashSet<isize>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<HashSet<isize>>> = std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Show again every window that was hidden on the asker's behalf.
pub fn show_hidden(hidden: &mut HashSet<isize>) -> usize {
    show_hidden_on(&mut TheDesk, hidden)
}

/// As `show_hidden`, on the desktop that is given.
pub fn show_hidden_on(desk: &mut impl Desk, hidden: &mut HashSet<isize>) -> usize {
    let mut shown = 0;
    for window in hidden.drain() {
        // (A window that is gone is skipped.)
        if desk.is_there(window) {
            // (8: SW_SHOWNA, as it is and without the keyboard.)
            desk.show(window, 8, true);
            shown += 1;
        }
    }
    // And every window that was cloaked is seen again. (Only what was
    // hidden was shown: a window of an elevated program that was cloaked
    // when the window manager went stayed unseen until its program was
    // started again, and the window manager itself may not uncloak it.)
    let uncloak: Vec<isize> = cloaked().drain().collect();
    for window in uncloak {
        if desk.is_there(window) && desk.dwm(window, 13, 0).is_ok() {
            shown += 1;
        }
    }
    shown
}

/// Answer one asker until it goes: a line for each thing asked, a line
/// for each answer. What it had hidden is shown again at the end.
pub fn answer_until_gone(reader: impl std::io::Read, writer: impl Write) {
    answer_until_gone_on(&mut TheDesk, reader, writer)
}

/// As `answer_until_gone`, on the desktop that is given.
pub fn answer_until_gone_on(
    desk: &mut impl Desk,
    reader: impl std::io::Read,
    mut writer: impl Write,
) {
    let mut hidden = HashSet::new();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else {
            break;
        };
        let answer = match serde_json::from_str::<Ask>(&line) {
            Ok(ask) => carry_out_on(desk, &ask, &mut hidden),
            Err(error) => Answer::Refused(format!("not understood: {error}")),
        };
        let Ok(mut text) = serde_json::to_string(&answer) else {
            break;
        };
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() || writer.flush().is_err() {
            break;
        }
    }
    let shown = show_hidden_on(desk, &mut hidden);
    if shown > 0 {
        log::warn!("the window manager went: {shown} window(s) it had hidden are shown again");
    }
}

/// The helper's own side: the pipe, and who may open it.
pub mod server {
    use super::*;
    use std::os::windows::io::FromRawHandle;
    use windows::core::{HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// The person this program runs for, as Windows writes them (`S-1-5-…`).
    pub fn own_user() -> anyhow::Result<String> {
        // SAFETY: the token is closed below; the buffer is as large as
        // Windows says the answer is, and the text Windows makes of it is
        // freed once it is copied.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            let mut buffer = vec![0u8; needed as usize];
            let asked = GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &mut needed,
            );
            let _ = CloseHandle(token);
            asked?;
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text)?;
            let written = text.to_string()?;
            let _ = LocalFree(HLOCAL(text.0.cast()));
            Ok(written)
        }
    }

    /// The file of a running program.
    pub(super) fn image_of(process: u32) -> Option<std::path::PathBuf> {
        // SAFETY: the handle is closed below; the buffer is ours.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process).ok()?;
            let mut name = [0u16; 1024];
            let mut length = name.len() as u32;
            let asked = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(name.as_mut_ptr()),
                &mut length,
            );
            let _ = CloseHandle(handle);
            asked.ok()?;
            Some(String::from_utf16_lossy(&name[..length as usize]).into())
        }
    }

    /// Whether the program at the other end of the pipe is the window
    /// manager that lies beside this helper.
    pub fn is_our_window_manager(process: u32) -> bool {
        is_window_manager(process, None)
    }

    /// As `is_our_window_manager`, where the helper does not lie beside
    /// the window manager it serves (the copy the task starts lies where
    /// only an administrator can write): the window manager is the file
    /// the task was registered with.
    pub fn is_window_manager(process: u32, named: Option<&std::path::Path>) -> bool {
        let same = |left: &std::path::Path, right: &std::path::Path| {
            let whole = |path: &std::path::Path| {
                std::fs::canonicalize(path)
                    .unwrap_or_else(|_| path.to_path_buf())
                    .to_string_lossy()
                    .to_lowercase()
            };
            whole(left) == whole(right)
        };
        let Some(theirs) = image_of(process) else {
            return false;
        };
        match named {
            Some(named) => same(named, &theirs),
            None => std::env::current_exe()
                .is_ok_and(|own| same(&own.with_file_name("gevrek-wm.exe"), &theirs)),
        }
    }

    /// Make a pipe that is the person's alone (with the system and the
    /// administrators, who can do all of this anyway; and with `also`,
    /// where the helper runs as another person than the one it is for),
    /// and that a program without an administrator's rights may write
    /// to: that is what it is for.
    fn make_pipe(pipe: &str, also: Option<&str>) -> anyhow::Result<HANDLE> {
        let also = also
            .map(|person| format!("(A;;GRGW;;;{person})"))
            .unwrap_or_default();
        let rights = HSTRING::from(format!(
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{}){also}S:(ML;;NW;;;ME)",
            own_user()?
        ));
        let name = HSTRING::from(pipe);
        // SAFETY: the descriptor Windows makes lives until it is freed
        // below, after the pipe was made with it.
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &rights,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let handle = CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                64 * 1024,
                64 * 1024,
                0,
                Some(&attributes),
            );
            let _ = LocalFree(HLOCAL(descriptor.0));
            if handle.is_invalid() {
                anyhow::bail!(
                    "the pipe {pipe} could not be made: {}",
                    std::io::Error::last_os_error()
                );
            }
            Ok(handle)
        }
    }

    /// Wait for the other end of a pipe, and say which process it is.
    /// The pipe becomes a file (which closes it).
    fn wait_for_other_end(handle: HANDLE) -> anyhow::Result<(std::fs::File, u32)> {
        // SAFETY: the handle is a pipe's of this program, and becomes a
        // file that closes it.
        unsafe {
            // (535: the other end was faster than this call, and is there.)
            if let Err(error) = ConnectNamedPipe(handle, None) {
                if error.code().0 as u32 & 0xFFFF != 535 {
                    let _ = CloseHandle(handle);
                    return Err(error.into());
                }
            }
            let mut process = 0u32;
            let known = GetNamedPipeClientProcessId(handle, &mut process).is_ok();
            let file = std::fs::File::from_raw_handle(handle.0 as _);
            anyhow::ensure!(known, "Windows does not say who opened the helper's pipe");
            Ok((file, process))
        }
    }

    /// A person as Windows writes them, and nothing else: it is put into
    /// the words that say whose the pipe is.
    pub fn is_a_person(text: &str) -> bool {
        text.starts_with("S-1-")
            && text.len() <= 184
            && text[4..]
                .chars()
                .all(|letter| letter.is_ascii_digit() || letter == '-')
    }

    /// How the helper serves.
    pub struct Serve<F: Fn(u32) -> bool> {
        /// Whether a process that opened the pipe may ask.
        pub may_ask: F,
        /// Have the hooks for the keyboard and the mouse (not in checks:
        /// they would hear this machine's own keys).
        pub hooks: bool,
        /// The person the helper is for, where it runs as another (an
        /// administrator's name was given to Windows' question).
        pub also_for: Option<String>,
    }

    /// Make the pipes, wait for the one that may ask, and answer it until
    /// it goes. An error where a pipe cannot be made (another helper has
    /// it) or the one that opened it may not ask.
    pub fn serve_once(pipe: &str, serve: Serve<impl Fn(u32) -> bool>) -> anyhow::Result<()> {
        let also = serve
            .also_for
            .as_deref()
            .filter(|person| is_a_person(person));
        // (The second pipe first: it is there when the first is answered.)
        let heard_pipe = make_pipe(&heard_pipe(pipe), also)?;
        let asked_pipe = make_pipe(pipe, also)?;
        let (told, to_tell) = std::sync::mpsc::channel::<Heard>();
        if serve.hooks {
            let tell = told.clone();
            match crate::hotkey::run_helper_hooks(move |raw| {
                let _ = tell.send(Heard::Input(raw));
            }) {
                Ok(()) => {
                    HEARS.store(true, std::sync::atomic::Ordering::Release);
                    // What the Windows key and the mouse did to a window.
                    let tell = told.clone();
                    std::thread::spawn(move || loop {
                        // (Asleep until a drag begins or ends: it looked
                        // thirty times a second, drag or no drag.)
                        let events =
                            crate::windows_key::wait_for_events(std::time::Duration::from_secs(30));
                        for event in events {
                            let heard = match event {
                                crate::windows_key::DragEvent::Started(window) => {
                                    Heard::DragStarted(window)
                                }
                                crate::windows_key::DragEvent::Ended(window) => {
                                    Heard::DragEnded(window)
                                }
                            };
                            if tell.send(heard).is_err() {
                                return;
                            }
                        }
                    });
                }
                Err(error) => log::warn!("{error}: gevrek's keys do nothing over elevated windows"),
            }
        }
        // Whoever opens the pipe and may not ask is let go of, and the
        // helper waits again. (It ended with the first one that opened
        // its pipe, whoever that was: any program of the person's could
        // end it by opening the pipe, and no elevated window was managed
        // until gevrek was started again.)
        let mut made = Some(asked_pipe);
        let (file, process) = loop {
            let waiting = match made.take() {
                Some(waiting) => waiting,
                None => make_pipe(pipe, also)?,
            };
            let (file, process) = wait_for_other_end(waiting)?;
            if (serve.may_ask)(process) {
                break (file, process);
            }
            log::warn!(
                "process {process} opened the helper's pipe and may not ask: it is let go of"
            );
            drop(file);
        };
        // The second pipe: opened by the same program, told what is heard.
        // (A handle is a number: carried to the thread as one.)
        let heard_pipe = heard_pipe.0 as isize;
        std::thread::spawn(move || {
            let Ok((mut file, other)) = wait_for_other_end(HANDLE(heard_pipe as *mut _)) else {
                return;
            };
            if other != process {
                log::warn!("process {other} opened the pipe of what is heard and is not told");
                return;
            }
            while let Ok(heard) = to_tell.recv() {
                let Ok(mut line) = serde_json::to_string(&heard) else {
                    continue;
                };
                line.push('\n');
                if file.write_all(line.as_bytes()).is_err() || file.flush().is_err() {
                    return;
                }
            }
        });
        let writer = file.try_clone()?;
        answer_until_gone(file, writer);
        Ok(())
    }
}

/// The window manager's side.
pub mod client {
    use super::*;

    struct Line {
        reader: BufReader<std::fs::File>,
        writer: std::fs::File,
    }

    static LINE: Mutex<Option<Line>> = Mutex::new(None);

    fn line() -> std::sync::MutexGuard<'static, Option<Line>> {
        LINE.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn ask_on(line: &mut Line, ask: &Ask) -> std::io::Result<Answer> {
        let mut text = serde_json::to_string(ask).map_err(std::io::Error::other)?;
        text.push('\n');
        line.writer.write_all(text.as_bytes())?;
        line.writer.flush()?;
        let mut answer = String::new();
        if line.reader.read_line(&mut answer)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        serde_json::from_str(&answer).map_err(std::io::Error::other)
    }

    /// What the helper's hooks go by, as the window manager has it now.
    #[derive(Default)]
    struct Table {
        registrations: Vec<(i32, gevrek_engine::input::Hotkey)>,
        suspended: bool,
        own_windows_key: bool,
        drag_with_windows_key: bool,
    }

    static TABLE: Mutex<Table> = Mutex::new(Table {
        registrations: Vec::new(),
        suspended: false,
        own_windows_key: false,
        drag_with_windows_key: false,
    });

    fn table() -> std::sync::MutexGuard<'static, Table> {
        TABLE.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Tell the helper what its hooks go by (where one answers).
    /// Tell the helper the table as it is now, from a thread of its own.
    /// (It was told from whichever thread changed the table, and that is
    /// the thread that hears the keys when bindings are taken or let go:
    /// a helper that was slow to answer, held up by a window that does
    /// not answer, held every key and the pointer of the whole system up
    /// with it.) Several changes in a row are told as one: the table that
    /// is told is the one there is when its turn comes.
    #[cfg(not(test))]
    fn tell_table() {
        static TELLER: std::sync::OnceLock<Option<std::sync::mpsc::SyncSender<()>>> =
            std::sync::OnceLock::new();
        let teller = TELLER.get_or_init(|| {
            // (Room for one that waits: more than one would tell the same.)
            let (wake, woken) = std::sync::mpsc::sync_channel::<()>(1);
            std::thread::Builder::new()
                .name("gevrek-helper-table".to_string())
                .spawn(move || {
                    for () in woken {
                        tell_table_now();
                    }
                })
                .ok()
                .map(|_| wake)
        });
        match teller {
            // (Full: one is waiting already, and will tell this table.)
            Some(wake) => {
                let _ = wake.try_send(());
            }
            None => tell_table_now(),
        }
    }

    /// (In the tests the table is told at once, on the thread that asks.)
    #[cfg(test)]
    fn tell_table() {
        tell_table_now();
    }

    fn tell_table_now() {
        let ask = {
            let table = table();
            Ask::Input {
                registrations: table.registrations.clone(),
                suspended: table.suspended,
                own_windows_key: table.own_windows_key,
                drag_with_windows_key: table.drag_with_windows_key,
            }
        };
        let _ = self::ask(&ask);
    }

    /// The bindings are these now.
    pub fn set_bindings(
        registrations: &std::collections::HashMap<i32, gevrek_engine::input::Hotkey>,
    ) {
        let mut sorted: Vec<_> = registrations
            .iter()
            .map(|(id, hotkey)| (*id, hotkey.clone()))
            .collect();
        sorted.sort_by_key(|(id, _)| *id);
        {
            let mut table = table();
            if table.registrations == sorted {
                return;
            }
            table.registrations = sorted;
        }
        tell_table();
    }

    /// The bindings rest (the window manager is paused), or no longer.
    pub fn set_suspended(suspended: bool) {
        {
            let mut table = table();
            if table.suspended == suspended {
                return;
            }
            table.suspended = suspended;
        }
        tell_table();
    }

    /// Whose the Windows key is, and whether it drags windows.
    pub fn set_windows_key(own: bool, drag: bool) {
        {
            let mut table = table();
            if (table.own_windows_key, table.drag_with_windows_key) == (own, drag) {
                return;
            }
            table.own_windows_key = own;
            table.drag_with_windows_key = drag;
        }
        tell_table();
    }

    /// Whether the program at the other end of the pipe is gevrek's
    /// helper: the copy kept where only an administrator can write, or the
    /// one beside this window manager.
    #[cfg(not(test))]
    fn served_by_the_helper(pipe: &std::fs::File) -> bool {
        use std::os::windows::io::AsRawHandle;
        let mut process = 0u32;
        // SAFETY: the handle is the open pipe's, and the number is ours.
        let asked = unsafe {
            windows::Win32::System::Pipes::GetNamedPipeServerProcessId(
                windows::Win32::Foundation::HANDLE(pipe.as_raw_handle()),
                &mut process,
            )
        };
        if asked.is_err() {
            return false;
        }
        let Some(theirs) = super::server::image_of(process) else {
            return false;
        };
        let whole = |path: &std::path::Path| {
            std::fs::canonicalize(path)
                .unwrap_or_else(|_| path.to_path_buf())
                .to_string_lossy()
                .to_lowercase()
        };
        let beside = std::env::current_exe()
            .ok()
            .map(|own| own.with_file_name("gevrek-elevated.exe"));
        let theirs = whole(&theirs);
        theirs == whole(&crate::elevated::kept_helper())
            || beside.is_some_and(|beside| theirs == whole(&beside))
    }

    /// Whether a helper answers.
    pub fn connected() -> bool {
        line().is_some()
    }

    /// Hear what the helper hears: its second pipe, read on a thread of
    /// its own until the helper goes.
    fn hear(pipe: &str) -> bool {
        let name = heard_pipe(pipe);
        let opened = (0..50).find_map(|_| {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&name)
            {
                Ok(file) => Some(file),
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                }
            }
        });
        let Some(file) = opened else {
            log::debug!("the helper's pipe of what it hears did not open: {name}");
            return false;
        };
        std::thread::spawn(move || {
            for line in BufReader::new(file).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(error) => {
                        log::debug!("what the helper hears is heard no more: {error}");
                        break;
                    }
                };
                match serde_json::from_str::<Heard>(&line) {
                    Ok(Heard::Input(raw)) => crate::hotkey::heard_by_helper(raw),
                    Ok(Heard::DragStarted(window)) => crate::windows_key::push_event(
                        crate::windows_key::DragEvent::Started(window),
                    ),
                    Ok(Heard::DragEnded(window)) => {
                        crate::windows_key::push_event(crate::windows_key::DragEvent::Ended(window))
                    }
                    Err(error) => log::debug!("what the helper told was not understood: {error}"),
                }
            }
            log::debug!("the helper's pipe of what it hears is closed");
            crate::hotkey::set_helper_answers(false);
        });
        true
    }

    /// Open the helper's pipe, if a helper of this version is there.
    /// Returns whether one answers now.
    pub fn connect(pipe: &str) -> bool {
        {
            let mut held = line();
            if held.is_some() {
                return true;
            }
            let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(pipe)
            else {
                return false;
            };
            // What answers there is the helper, and not whoever made a
            // pipe of that name first: a pipe's name is the machine's, and
            // the window manager told whatever served it every key it
            // listens for, and ran what it sent back as keys pressed.
            #[cfg(not(test))]
            if !served_by_the_helper(&file) {
                static SAID: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    log::warn!("what serves {pipe} is not gevrek's helper: it is not spoken to");
                }
                return false;
            }
            let Ok(writer) = file.try_clone() else {
                return false;
            };
            let mut made = Line {
                reader: BufReader::new(file),
                writer,
            };
            match ask_on(&mut made, &Ask::Hello { version: VERSION }) {
                Ok(Answer::Hello { version, hears }) if version == VERSION => {
                    log::info!("the helper for elevated windows answers on {pipe}");
                    *held = Some(made);
                    // (Its hooks answer for elevated windows from now on,
                    // and the window manager's own leave those to it.)
                    let hears = hears && hear(pipe);
                    crate::hotkey::set_helper_answers(hears);
                    if !hears {
                        log::info!("the helper has no hooks: gevrek's keys do nothing over elevated windows");
                    }
                }
                other => {
                    // (Said once: it is asked for again and again.)
                    static SAID: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        log::warn!(
                            "what answers on {pipe} is no helper of this version ({other:?}): after an update, `gevrek elevated install` keeps the new one"
                        );
                    }
                    return false;
                }
            }
        }
        tell_table();
        true
    }

    /// Let the helper go (it shows what it had hidden, and ends).
    pub fn disconnect() {
        log::debug!("the helper for elevated windows is let go");
        *line() = None;
        crate::hotkey::set_helper_answers(false);
    }

    /// Ask the helper. `None` where none answers; a helper that went
    /// away is no longer asked.
    pub fn ask(ask: &Ask) -> Option<Answer> {
        let mut held = line();
        let made = held.as_mut()?;
        match ask_on(made, ask) {
            Ok(answer) => Some(answer),
            Err(error) => {
                log::warn!("the helper for elevated windows went away: {error}");
                *held = None;
                crate::hotkey::set_helper_answers(false);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// What a desktop was asked to do.
    #[derive(Debug, Clone, PartialEq)]
    enum Did {
        Place(isize, After, u32),
        Show(isize, i32, bool),
        Foreground(isize),
        Close(isize),
        Style(isize, bool, i32, i32),
        Alpha(isize, u8),
        Dwm(isize, u32, u32),
        HearBy(Vec<(i32, gevrek_engine::input::Hotkey)>),
    }

    /// A desktop that does nothing and writes down what it was asked.
    #[derive(Default)]
    struct Written {
        /// The windows that may be asked about.
        there: HashSet<isize>,
        styles: HashMap<(isize, bool), i32>,
        did: Vec<Did>,
    }

    impl Written {
        fn with(windows: &[isize]) -> Self {
            Self {
                there: windows.iter().copied().collect(),
                ..Self::default()
            }
        }
    }

    impl Desk for Written {
        fn may_be_asked_of(&mut self, window: isize) -> Result<(), String> {
            if self.there.contains(&window) {
                Ok(())
            } else {
                Err("no such window".to_string())
            }
        }

        fn place(
            &mut self,
            window: isize,
            after: After,
            _x: i32,
            _y: i32,
            _width: i32,
            _height: i32,
            flags: u32,
        ) -> Result<(), String> {
            self.did.push(Did::Place(window, after, flags));
            Ok(())
        }

        fn show(&mut self, window: isize, command: i32, posted: bool) {
            self.did.push(Did::Show(window, command, posted));
        }

        fn foreground(&mut self, window: isize) {
            self.did.push(Did::Foreground(window));
        }

        fn close(&mut self, window: isize) -> Result<(), String> {
            self.did.push(Did::Close(window));
            Ok(())
        }

        fn style(&mut self, window: isize, extended: bool) -> i32 {
            self.styles.get(&(window, extended)).copied().unwrap_or(0)
        }

        fn set_style(&mut self, window: isize, extended: bool, value: i32) {
            let before = self.style(window, extended);
            self.styles.insert((window, extended), value);
            self.did.push(Did::Style(window, extended, before, value));
        }

        fn alpha(&mut self, window: isize, alpha: u8) -> Result<(), String> {
            self.did.push(Did::Alpha(window, alpha));
            Ok(())
        }

        fn dwm(&mut self, window: isize, attribute: u32, value: u32) -> Result<(), String> {
            self.did.push(Did::Dwm(window, attribute, value));
            Ok(())
        }

        fn is_there(&mut self, window: isize) -> bool {
            self.there.contains(&window)
        }

        fn hears(&self) -> bool {
            true
        }

        fn hear_by(
            &mut self,
            registrations: Vec<(i32, gevrek_engine::input::Hotkey)>,
            _suspended: bool,
            _own_windows_key: bool,
            _drag_with_windows_key: bool,
        ) {
            self.did.push(Did::HearBy(registrations));
        }
    }

    /// What a desktop was asked to do that is more than the helper may
    /// do, if anything.
    fn beyond(desk: &Written) -> Option<String> {
        for did in &desk.did {
            let (window, wrong) = match did {
                Did::Place(window, after, flags) => {
                    let below = match after {
                        After::Below(other) => !desk.there.contains(other),
                        _ => false,
                    };
                    (Some(*window), flags & !PLACE_FLAGS != 0 || below)
                }
                Did::Show(window, command, _) => {
                    (Some(*window), !(SW_HIDE..=SHOW_LAST).contains(command))
                }
                Did::Style(window, extended, before, after) => {
                    let bits = if *extended { EXTENDED_BITS } else { STYLE_BITS };
                    (Some(*window), (before ^ after) & !bits != 0)
                }
                Did::Dwm(window, attribute, _) => {
                    (Some(*window), !DWM_ATTRIBUTES.contains(attribute))
                }
                Did::Foreground(window) | Did::Close(window) | Did::Alpha(window, _) => {
                    (Some(*window), false)
                }
                Did::HearBy(registrations) => (
                    None,
                    registrations
                        .iter()
                        .any(|(_, hotkey)| !crate::hotkey::may_be_heard_by_helper(hotkey)),
                ),
            };
            if wrong || window.is_some_and(|window| !desk.there.contains(&window)) {
                return Some(format!("{did:?}"));
            }
        }
        // What was hidden last and is there still, is shown at the end.
        let mut hidden: HashSet<isize> = HashSet::new();
        for did in &desk.did {
            if let Did::Show(window, command, _) = did {
                if *command == SW_HIDE {
                    hidden.insert(*window);
                } else {
                    hidden.remove(window);
                }
            }
        }
        hidden
            .iter()
            .next()
            .map(|window| format!("{window} is left hidden"))
    }

    /// One of everything that may be asked.
    fn one_of_each() -> Vec<Ask> {
        let key = |key: &str, win: bool| {
            gevrek_engine::input::Hotkey::new(key.to_string(), false, false, false, win)
        };
        vec![
            Ask::Hello { version: VERSION },
            Ask::PlaceMany {
                places: vec![
                    Placed {
                        window: 10,
                        x: -8,
                        y: 0,
                        width: 800,
                        height: 600,
                    },
                    Placed {
                        window: 20,
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                ],
                flags: 0x14,
            },
            Ask::Place {
                window: 10,
                after: After::Below(20),
                x: -8,
                y: 0,
                width: 800,
                height: 600,
                flags: 0x14,
            },
            Ask::Place {
                window: 20,
                after: After::Topmost,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                flags: u32::MAX,
            },
            Ask::Show {
                window: 10,
                command: 0,
                posted: true,
            },
            Ask::Show {
                window: 20,
                command: 0,
                posted: false,
            },
            Ask::Show {
                window: 10,
                command: 9,
                posted: false,
            },
            Ask::Foreground { window: 20 },
            Ask::Close { window: 30 },
            Ask::Style {
                window: 10,
                extended: false,
                value: -1,
            },
            Ask::Style {
                window: 10,
                extended: true,
                value: 0x0008_0000,
            },
            Ask::Alpha {
                window: 20,
                alpha: 128,
            },
            Ask::Dwm {
                window: 20,
                attribute: 33,
                value: 2,
            },
            Ask::Input {
                registrations: vec![
                    (1, key("J", true)),
                    (2, key("A", false)),
                    (3, key("F5", false)),
                ],
                suspended: false,
                own_windows_key: true,
                drag_with_windows_key: true,
            },
        ]
    }

    #[test]
    fn what_is_asked_is_done_within_what_the_helper_may_do() {
        let mut desk = Written::with(&[10, 20]);
        desk.styles.insert((10, false), 0x1000_0000);
        let mut hidden = HashSet::new();
        let mut ask = |desk: &mut Written, ask: Ask| carry_out_on(desk, &ask, &mut hidden);

        // Every flag there is: the ones that show or hide are not passed on.
        let all = Ask::Place {
            window: 10,
            after: After::Top,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            flags: u32::MAX,
        };
        assert_eq!(ask(&mut desk, all), Answer::Done);
        assert_eq!(
            desk.did.pop(),
            Some(Did::Place(10, After::Top, PLACE_FLAGS))
        );
        assert_eq!(PLACE_FLAGS & (0x0040 | 0x0080), 0);

        // Below a window that may not be asked about: not at all.
        let below = Ask::Place {
            window: 10,
            after: After::Below(99),
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            flags: 0,
        };
        assert!(matches!(ask(&mut desk, below), Answer::Refused(_)));

        // Only what shows, hides, minimises, maximises and restores.
        for command in [-1, 10, 11, i32::MIN, i32::MAX] {
            let show = Ask::Show {
                window: 10,
                command,
                posted: false,
            };
            assert!(
                matches!(ask(&mut desk, show), Answer::Refused(_)),
                "{command}"
            );
        }
        assert!(desk.did.is_empty(), "{:?}", desk.did);

        // Of a style only gevrek's bits, and nothing where they are as asked.
        let style = |value| Ask::Style {
            window: 10,
            extended: false,
            value,
        };
        assert_eq!(ask(&mut desk, style(-1)), Answer::Done);
        assert_eq!(
            desk.did.pop(),
            Some(Did::Style(10, false, 0x1000_0000, 0x1000_0000 | STYLE_BITS))
        );
        assert_eq!(ask(&mut desk, style(STYLE_BITS)), Answer::Done);
        assert!(desk.did.is_empty(), "{:?}", desk.did);

        // A binding of a plain letter does not reach the hooks.
        let input = one_of_each().pop().unwrap();
        assert_eq!(ask(&mut desk, input), Answer::Done);
        let Some(Did::HearBy(heard)) = desk.did.pop() else {
            panic!("the hooks were told nothing");
        };
        assert_eq!(heard.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [1, 3]);

        // A window that may not be asked about is asked nothing.
        for ask_of_another in one_of_each() {
            let mut desk = Written::with(&[]);
            let answer = carry_out_on(&mut desk, &ask_of_another, &mut HashSet::new());
            let about_no_window = matches!(ask_of_another, Ask::Hello { .. } | Ask::Input { .. });
            assert_eq!(
                matches!(answer, Answer::Refused(_)),
                !about_no_window,
                "{ask_of_another:?}"
            );
            assert!(
                desk.did.iter().all(|did| matches!(did, Did::HearBy(_))),
                "{:?}",
                desk.did
            );
        }
    }

    #[test]
    fn what_was_hidden_is_shown_again_when_the_asker_goes() {
        let hide = |window| {
            serde_json::to_string(&Ask::Show {
                window,
                command: 0,
                posted: true,
            })
            .unwrap()
        };
        let show = serde_json::to_string(&Ask::Show {
            window: 20,
            command: 5,
            posted: true,
        })
        .unwrap();
        // 10 stays hidden, 20 is shown again by the asker, 30 is hidden
        // and is gone when the asker goes.
        let asked = format!("{}\n{}\n{show}\n{}\n", hide(10), hide(20), hide(30));
        let mut desk = Written::with(&[10, 20, 30]);
        let mut hidden = HashSet::new();
        for line in asked.lines() {
            let ask: Ask = serde_json::from_str(line).unwrap();
            assert_eq!(carry_out_on(&mut desk, &ask, &mut hidden), Answer::Done);
        }
        desk.there.remove(&30);
        desk.did.clear();
        assert_eq!(show_hidden_on(&mut desk, &mut hidden), 1);
        assert_eq!(desk.did, [Did::Show(10, 8, true)]);
        assert!(hidden.is_empty());

        // What was cloaked is seen again too; what the asker uncloaked
        // itself is left alone.
        let cloak = |window, value| Ask::Dwm {
            window,
            attribute: 13,
            value,
        };
        let mut desk = Written::with(&[40, 50]);
        for ask in [cloak(40, 1), cloak(50, 1), cloak(50, 0)] {
            assert_eq!(carry_out_on(&mut desk, &ask, &mut hidden), Answer::Done);
        }
        desk.did.clear();
        assert_eq!(show_hidden_on(&mut desk, &mut hidden), 1);
        assert_eq!(desk.did, [Did::Dwm(40, 13, 0)]);

        // And over the line: the asker goes after hiding, with no word.
        let mut desk = Written::with(&[10, 20]);
        let mut answered = Vec::new();
        answer_until_gone_on(
            &mut desk,
            format!("{}\n{}", hide(10), hide(20)).as_bytes(),
            &mut answered,
        );
        assert_eq!(beyond(&desk), None);
        assert_eq!(desk.did.len(), 4);
    }

    /// The helper has an administrator's rights and reads what a program
    /// without them writes: whatever is written, the helper comes to its
    /// end, answers each line with one, and does nothing beyond what it
    /// may do.
    #[test]
    fn no_asking_from_outside_ends_the_helper_or_takes_it_further() {
        use crate::spoil::{harder, lived, places, spoil, Chance};
        let whole: Vec<serde_json::Value> = one_of_each()
            .iter()
            .map(|ask| serde_json::to_value(ask).unwrap())
            .collect();
        let whole = serde_json::Value::Array(whole);
        let mut all = Vec::new();
        places(&whole, &mut Vec::new(), &mut all);

        let (times, seed) = harder();
        let mut chance = Chance(0x5be0_cd19_137e_2179 ^ seed.rotate_left(11) | 1);
        let mut wrong: Vec<String> = Vec::new();
        for _ in 0..2000 * times {
            let mut document = whole.clone();
            let mut done = Vec::new();
            for _ in 0..1 + chance.below(4) {
                done.push(spoil(&mut document, &all, &mut chance));
            }
            let lines: Vec<String> = match &document {
                serde_json::Value::Array(asks) => asks.iter().map(|ask| ask.to_string()).collect(),
                other => vec![other.to_string()],
            };
            let mut asked = lines.join("\n");
            // (Sometimes cut short, as by an asker that goes in the middle.)
            if chance.below(6) == 0 {
                let mut end = chance.below(asked.len() + 1);
                while !asked.is_char_boundary(end) {
                    end -= 1;
                }
                asked.truncate(end);
            }
            let found = std::sync::Arc::new(Mutex::new(None::<String>));
            let tell = found.clone();
            let text = asked.clone();
            let run = lived(move || {
                let mut desk = Written::with(&[10, 20, 30, -1, 0, 1]);
                let mut answered = Vec::new();
                answer_until_gone_on(&mut desk, text.as_bytes(), &mut answered);
                let answers = String::from_utf8(answered).unwrap_or_default();
                let mut said = beyond(&desk);
                if answers.lines().count() != text.lines().count() {
                    said = Some(format!(
                        "{} answers to {} lines",
                        answers.lines().count(),
                        text.lines().count()
                    ));
                }
                if answers
                    .lines()
                    .any(|line| serde_json::from_str::<Answer>(line).is_err())
                {
                    said = Some("an answer that is none".to_string());
                }
                *tell.lock().unwrap() = said;
            });
            let said = found.lock().unwrap().take();
            if let Some(why) = run.err().or(said) {
                wrong.push(format!("with {}: {why}", done.join(", ")));
            }
        }
        wrong.truncate(20);
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn what_is_asked_is_written_as_one_line_and_read_back() {
        let ask = Ask::Place {
            window: 0x1234,
            after: After::Below(0x99),
            x: -8,
            y: 0,
            width: 800,
            height: 600,
            flags: 0x14,
        };
        let line = serde_json::to_string(&ask).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Ask>(&line).unwrap(), ask);
    }

    #[test]
    fn the_places_among_the_others_are_windows_own_numbers() {
        assert_eq!(After::of(None), After::Top);
        assert_eq!(After::of(Some(After::Bottom.hwnd())), After::Bottom);
        assert_eq!(After::of(Some(HWND(-1isize as *mut _))), After::Topmost);
        assert_eq!(After::of(Some(HWND(-2isize as *mut _))), After::NotTopmost);
        assert_eq!(
            After::of(Some(HWND(0x50isize as *mut _))),
            After::Below(0x50)
        );
        assert_eq!(After::Below(0x50).hwnd().0 as isize, 0x50);
        assert_eq!(After::Topmost.hwnd().0 as isize, -1);
    }

    #[test]
    fn nothing_is_done_to_what_is_no_window_of_another_program() {
        let mut hidden = HashSet::new();
        // No window at all.
        let gone = Ask::Show {
            window: 0x7FFF_FFF0,
            command: 0,
            posted: true,
        };
        assert!(matches!(carry_out(&gone, &mut hidden), Answer::Refused(_)));
        assert!(hidden.is_empty());
        // A helper of another version.
        assert!(matches!(
            carry_out(
                &Ask::Hello {
                    version: VERSION + 1
                },
                &mut hidden
            ),
            Answer::Refused(_)
        ));
        assert_eq!(
            carry_out(&Ask::Hello { version: VERSION }, &mut hidden),
            Answer::Hello {
                version: VERSION,
                hears: false
            }
        );
        // What gevrek does not tell the compositor is not passed on.
        assert!(matches!(
            carry_out(
                &Ask::Dwm {
                    window: 0x7FFF_FFF0,
                    attribute: 20,
                    value: 1
                },
                &mut hidden
            ),
            Answer::Refused(_)
        ));
    }

    #[test]
    fn a_line_that_is_not_understood_is_refused_and_the_next_one_answered() {
        let asked = format!(
            "this is no question\n{}\n",
            serde_json::to_string(&Ask::Hello { version: VERSION }).unwrap()
        );
        let mut answered = Vec::new();
        answer_until_gone(asked.as_bytes(), &mut answered);
        let lines: Vec<Answer> = String::from_utf8(answered)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(matches!(lines[0], Answer::Refused(_)));
        assert!(matches!(
            lines[1],
            Answer::Hello {
                version: VERSION,
                ..
            }
        ));
    }

    #[test]
    fn only_a_person_as_windows_writes_them_is_put_into_the_pipes_rights() {
        assert!(server::is_a_person(
            "S-1-5-21-1004336348-1177238915-682003330-1001"
        ));
        assert!(!server::is_a_person("S-1-5-21)(A;;GA;;;WD"));
        assert!(!server::is_a_person("everyone"));
        assert!(!server::is_a_person(""));
    }

    #[test]
    fn what_is_heard_is_written_as_one_line_and_read_back() {
        let heard = Heard::Input(gevrek_engine::input::RawInputEvent {
            key: "J".to_string(),
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
            device: crate::settings::BindingDevice::Keyboard,
            down: true,
            momentary: false,
        });
        let line = serde_json::to_string(&heard).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<Heard>(&line).unwrap(), heard);
    }

    #[test]
    fn the_window_manager_and_the_helper_speak_over_a_pipe() {
        // (A pipe of this test's own; whoever opens it may ask.)
        let pipe = format!(r"\\.\pipe\gevrek-elevated-test-{}", std::process::id());
        let served = pipe.clone();
        let helper = std::thread::spawn(move || {
            server::serve_once(
                &served,
                server::Serve {
                    may_ask: |_| true,
                    hooks: false,
                    also_for: None,
                },
            )
        });
        let mut tries = 0;
        while !client::connect(&pipe) {
            tries += 1;
            assert!(tries < 200, "the helper's pipe never came");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(client::connected());
        // A window that is not there: asked, and refused, over the pipe.
        let answer = client::ask(&Ask::Close {
            window: 0x7FFF_FFF0,
        });
        assert!(matches!(answer, Some(Answer::Refused(_))));
        client::disconnect();
        assert!(!client::connected());
        helper.join().unwrap().unwrap();
    }
}
