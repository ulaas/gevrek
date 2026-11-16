//! Locking the session and knowing when the user is away.
//!
//! **Lock** (`ext-session-lock`): a lock screen asks to take the session.
//! From then on nothing of the session is drawn or reachable: each output
//! shows the lock screen's surface for it (or nothing), keys and the
//! pointer go only to those surfaces, bindings rest, and the control
//! endpoint refuses commands. If the lock screen ends without unlocking,
//! the session stays locked: that is the protocol's promise, and why the
//! lock is the compositor's state and not the lock program's.
//!
//! **Idle** (`ext-idle-notify`): programs ask to be told after so many
//! seconds without input (to dim, to lock, to turn displays off). Programs
//! that play a video or hold a call inhibit that (`idle-inhibit`).

use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        log::info!("The session is locked");
        let from_black = self.veiled && self.ends_with_program && !self.locked;
        self.locked = true;
        if from_black {
            // The login's gevrek began in black ([`State::begin_in_black`]):
            // there is no desktop to cover, the login comes out of the
            // black when it has a picture.
            self.arrive_when(false);
        } else {
            self.lock_change = self.lock_change_length().map(|length| LockChange {
                since: std::time::Instant::now(),
                kind: ChangeKind::Lock,
                shown: None,
                length,
            });
        }
        gevrek_host::lock::set(true);
        self.lock_surfaces.clear();
        // Nothing of the session has the keyboard or the pointer any more.
        // A menu that is open, or an X11 program that holds the keyboard,
        // has them by a grab, and a grab keeps what is typed from going
        // anywhere else: with one alive the password typed at the lock
        // screen would go to that program. So the grabs end first.
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(mut grab) = self.popup_grab.take() {
            grab.ungrab(smithay::desktop::PopupUngrabStrategy::All);
        }
        self.x11_key_grab = None;
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(self, serial, 0);
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.unset_grab(self);
            keyboard.set_focus(self, None, serial);
        }
        self.drag = None;
        self.desktop.needs_redraw = true;
        // From the next frame on only lock surfaces are drawn.
        confirmation.lock();
    }

    fn unlock(&mut self) {
        log::info!("The session is unlocked");
        self.locked = false;
        self.lock_change = self.lock_change_length().map(|length| LockChange {
            since: std::time::Instant::now(),
            kind: ChangeKind::Unlock,
            shown: None,
            length,
        });
        gevrek_host::lock::set(false);
        self.lock_surfaces.clear();
        // The window that had the focus gets the keyboard back.
        self.desktop.focus_dirty = true;
        self.desktop.needs_redraw = true;
        self.settle();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(id) = Output::from_resource(&output).and_then(|output| {
            self.outputs
                .iter()
                .find(|(_, known)| *known == output)
                .map(|(id, _)| *id)
        }) else {
            return;
        };
        if let Some(rect) = self.desktop.output_rectangle(id) {
            surface.with_pending_state(|state| {
                state.size = Some((rect.width.max(1) as u32, rect.height.max(1) as u32).into());
            });
            surface.send_configure();
        }
        self.lock_surfaces.retain(|(other, _)| *other != id);
        self.lock_surfaces.push((id, surface));
        self.focus_lock_surface();
        self.desktop.needs_redraw = true;
    }
}

/// The change between the desktop and the lock screen, while it is
/// drawn: never a cut. Locking, the desktop goes dark from its edges and
/// dissolves into its own wallpaper blurred, and the lock screen comes
/// in over that; unlocking, the desktop comes sharp again out of the
/// blurred wallpaper and the dark leaves by the edges last. (The keys
/// and the pointer are the lock screen's from the first moment: only
/// the picture takes its time.)
pub(crate) struct LockChange {
    pub since: std::time::Instant,
    pub kind: ChangeKind,
    /// Since when the lock screen has a picture to show.
    pub shown: Option<std::time::Instant>,
    pub length: std::time::Duration,
}

/// Which change is drawn. Besides the lock's two there are the two of
/// a whole gevrek coming and going on the display — the desktop after
/// the login, the login after the desktop — which use the same veil:
/// leaving, everything goes under it and stays there (the display keeps
/// that picture while another gevrek takes over, or this one ends);
/// arriving, everything comes out from under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Lock,
    Unlock,
    /// `quit`: gevrek ends when it is under the veil.
    Leave {
        quit: bool,
    },
    Arrive,
}

/// How a moment of that change looks, each from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LockLook {
    /// The blurred wallpaper and the dark, over what is under them.
    pub veil: f32,
    /// The dark at the edges.
    pub vignette: f32,
    /// What is under the veil: the lock screen (true) or the desktop.
    pub covered: bool,
    /// Black over all of it: where a gevrek leaves or arrives. The
    /// display is black for a moment while the consoles are changed, so
    /// that is what one gevrek ends in and the other begins with.
    pub black: f32,
}

fn eased(part: f32) -> f32 {
    let part = part.clamp(0.0, 1.0);
    part * part * (3.0 - 2.0 * part)
}

/// The look of a moment: `part` of the change's length has passed
/// (more than 1 when the lock screen was late), and `lock_part` of the
/// lock screen's own coming out from under the veil.
///
/// Locking has two halves, and the two pictures are never mixed: the
/// desktop goes under the veil, and then the lock screen comes out
/// from under it (a lock screen that is late is waited for under the
/// whole veil).
pub fn lock_look(locking: bool, part: f32, lock_part: f32) -> LockLook {
    if !locking {
        return LockLook {
            // The picture first, the edges last.
            veil: 1.0 - eased(part / 0.75),
            vignette: 1.0 - eased(part),
            covered: false,
            black: 0.0,
        };
    }
    if part < 1.0 {
        LockLook {
            // The edges first, the whole a little after.
            vignette: eased(part / 0.6),
            veil: eased(part),
            covered: false,
            black: 0.0,
        }
    } else {
        let out = eased(lock_part);
        LockLook {
            // The picture first, the edges last.
            veil: 1.0 - eased(lock_part / 0.75),
            vignette: 1.0 - out,
            covered: true,
            black: 0.0,
        }
    }
}

/// The look of a moment of leaving (everything goes under the veil) or
/// arriving (it comes out); `covered`: it is the lock screen that is
/// under the veil (the login's gevrek), not a desktop.
pub fn passage_look(leaving: bool, part: f32, covered: bool) -> LockLook {
    if leaving {
        LockLook {
            vignette: eased(part / 0.5),
            veil: eased(part / 0.7),
            // The last of it: into black.
            black: eased((part - 0.45) / 0.55),
            covered,
        }
    } else {
        LockLook {
            // Out of black first, then out from under the veil.
            black: 1.0 - eased(part / 0.45),
            veil: 1.0 - eased((part - 0.2) / 0.6),
            vignette: 1.0 - eased((part - 0.2) / 0.8),
            covered,
        }
    }
}

/// Set by the signal that asks a gevrek to go under its veil (the
/// display manager sends it to the login's gevrek before it shows the
/// desktop's console).
pub static LEAVE_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn leave_asked(_: libc::c_int) {
    LEAVE_ASKED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Set by the signal that asks for the next keyboard layout (the login
/// sends it to the gevrek it runs in when its layout button is pressed:
/// it has no other line to it).
pub static NEXT_LAYOUT_ASKED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

extern "C" fn next_layout_asked(_: libc::c_int) {
    NEXT_LAYOUT_ASKED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Listen for those signals (SIGUSR1: leave; SIGUSR2: the next layout).
pub fn listen_for_leave() {
    // SAFETY: the handlers only set a flag.
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            leave_asked as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGUSR2,
            next_layout_asked as *const () as libc::sighandler_t,
        );
    }
}

/// A desktop whose bar is not there after this long comes out without it.
const SHELL_LATE: std::time::Duration = std::time::Duration::from_millis(2500);

/// A lock screen that shows nothing for this long is not waited for.
const LOCK_SCREEN_LATE: std::time::Duration = std::time::Duration::from_secs(3);

impl State {
    /// How long the change between desktop and lock screen takes: none
    /// with the animations (or these changes) switched off, else what
    /// `desktop.animations.session` says, within reason.
    fn lock_change_length(&self) -> Option<std::time::Duration> {
        let animations = &self.config.desktop.animations;
        (animations.enabled && animations.session.enabled).then(|| {
            std::time::Duration::from_millis(
                u64::from(animations.session.duration_ms).clamp(100, 3000),
            )
        })
    }

    /// The lock screen's part of its coming out from under the veil:
    /// from when it has a picture, and not before the desktop is covered.
    fn lock_part(change: &LockChange) -> f32 {
        change.shown.map_or(0.0, |shown| {
            let from = shown.max(change.since + change.length);
            std::time::Instant::now()
                .saturating_duration_since(from)
                .as_secs_f32()
                / (change.length.as_secs_f32() * 0.8).max(0.001)
        })
    }

    /// How the change looks now, while there is one (and afterwards,
    /// for a gevrek that has left: it stays under its veil).
    pub(crate) fn lock_look(&self) -> Option<LockLook> {
        let Some(change) = self.lock_change.as_ref() else {
            return self.veiled.then(|| passage_look(true, 1.0, self.locked));
        };
        let part = change.since.elapsed().as_secs_f32() / change.length.as_secs_f32().max(0.001);
        Some(match change.kind {
            ChangeKind::Lock => lock_look(true, part, Self::lock_part(change)),
            ChangeKind::Unlock => lock_look(false, part, 0.0),
            ChangeKind::Leave { .. } => passage_look(true, part, self.locked),
            ChangeKind::Arrive => passage_look(
                false,
                change.shown.map_or(0.0, |shown| {
                    shown.elapsed().as_secs_f32() / change.length.as_secs_f32().max(0.001)
                }),
                self.locked,
            ),
        })
    }

    /// Everything goes under the veil and stays there. False when
    /// nothing is drawn of it (the animations are off).
    pub(crate) fn leave(&mut self, quit: bool) -> bool {
        let Some(length) = self.lock_change_length() else {
            return false;
        };
        // (Into black and out of it is a longer way than under a veil.)
        let length = length.mul_f32(1.4);
        self.lock_change = Some(LockChange {
            since: std::time::Instant::now(),
            kind: ChangeKind::Leave { quit },
            shown: None,
            length,
        });
        self.desktop.needs_redraw = true;
        true
    }

    /// Everything comes out from under the veil: a session's first
    /// frames, and a gevrek that left and is shown again.
    pub fn arrive(&mut self) {
        self.arrive_when(true);
    }

    /// As [`State::arrive`]; not `ready`, it stays under the veil until
    /// the desktop has its bar (or the bar is late): a desktop does not
    /// come out empty and get its bar a moment after.
    pub fn arrive_when(&mut self, ready: bool) {
        self.veiled = false;
        let now = std::time::Instant::now();
        self.lock_change = self.lock_change_length().map(|length| LockChange {
            since: now,
            length: length.mul_f32(1.4),
            kind: ChangeKind::Arrive,
            shown: ready.then_some(now),
        });
        self.desktop.needs_redraw = true;
    }

    /// The login's gevrek begins in black and stays there until its
    /// login has a picture: what was on the display before it (a boot
    /// splash that ended in black, or nothing) goes over into the login
    /// without gevrek's empty ground being seen in between. (Not where
    /// these changes are switched off: there it begins as it is.)
    pub fn begin_in_black(&mut self) {
        if self.lock_change_length().is_some() {
            self.veiled = true;
            self.desktop.needs_redraw = true;
        }
    }

    /// The lock screen has a picture on a display.
    fn lock_screen_is_shown(&self) -> bool {
        self.lock_surfaces.iter().any(|(_, surface)| {
            smithay::backend::renderer::utils::with_renderer_surface_state(
                surface.wl_surface(),
                |state| state.buffer().is_some(),
            )
            .unwrap_or(false)
        })
    }

    /// Some program of the shell has a surface with a picture on a
    /// display (the bar, a dock, a wallpaper program).
    fn shell_is_shown(&self) -> bool {
        self.outputs.iter().any(|(_, output)| {
            smithay::desktop::layer_map_for_output(output)
                .layers()
                .any(|layer| {
                    smithay::backend::renderer::utils::with_renderer_surface_state(
                        layer.wl_surface(),
                        |state| state.buffer().is_some(),
                    )
                    .unwrap_or(false)
                })
        })
    }

    /// Asked to stop: a session goes under its veil first, so that the
    /// desktop is not cut off. True while that is being drawn.
    pub(crate) fn leaving_before_it_stops(&mut self) -> bool {
        if !self.session_mode || self.veiled {
            return false;
        }
        match &self.lock_change {
            Some(LockChange {
                kind: ChangeKind::Leave { .. },
                ..
            }) => true,
            _ => self.leave(true),
        }
    }

    /// Move the change on: every frame is drawn while it runs, the lock
    /// screen's coming out starts with its first picture, and the change
    /// is over when everything has arrived.
    pub(crate) fn tick_lock_change(&mut self) {
        if LEAVE_ASKED.swap(false, std::sync::atomic::Ordering::SeqCst) && !self.veiled {
            self.leave(false);
        }
        if NEXT_LAYOUT_ASKED.swap(false, std::sync::atomic::Ordering::SeqCst) {
            if let Err(error) =
                self.switch_keyboard_layout(gevrek_model::command::LayoutTarget::Next)
            {
                log::debug!("the next keyboard layout: {error}");
            }
        }
        let Some(change) = &self.lock_change else {
            return;
        };
        self.desktop.needs_redraw = true;
        let part = change.since.elapsed().as_secs_f32() / change.length.as_secs_f32().max(0.001);
        match change.kind {
            ChangeKind::Unlock => {
                if part >= 1.0 {
                    self.lock_change = None;
                }
            }
            ChangeKind::Arrive => match change.shown {
                Some(shown) => {
                    if shown.elapsed() >= change.length {
                        self.lock_change = None;
                    }
                }
                None => {
                    // (What is waited for: the login's picture, or the
                    // desktop's bar.)
                    let there = if self.locked {
                        self.lock_screen_is_shown()
                    } else {
                        self.shell_is_shown()
                    };
                    if there || change.since.elapsed() > SHELL_LATE {
                        if let Some(change) = &mut self.lock_change {
                            change.shown = Some(std::time::Instant::now());
                        }
                    }
                }
            },
            ChangeKind::Leave { .. } => {
                if part >= 1.0 {
                    self.lock_change = None;
                    self.veiled = true;
                }
            }
            ChangeKind::Lock => {
                if change.shown.is_none() {
                    if self.lock_screen_is_shown() {
                        if let Some(change) = &mut self.lock_change {
                            change.shown = Some(std::time::Instant::now());
                        }
                    } else if change.since.elapsed() > LOCK_SCREEN_LATE {
                        self.lock_change = None;
                    }
                } else if part >= 1.0 && Self::lock_part(change) >= 1.0 {
                    self.lock_change = None;
                }
            }
        }
    }

    /// The lock surface of an output, if the lock screen made one.
    pub(crate) fn lock_surface(&self, output: MonitorHandle) -> Option<&LockSurface> {
        self.lock_surfaces
            .iter()
            .find(|(id, _)| *id == output)
            .map(|(_, surface)| surface)
            .filter(|surface| surface.alive())
    }

    /// While locked the keyboard is the lock screen's: the surface of the
    /// output the pointer is on, else any.
    pub(crate) fn focus_lock_surface(&mut self) {
        let (x, y) = self.desktop.pointer;
        let under = self.desktop.outputs.iter().find(|output| {
            let rect = output.rect;
            x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
        });
        let surface: Option<WlSurface> = under
            .and_then(|output| self.lock_surface(output.id))
            .or_else(|| {
                self.lock_surfaces
                    .iter()
                    .map(|(_, surface)| surface)
                    .find(|s| s.alive())
            })
            .map(|surface| surface.wl_surface().clone());
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(
                self,
                surface.map(crate::surface::KeyTarget::Wl),
                smithay::utils::SERIAL_COUNTER.next_serial(),
            );
        }
    }

    /// There was input: programs waiting for the user to be away start
    /// counting again.
    pub(crate) fn activity(&mut self) {
        self.last_activity = std::time::Instant::now();
        self.idled_off = false;
        // The warning that the screen is about to lock is answered.
        self.set_lock_warning(0);
        // Any input switches the displays on again; but not the letting go
        // of the very key that switched them off.
        if self
            .displays_off
            .is_some_and(|since| since.elapsed() >= std::time::Duration::from_millis(700))
        {
            log::info!("Input: the displays are switched on");
            self.displays_off = None;
            self.desktop.needs_redraw = true;
        }
        let seat = self.seat.clone();
        if let Some(notifier) = &mut self.idle_notifier {
            notifier.notify_activity(&seat);
        }
    }

    /// How dark the picture is as a warning before the lock, in
    /// twentieths; drawn anew when it changed.
    fn set_lock_warning(&mut self, twentieths: u8) {
        let twentieths = twentieths.min(20);
        if self.lock_warning != twentieths {
            self.lock_warning = twentieths;
            self.lock_warning_commit.increment();
            self.desktop.needs_redraw = true;
        }
    }

    /// Switch the displays off: they show nothing and save power until
    /// the next key or movement.
    pub fn switch_displays_off(&mut self) {
        if self.displays_off.is_none() {
            log::info!("The displays are switched off");
            self.displays_off = Some(std::time::Instant::now());
            self.desktop.needs_redraw = true;
        }
    }

    /// Switch the displays off when nobody has used the session for as
    /// long as `desktop.session.displays_off_after_minutes` says, unless
    /// a program keeps the screen awake.
    pub(crate) fn tick_idle_displays(&mut self) {
        if self.displays_off.is_some()
            || self.idled_off
            || !self.idle_inhibitors.is_empty()
            || self.kept_awake_by_fullscreen()
        {
            return;
        }
        let after = std::env::var("GEVREK_IDLE_DISPLAYS_OFF_SECONDS")
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| *seconds > 0.0)
            .map(std::time::Duration::from_secs_f64)
            .or_else(|| {
                let minutes = self.config.desktop.session.displays_off_after_minutes;
                (minutes > 0).then(|| std::time::Duration::from_secs(u64::from(minutes) * 60))
            });
        if after.is_some_and(|after| self.last_activity.elapsed() >= after) {
            self.idled_off = true;
            self.switch_displays_off();
        }
    }

    /// Lock the screen when nobody has used the session for as long as
    /// `desktop.session.lock_after_minutes` says, unless a program keeps
    /// it awake. Called with every tick of the host.
    pub(crate) fn tick_idle_lock(&mut self) {
        if self.locked || !self.idle_inhibitors.is_empty() {
            return;
        }
        if self.kept_awake_by_fullscreen() {
            self.set_lock_warning(0);
            return;
        }
        // (The checks do not wait minutes: they say seconds.)
        let after = std::env::var("GEVREK_IDLE_LOCK_SECONDS")
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| *seconds > 0.0)
            .map(std::time::Duration::from_secs_f64)
            .or_else(|| {
                let minutes = self.config.desktop.session.lock_after_minutes;
                (minutes > 0).then(|| std::time::Duration::from_secs(u64::from(minutes) * 60))
            });
        let Some(after) = after else {
            self.set_lock_warning(0);
            return;
        };
        // In the seconds before, the picture grows dark as a warning.
        let dark = self
            .config
            .desktop
            .session
            .dimmed(self.last_activity.elapsed(), after);
        self.set_lock_warning((dark * 20.0).round() as u8);
        if self.last_activity.elapsed() >= after {
            log::info!(
                "Nobody has used the session for {:.0} seconds: locking",
                after.as_secs_f64()
            );
            // Counted anew, so that a locker that takes a moment to come
            // up (or is not installed) is not started with every tick.
            self.last_activity = std::time::Instant::now();
            self.start_locker();
        }
    }

    /// A window that fills a whole display is shown, and the
    /// configuration lets that keep the session awake
    /// (`desktop.session.awake_while_fullscreen`).
    pub(crate) fn kept_awake_by_fullscreen(&self) -> bool {
        self.config.desktop.session.awake_while_fullscreen
            && self
                .desktop
                .windows
                .iter()
                .any(|(id, window)| window.fullscreen && self.desktop.shown(*id))
    }

    /// A program began or ended holding the session awake.
    pub(crate) fn sync_idle_inhibit(&mut self) {
        use smithay::utils::IsAlive;
        self.idle_inhibitors.retain(|surface| surface.alive());
        let inhibited = !self.idle_inhibitors.is_empty();
        if let Some(notifier) = &mut self.idle_notifier {
            notifier.set_is_inhibited(inhibited);
        }
    }

    /// Lock the screen: start the program that does it. The one the
    /// configuration names, else gevrek's own when it is installed, else
    /// the first lock program found. Until gevrek has its own this is how
    /// the `lock` action locks.
    pub(crate) fn start_locker(&mut self) {
        if self.locked {
            return;
        }
        let beside = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|folder| folder.join("gevrek-lock")))
            .filter(|path| path.exists())
            .map(|path| path.to_string_lossy().into_owned());
        let on_path = |name: &str| {
            std::env::var_os("PATH").is_some_and(|paths| {
                std::env::split_paths(&paths).any(|folder| folder.join(name).is_file())
            })
        };
        let program = beside.or_else(|| {
            ["swaylock", "hyprlock", "gtklock", "waylock"]
                .into_iter()
                .find(|name| on_path(name))
                .map(str::to_string)
        });
        let Some(program) = program else {
            log::warn!(
                "No lock screen is installed (gevrek-lock, swaylock, hyprlock, gtklock or waylock): the screen was not locked"
            );
            return;
        };
        // gevrek's own lock screen is told the wallpaper of this moment
        // (the common one; with a slideshow the picture that is shown), so
        // that it shows what the running configuration shows and not what
        // a file says. An empty name: none.
        let wallpaper = &self.config.desktop.wallpaper;
        let picture = if self.config.desktop.session.lock_shows_wallpaper {
            self.wallpapers
                .borrow_mut()
                .slide(&wallpaper.slideshow)
                .or_else(|| wallpaper.of("").0)
        } else {
            None
        };
        let mut environment = std::collections::BTreeMap::new();
        environment.insert(
            "GEVREK_LOCK_WALLPAPER".to_string(),
            picture.map_or_else(String::new, |path| path.to_string_lossy().into_owned()),
        );
        // And the accent the desktop has now (the wallpaper's colour
        // where the theme takes it from there).
        let (red, green, blue) = self
            .core
            .wallpaper_accent
            .unwrap_or(self.config.desktop.appearance.active_border_color);
        environment.insert(
            "GEVREK_LOCK_ACCENT".to_string(),
            format!("{red:02x}{green:02x}{blue:02x}"),
        );
        let launch = gevrek_model::settings::LaunchProfile {
            program,
            environment,
            ..Default::default()
        };
        match crate::launch::spawn(&launch) {
            Ok(pid) => log::info!("Lock screen: {} (pid {pid})", launch.program),
            Err(error) => log::error!("The lock screen did not start: {error}"),
        }
    }
}

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        self.idle_notifier
            .as_mut()
            .expect("the idle notifier is made before any client can ask for it")
    }
}

smithay::delegate_session_lock!(State);
smithay::delegate_idle_notify!(State);

#[cfg(test)]
mod change_tests {
    use super::*;

    #[test]
    fn locking_covers_the_desktop_and_then_the_lock_screen_comes_out() {
        let start = lock_look(true, 0.0, 0.0);
        assert_eq!(
            (start.veil, start.vignette, start.covered),
            (0.0, 0.0, false)
        );
        assert_eq!(start.black, 0.0, "a lock is never through black");
        let early = lock_look(true, 0.3, 0.0);
        assert!(early.vignette > early.veil && early.veil > 0.0, "{early:?}");
        assert!(
            !early.covered,
            "the desktop is under the veil until it is covered"
        );
        // The lock screen was late: everything is covered and waits.
        let waiting = lock_look(true, 2.5, 0.0);
        assert_eq!(
            (waiting.veil, waiting.vignette, waiting.covered),
            (1.0, 1.0, true)
        );
        let coming = lock_look(true, 2.7, 0.5);
        assert!(
            coming.covered && coming.veil > 0.0 && coming.veil < 1.0,
            "{coming:?}"
        );
        assert!(coming.vignette > coming.veil, "the edges clear last");
        let done = lock_look(true, 3.0, 1.2);
        assert_eq!((done.veil, done.vignette, done.covered), (0.0, 0.0, true));
        // Nothing of the desktop from the moment it is covered.
        for step in 0..=40 {
            let part = step as f32 / 20.0;
            assert_eq!(lock_look(true, part, 0.0).covered, part >= 1.0);
        }
    }

    #[test]
    fn leaving_ends_in_black_and_arriving_begins_with_it() {
        let start = passage_look(true, 0.0, false);
        assert_eq!((start.veil, start.vignette, start.black), (0.0, 0.0, 0.0));
        let half = passage_look(true, 0.4, false);
        assert!(
            half.veil > 0.0 && half.black == 0.0,
            "the veil first: {half:?}"
        );
        let left = passage_look(true, 1.0, false);
        assert_eq!((left.veil, left.vignette, left.black), (1.0, 1.0, 1.0));
        // (And stays there however long it takes for the other to come.)
        assert_eq!(passage_look(true, 7.0, true).black, 1.0);
        let arriving = passage_look(false, 0.0, false);
        assert_eq!((arriving.veil, arriving.black), (1.0, 1.0));
        let out = passage_look(false, 0.5, false);
        assert!(
            out.black == 0.0 && out.veil > 0.0,
            "out of black, still under the veil: {out:?}"
        );
        let there = passage_look(false, 1.0, false);
        assert_eq!((there.veil, there.vignette, there.black), (0.0, 0.0, 0.0));
        // Never darker again on the way in, never lighter on the way out.
        let dark = |look: LockLook| look.black * 2.0 + look.veil;
        for step in 0..20 {
            let (one, next) = (step as f32 / 20.0, (step + 1) as f32 / 20.0);
            assert!(
                dark(passage_look(false, next, false)) <= dark(passage_look(false, one, false))
            );
            assert!(dark(passage_look(true, next, false)) >= dark(passage_look(true, one, false)));
        }
    }

    #[test]
    fn unlocking_comes_sharp_first_and_the_edges_clear_last() {
        let start = lock_look(false, 0.0, 0.0);
        assert_eq!((start.veil, start.vignette), (1.0, 1.0));
        let late = lock_look(false, 0.8, 0.0);
        assert_eq!(late.veil, 0.0);
        assert!(late.vignette > 0.0 && late.vignette < 1.0, "{late:?}");
        let done = lock_look(false, 1.0, 0.0);
        assert_eq!((done.veil, done.vignette, done.covered), (0.0, 0.0, false));
        // Each moment is no more covered than the one before.
        let mut before = 1.0;
        for step in 0..=20 {
            let look = lock_look(false, step as f32 / 20.0, 0.0);
            assert!(look.veil <= before);
            before = look.veil;
        }
    }
}
