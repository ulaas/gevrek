//! What every backend sets up the same way once it has its outputs: the
//! socket clients connect through, the dispatch of their requests, the
//! start of the WM, Xwayland, and the passes on which the host looks for
//! commands and configuration requests.
//!
//! The host used to pass every 8 ms whatever happened. Now it waits for
//! work, as the Windows loop does: whoever queues something says so
//! (`gevrek_host::wake`, or [`State::host_soon`] from inside the loop) and
//! the pass is made at once. Much of a pass still looks things up instead
//! of being told (the configuration file, the idle times, started
//! programs), so an idle host passes every [`IDLE_INTERVAL`]; while
//! something it has to advance is going on, and for [`SETTLE`] after the
//! last work, it passes every [`HOST_TICK`] as before.

use crate::state::{ClientState, State};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::ping::{make_ping, Ping};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{
    EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, RegistrationToken,
};
use smithay::reexports::wayland_server::Display;
use smithay::wayland::socket::ListeningSocketSource;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often the host passes while there is something to advance.
const HOST_TICK: Duration = Duration::from_millis(8);

/// The longest an idle host waits before it looks around on its own.
pub(crate) const IDLE_INTERVAL: Duration = Duration::from_millis(100);

/// How long after the last work the host keeps passing every
/// [`HOST_TICK`]: what follows work is often seen by looking.
pub(crate) const SETTLE: Duration = Duration::from_millis(500);

/// How long to wait before the next pass.
pub(crate) fn interval(busy: bool, since_work: Option<Duration>) -> Duration {
    if busy || since_work.is_some_and(|since| since < SETTLE) {
        HOST_TICK
    } else {
        IDLE_INTERVAL
    }
}

/// The pace of the host's passes, kept in [`State`].
pub struct HostPace {
    /// Wakes the loop for a pass at once; none before the session is open.
    ping: Option<Ping>,
    /// The timer of the passes, to be put in anew when a pass is wanted now.
    timer: Option<RegistrationToken>,
    /// When the host last had work (a command, a configuration, a key held).
    pub(crate) last_work: Option<Instant>,
    /// Counting the passes of the running second.
    counting_since: Instant,
    counted: u32,
    /// The passes of the last whole second (`gevrek health` shows it).
    pub(crate) passes_per_second: f64,
}

impl Default for HostPace {
    fn default() -> Self {
        Self {
            ping: None,
            timer: None,
            last_work: None,
            counting_since: Instant::now(),
            counted: 0,
            passes_per_second: 0.0,
        }
    }
}

impl HostPace {
    fn count(&mut self, now: Instant) {
        self.counted += 1;
        let over = now.saturating_duration_since(self.counting_since);
        if over >= Duration::from_secs(1) {
            self.passes_per_second = f64::from(self.counted) / over.as_secs_f64();
            self.counted = 0;
            self.counting_since = now;
        }
    }
}

impl State {
    /// Something inside the loop gave the host work that cannot wait for
    /// its next look around (a key that is held and may repeat).
    pub(crate) fn host_soon(&mut self) {
        self.host_pace.last_work = Some(Instant::now());
        if let Some(ping) = &self.host_pace.ping {
            ping.ping();
        }
    }
}

/// The loop that is woken by `gevrek_host::wake`: the one opened last (one
/// to a process, but for tests that open several one after another).
static WOKEN: Mutex<Option<Ping>> = Mutex::new(None);

/// The timer of the host's passes: one at once, the next after what the
/// pass found.
fn pass_timer(
    handle: &LoopHandle<'static, State>,
    signal: LoopSignal,
) -> anyhow::Result<RegistrationToken> {
    handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            let now = Instant::now();
            state.host_pace.count(now);
            if !state.host_tick() {
                signal.stop();
            }
            let since_work = state.host_pace.last_work.map(|last| last.elapsed());
            TimeoutAction::ToDuration(interval(state.host_busy(), since_work))
        })
        .map_err(|error| anyhow::anyhow!("timer source: {error}"))
}

/// Open the session on `event_loop`. `ready` is called once clients can
/// connect, with the name of the Wayland socket. The loop is stopped when
/// the host says the WM is to end.
pub(crate) fn open(
    event_loop: &EventLoop<'static, State>,
    display: Display<State>,
    state: &mut State,
    ready: impl FnOnce(&mut State, &str),
) -> anyhow::Result<()> {
    let handle = event_loop.handle();
    // Sources that come and go with what clients do (a picture that is
    // waited for, Xwayland) are added through this.
    state.loop_handle = Some(handle.clone());
    // Clients connect through a socket in XDG_RUNTIME_DIR.
    let socket = ListeningSocketSource::new_auto()?;
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    handle
        .insert_source(socket, |stream, _, state| {
            let client = ClientState::of(&stream);
            if let Err(error) = state.display.insert_client(stream, Arc::new(client)) {
                log::warn!("client refused: {error}");
            }
        })
        .map_err(|error| anyhow::anyhow!("socket source: {error}"))?;
    handle
        .insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, display, state| {
                // SAFETY: the display is not dropped while the source lives.
                unsafe {
                    display.get_mut().dispatch_clients(state)?;
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|error| anyhow::anyhow!("display source: {error}"))?;

    // Programs can ask to be told when the user is away; that needs timers
    // of this loop.
    state.idle_notifier = Some(smithay::wayland::idle_notify::IdleNotifierState::new(
        &state.display,
        handle.clone(),
    ));
    // Programs started from here on are clients of gevrek, also those the
    // startup of the configuration starts at once.
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);
    state.start()?;
    // X11 programs run through Xwayland unless that is turned off.
    // (`desktop.platform.wayland.xwayland`, or `--no-xwayland`.)
    let wayland = state.config.desktop.platform.wayland.clone();
    if !wayland.cursor_theme.is_empty() {
        std::env::set_var("XCURSOR_THEME", &wayland.cursor_theme);
    }
    if wayland.cursor_size > 0 {
        std::env::set_var("XCURSOR_SIZE", wayland.cursor_size.to_string());
    }
    if wayland.xwayland && std::env::var_os("GEVREK_NO_XWAYLAND").is_none() {
        match state.start_xwayland(handle.clone()) {
            Ok(()) => state.xwayland_wanted = true,
            Err(error) => log::warn!("Xwayland is not available: {error}"),
        }
    }
    log::info!("gevrek is running on Wayland socket {socket_name}");
    ready(state, &socket_name);

    // The host side: commands and configuration from the IPC server. A
    // pass is made when the timer says so, and at once when the loop is
    // woken. (The timer stays where it is: taking it out and putting it
    // in anew left an event of the old one behind now and then, which the
    // loop said it had no source for.) Woken, the host has had work, so
    // the timer's next passes come every `HOST_TICK` for a while.
    let signal = event_loop.get_signal();
    state.host_pace.timer = Some(pass_timer(&handle, signal.clone())?);
    let (ping, woken) = make_ping().map_err(|error| anyhow::anyhow!("wake signal: {error}"))?;
    handle
        .insert_source(woken, move |_, _, state| {
            let now = Instant::now();
            state.host_pace.count(now);
            state.host_pace.last_work = Some(now);
            if !state.host_tick() {
                signal.stop();
            }
        })
        .map_err(|error| anyhow::anyhow!("wake source: {error}"))?;
    *WOKEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ping.clone());
    state.host_pace.ping = Some(ping);
    gevrek_host::wake::install(|| {
        if let Some(ping) = &*WOKEN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            ping.ping();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_host_waits_and_a_busy_one_does_not() {
        assert_eq!(interval(false, None), IDLE_INTERVAL);
        assert_eq!(interval(true, None), HOST_TICK);
        assert_eq!(interval(false, Some(Duration::from_millis(20))), HOST_TICK);
        assert_eq!(interval(false, Some(SETTLE)), IDLE_INTERVAL);
    }

    #[test]
    fn the_passes_of_a_second_are_counted() {
        let mut pace = HostPace::default();
        let start = pace.counting_since;
        for pass in 1..=10 {
            pace.count(start + Duration::from_millis(100 * pass));
        }
        assert!(
            (pace.passes_per_second - 10.0).abs() < 0.01,
            "{}",
            pace.passes_per_second
        );
    }
}
