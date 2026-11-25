//! Process-level safety nets that put hidden windows back on screen when the
//! WM goes away by a route that skips the clean shutdown in `Runtime::run`:
//! Ctrl+C or closing the console, or the user logging off / shutting down.
//!
//! Both funnel into [`hidden_journal::emergency_restore`], which works from
//! any thread without access to the runtime. The other gaps are covered
//! elsewhere: a runtime panic is caught in `wm.rs` and the `Runtime` drop
//! guard restores; a hard kill is recovered by `recover_orphans` on the next
//! start. There is deliberately no panic hook: panics in gRPC handler tasks
//! are caught by tokio while the runtime keeps running, and unhiding every
//! window then would desync the runtime's workspace state.

use crate::dwm::hidden_journal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    TranslateMessage, MSG, WINDOW_STYLE, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW,
    WS_EX_TOOLWINDOW,
};

static STOP_FLAG: OnceLock<Arc<Mutex<bool>>> = OnceLock::new();

/// Set once the runtime has shut down cleanly.
static SHUTDOWN_COMPLETE: AtomicBool = AtomicBool::new(false);

/// How long a console close / logoff / shutdown waits for the clean shutdown.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(4);

/// The runtime thread has finished its clean shutdown.
pub fn shutdown_complete() {
    SHUTDOWN_COMPLETE.store(true, Ordering::Release);
}

pub use gevrek_host::heartbeat::{beat, time_since_last_beat};

/// The session was about to end, the hidden windows were shown for it
/// (so their programs could ask to save), and the end was called off.
static SESSION_END_CALLED_OFF: AtomicBool = AtomicBool::new(false);

/// Whether a session's end was called off since this was last asked: the
/// windows that were shown for it are to be hidden again.
pub fn session_end_called_off() -> bool {
    SESSION_END_CALLED_OFF.swap(false, Ordering::AcqRel)
}

fn request_stop() {
    if let Some(flag) = STOP_FLAG.get() {
        *flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }
    // (The loop reads the flag at its next pass: now.)
    gevrek_host::wake::now();
}

fn restore_now(reason: &str) {
    let restored = hidden_journal::emergency_restore();
    if restored > 0 {
        log::warn!("{reason}: restored {restored} hidden window(s)");
    }
}

/// Install the console control handler and session-end watcher.
/// `stop_flag` is set so the normal shutdown path also runs when possible.
/// Call once, early in `main`.
pub fn install(stop_flag: Arc<Mutex<bool>>) {
    let _ = STOP_FLAG.set(stop_flag);
    install_console_handler();
    spawn_session_watcher();
}

unsafe extern "system" fn console_ctrl_handler(ctrl_type: u32) -> BOOL {
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
        | CTRL_SHUTDOWN_EVENT => {
            // Restore first: for CTRL_CLOSE the process is terminated soon
            // after this returns, so the graceful path may never finish.
            restore_now("Console control event");
            request_stop();
            if ctrl_type != CTRL_C_EVENT && ctrl_type != CTRL_BREAK_EVENT {
                // For close, logoff and shutdown Windows ends the process
                // as soon as this handler returns (and after about 5 s in
                // any case). Give the clean shutdown that time: it saves
                // the window state and stops the processes the WM started.
                let deadline = Instant::now() + SHUTDOWN_GRACE;
                while !SHUTDOWN_COMPLETE.load(Ordering::Acquire) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            BOOL::from(true)
        }
        _ => BOOL::from(false),
    }
}

fn install_console_handler() {
    // A process started with CREATE_NEW_PROCESS_GROUP, or by a parent that
    // ignores Ctrl+C, inherits "ignore Ctrl+C": the handler below would
    // never be called for it. Clear that first.
    // SAFETY: a null handler with `false` only clears the ignore flag.
    if let Err(e) = unsafe { SetConsoleCtrlHandler(None, false) } {
        log::debug!("Could not clear the inherited Ctrl+C ignore flag: {e}");
    }
    // SAFETY: registers a static extern "system" handler.
    if let Err(e) = unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), true) } {
        log::warn!("Could not install console control handler: {e}");
    }
}

unsafe extern "system" fn session_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Unhide before the session asks apps to close, so apps hidden on
        // other workspaces can show their "save changes?" prompts.
        WM_QUERYENDSESSION => {
            restore_now("Session ending");
            LRESULT(1)
        }
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                restore_now("Session ended");
                request_stop();
                // Windows ends the process when this returns: the clean
                // shutdown (the saved windows, the programs gevrek
                // started, the taskbar) gets the time it gets when a
                // console closes.
                let deadline = Instant::now() + SHUTDOWN_GRACE;
                while !SHUTDOWN_COMPLETE.load(Ordering::Acquire) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
            } else {
                // The end was called off (a program had unsaved work):
                // gevrek goes on, and its workspaces are as they were.
                SESSION_END_CALLED_OFF.store(true, Ordering::Release);
                gevrek_host::wake::now();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

/// GUI processes do not get CTRL_LOGOFF/CTRL_SHUTDOWN; they get
/// WM_QUERYENDSESSION/WM_ENDSESSION, which are only sent to top-level windows
/// (not message-only ones). A hidden tool window on its own thread receives
/// them even if the runtime thread is busy or hung.
fn spawn_session_watcher() {
    let spawned = std::thread::Builder::new()
        .name("gevrek-session-watch".into())
        .spawn(|| unsafe {
            let class_name = windows::core::w!("Gevrek_Session_Watch");
            let class = WNDCLASSW {
                lpfnWndProc: Some(session_window_proc),
                lpszClassName: class_name,
                ..WNDCLASSW::default()
            };
            let _ = RegisterClassW(&class);
            // Never shown: no WS_VISIBLE, zero size, tool window (no taskbar).
            if let Err(e) = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                windows::core::w!("Gevrek Session Watch"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            ) {
                log::warn!("Could not create session-end watcher window: {e}");
                return;
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    if let Err(e) = spawned {
        log::warn!("Could not start session-end watcher thread: {e}");
    }
}
