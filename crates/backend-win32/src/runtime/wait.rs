//! How the runtime loop waits between passes.
//!
//! The loop used to sleep one animation frame after every pass, sixty
//! passes a second with nothing to do. Now it waits for work: for the
//! signal the hooks, the input thread and the IPC server give when they
//! have queued something (`gevrek_host::wake`), for a message to one of
//! this thread's own windows, or for the time after which the next pass is
//! due anyway.
//!
//! Much of a pass still looks things up instead of being told (the
//! foreground window, the configuration file, the stop and pause flags,
//! started programs), so an idle loop passes every [`IDLE_INTERVAL`]. While
//! something moves, and for [`SETTLE`] after the last work, it passes once
//! a frame as before.

use std::sync::OnceLock;
use std::time::Duration;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    MsgWaitForMultipleObjectsEx, MWMO_INPUTAVAILABLE, QS_ALLINPUT,
};

/// The longest an idle loop waits before it looks around on its own.
pub const IDLE_INTERVAL: Duration = Duration::from_millis(250);

/// How long after the last work the loop keeps passing once a frame: what
/// follows work (a window settling into its place, a layout still to be
/// made) is often seen by looking, not announced.
pub const SETTLE: Duration = Duration::from_millis(500);

/// The state the clients read is put together at least this often,
/// whatever happened: for what changes without anything saying so.
pub const STATE_AT_THE_LATEST: Duration = Duration::from_secs(10);

/// What a pass found, as far as the next wait depends on it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Activity {
    /// Something on screen is moving and has to be advanced every frame.
    pub animating: bool,
    /// How long ago the loop last had work (a command, an event, a key).
    pub since_work: Option<Duration>,
    /// The mouse is watched for focus-follows-mouse, this often.
    pub watch_mouse_every: Option<Duration>,
}

/// How long to wait for work before the next pass.
pub fn interval(frame: Duration, activity: Activity) -> Duration {
    let frame = frame.max(Duration::from_millis(1));
    if activity.animating || activity.since_work.is_some_and(|since| since < SETTLE) {
        return frame;
    }
    match activity.watch_mouse_every {
        Some(every) => every.clamp(frame, IDLE_INTERVAL.max(frame)),
        None => IDLE_INTERVAL.max(frame),
    }
}

/// The event the producers set. Auto-reset: one wait takes it, and a
/// signal given while the loop is busy is kept for its next wait.
fn signal() -> Option<HANDLE> {
    static SIGNAL: OnceLock<Option<isize>> = OnceLock::new();
    SIGNAL
        .get_or_init(|| {
            // SAFETY: an unnamed event with default security; the handle
            // lives as long as the process.
            match unsafe { CreateEventW(None, false, false, None) } {
                Ok(handle) => {
                    let raw = handle.0 as isize;
                    gevrek_host::wake::install(move || {
                        // SAFETY: the handle is never closed.
                        let _ = unsafe { SetEvent(HANDLE(raw as *mut _)) };
                    });
                    Some(raw)
                }
                Err(error) => {
                    log::warn!(
                        "No wake signal for the runtime loop ({error}); it passes once a frame"
                    );
                    None
                }
            }
        })
        .map(|raw| HANDLE(raw as *mut _))
}

/// Make the signal before anything can want to give it.
pub fn prepare() -> bool {
    signal().is_some()
}

/// Wait until there is work, a message for this thread, or `timeout` has
/// passed. Without the signal (it could not be made) the wait is never
/// longer than `frame`.
pub fn for_work(timeout: Duration, frame: Duration) {
    let Some(signal) = signal() else {
        std::thread::sleep(timeout.min(frame));
        return;
    };
    let milliseconds = timeout.as_millis().clamp(1, 60_000) as u32;
    // SAFETY: one valid handle; the call only waits.
    unsafe {
        MsgWaitForMultipleObjectsEx(
            Some(&[signal]),
            milliseconds,
            QS_ALLINPUT,
            MWMO_INPUTAVAILABLE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const FRAME: Duration = Duration::from_millis(16);

    #[test]
    fn an_idle_loop_waits_long_and_a_busy_one_a_frame() {
        assert_eq!(interval(FRAME, Activity::default()), IDLE_INTERVAL);
        let animating = Activity {
            animating: true,
            ..Activity::default()
        };
        assert_eq!(interval(FRAME, animating), FRAME);
        let just_worked = Activity {
            since_work: Some(Duration::from_millis(120)),
            ..Activity::default()
        };
        assert_eq!(interval(FRAME, just_worked), FRAME);
        let settled = Activity {
            since_work: Some(SETTLE),
            ..Activity::default()
        };
        assert_eq!(interval(FRAME, settled), IDLE_INTERVAL);
    }

    #[test]
    fn the_mouse_is_looked_at_as_often_as_focus_follows_mouse_asks() {
        let watching = |every: u64| Activity {
            watch_mouse_every: Some(Duration::from_millis(every)),
            ..Activity::default()
        };
        assert_eq!(interval(FRAME, watching(40)), Duration::from_millis(40));
        // Never more often than a frame, never more rarely than idle.
        assert_eq!(interval(FRAME, watching(0)), FRAME);
        assert_eq!(interval(FRAME, watching(900)), IDLE_INTERVAL);
    }

    #[test]
    fn a_frame_longer_than_the_idle_interval_is_kept() {
        let slow = Duration::from_millis(250);
        assert_eq!(interval(slow, Activity::default()), slow);
        assert_eq!(
            interval(
                Duration::ZERO,
                Activity {
                    animating: true,
                    ..Activity::default()
                }
            ),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn a_wake_ends_the_wait_and_is_kept_for_the_next_one() {
        assert!(prepare());
        // Given before the wait: the wait returns at once.
        gevrek_host::wake::now();
        let started = Instant::now();
        for_work(Duration::from_secs(5), Duration::from_secs(5));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );

        // Given during the wait, from another thread.
        let waker = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            gevrek_host::wake::now();
        });
        let started = Instant::now();
        for_work(Duration::from_secs(5), Duration::from_secs(5));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        waker.join().unwrap();

        // Nothing given: the wait lasts its time. (Another test of this
        // process may give the signal meanwhile, so one of a few will do.)
        let lasted = (0..5).any(|_| {
            let started = Instant::now();
            for_work(Duration::from_millis(60), Duration::from_secs(5));
            started.elapsed() >= Duration::from_millis(40)
        });
        assert!(lasted);
    }
}
