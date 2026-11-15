//! gevrek on real hardware: outputs through DRM/KMS, devices through
//! libinput, and a seat session (libseat) that owns the virtual terminal.
//!
//! It first ran in a KVM virtual machine with a virtual GPU, keyboard and
//! mouse (`tests/wayland/vm/run.sh`), where output, drawing, keyboard,
//! pointer, leaving and coming back to the virtual terminal and Xwayland
//! were checked. **It has not run on a physical machine**, and what was
//! added since that run is compiled and not yet run anywhere: frames paced
//! by each display's vertical blank, modes and switched-off displays from
//! the configuration, device settings through libinput, keyboard LEDs,
//! the displays looked at again after a virtual terminal switch.
//!
//! For NVIDIA's driver, which this has not met yet, it does what niri does
//! on the same machine: no overlay planes (`FRAME_FLAGS`), and a client's
//! picture is waited for before it is shown (`CompositorHandler::
//! new_surface` in `state.rs`).
//!
//! What it does: one GPU; displays plugged in and out while it runs;
//! graphics-card buffers from clients (`linux-dmabuf`). What it does not:
//! more than one GPU, explicit synchronisation (`linux-drm-syncobj`),
//! variable refresh, turning displays off, touch and tablets as such.
//! Touchpad swipes are passed on (`gestures.rs` decides what they do, and
//! that part is tested); no touchpad has been tried.
//!
//! `State`, `render.rs` and `input.rs` are the same as for the nested
//! backend (`winit.rs`); this file only replaces where frames go and where
//! input comes from.

use crate::render::BACKGROUND;
use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::Rect;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::compositor::{DrmCompositor, FrameFlags};
use smithay::backend::drm::exporter::gbm::GbmFramebufferExporter;
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent};
use smithay::output::OutputModeSource;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::drm::control::{connector, crtc, Device as ControlDevice, ModeTypeFlags};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use smithay::utils::DeviceFd;
use std::cell::RefCell;
use std::rc::Rc;

/// (What is kept with a frame on its way: whom to tell when it is shown.)
type Compositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    Option<smithay::desktop::utils::OutputPresentationFeedback>,
    DrmDeviceFd,
>;

/// One connected display.
struct Surface {
    output: MonitorHandle,
    connector: connector::Handle,
    crtc: crtc::Handle,
    compositor: Compositor,
    /// The mode the display is in.
    mode: smithay::reexports::drm::control::Mode,
    /// How the picture is turned on it.
    transform: gevrek_model::outputs::OutputTransform,
    /// A frame is on its way to the screen: wait for its vblank.
    pending: bool,
    /// The display is switched off (it shows nothing until drawn to again).
    off: bool,
}

/// What the callbacks of the event loop share.
struct Hardware {
    session: LibSeatSession,
    drm: DrmDevice,
    renderer: GlesRenderer,
    gbm: GbmDevice<DrmDeviceFd>,
    allocator: GbmAllocator<DrmDeviceFd>,
    exporter: GbmFramebufferExporter<DrmDeviceFd>,
    render_formats: Vec<smithay::backend::allocator::Format>,
    surfaces: Vec<Surface>,
    libinput: Libinput,
    /// The session is ours (we are on the foreground virtual terminal).
    active: bool,
    /// When the outputs were last looked at for drawing.
    last_draw: std::time::Instant,
    /// The colour temperature the displays were last given (`None`: left
    /// as they are), and for how many displays: a display that comes, or
    /// a change of the hour or the configuration, makes it be given anew.
    night: Option<(Option<u32>, usize)>,
    /// The colour the displays have now, in kelvin (the displays' own:
    /// 6500), and the change that is under way: from, to, since when.
    night_now: u32,
    night_fade: Option<(u32, u32, std::time::Instant)>,
    /// The image outputs are drawn into for programs that ask for pictures.
    capture: crate::capture::CaptureTarget<smithay::backend::renderer::gles::GlesTexture>,
}

/// How often gevrek looks whether something is to be drawn on an output
/// that has no frame on its way to the screen. A display that is being
/// drawn to is paced by its own vertical blank (`DrmEvent::VBlank`), at its
/// own refresh rate; this only starts the first frame after a pause.
const FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(4);

/// How long the picture takes to turn warm when the night light comes on,
/// and to turn back.
const NIGHT_LIGHT_FADE: std::time::Duration = std::time::Duration::from_millis(1800);

/// With nothing on its way to a display, programs are told of a frame no
/// more often than this (the pace of a fast display).
const TOLD_APART: std::time::Duration = std::time::Duration::from_millis(6);

/// With a frame on its way, programs are told at its vertical blank; when
/// none has come for this long they are told all the same (a vertical
/// blank that never comes must not leave every program waiting).
const TOLD_WITHOUT_A_BLANK: std::time::Duration = std::time::Duration::from_millis(100);

/// How often it looks when nothing moves and nothing asked for a frame:
/// a frame that is asked for is drawn after the event that asked (see the
/// loop in [`run`]), so this only has to bring the safety redraw.
const IDLE_FRAME_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// What a frame may use of the display device: a window that covers the
/// display goes to it as it is, and the pointer has its own plane. Overlay
/// planes are left alone: NVIDIA's driver reports them in an order that
/// puts windows above or below where they belong, and elsewhere they have
/// cost more than they saved (niri and Smithay's own compositor leave them
/// for the same reasons).
const FRAME_FLAGS: FrameFlags =
    FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY.union(FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT);

/// Everything is drawn again at least this often even when nothing asked
/// for it, so that a change that forgot to ask shows up.
const SAFETY_REDRAW: std::time::Duration = std::time::Duration::from_millis(500);

/// Run the compositor on the hardware of this seat until it is asked to
/// stop. `build` makes the state for the display; `ready` is called once
/// clients can connect, with the name of the Wayland socket.
pub fn run(
    build: impl FnOnce(DisplayHandle) -> State,
    ready: impl FnOnce(&mut State, &str),
) -> anyhow::Result<()> {
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    let display: Display<State> = Display::new()?;
    let mut state = build(display.handle());
    // Nothing else draws the pointer here.
    state.draw_own_cursor = true;
    state.vt_switching = true;
    state.live_outputs = true;
    // libinput reports touchpad swipes.
    state.gestures_available = true;

    // The seat: it lends us the display and input devices while we are on
    // the foreground virtual terminal.
    let (mut session, session_notifier) = LibSeatSession::new().map_err(|error| {
        anyhow::anyhow!("no seat session (is seatd or logind running?): {error}")
    })?;
    let seat_name = session.seat();

    // The display device.
    let gpu = primary_gpu(&seat_name)
        .ok()
        .flatten()
        .or_else(|| {
            all_gpus(&seat_name)
                .ok()
                .and_then(|gpus| gpus.into_iter().next())
        })
        .ok_or_else(|| anyhow::anyhow!("no display device on seat {seat_name}"))?;
    log::info!("Using display device {}", gpu.display());
    let fd = session
        .open(
            &gpu,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|error| anyhow::anyhow!("could not open {}: {error}", gpu.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    // Started from a bare console, the display is taken over clean:
    // everything on it is switched off and set up again. Started by
    // gevrek's display manager (`GEVREK_KEEP_DISPLAY`), what the display
    // shows is left as it is until the first frame: the login's last
    // picture stays, and the display is not switched off in between.
    let keep = std::env::var_os("GEVREK_KEEP_DISPLAY").is_some_and(|value| value == "1");
    let (drm, drm_notifier) =
        DrmDevice::new(fd.clone(), !keep).map_err(|error| anyhow::anyhow!("DRM: {error}"))?;
    // Explicit sync, where the driver can wake us when a picture is ready.
    // (GEVREK_NO_EXPLICIT_SYNC=1 leaves it out, should a driver do it badly.)
    if std::env::var_os("GEVREK_NO_EXPLICIT_SYNC").is_none()
        && smithay::wayland::drm_syncobj::supports_syncobj_eventfd(&fd)
    {
        state.syncobj_state = Some(
            smithay::wayland::drm_syncobj::DrmSyncobjState::new::<State>(
                &state.display,
                fd.clone(),
            ),
        );
        log::info!("Explicit sync is offered (linux-drm-syncobj)");
    } else {
        log::info!("Explicit sync is not offered");
    }
    let gbm = GbmDevice::new(fd.clone()).map_err(|error| anyhow::anyhow!("GBM: {error}"))?;
    // SAFETY: the GBM device outlives the EGL display made from it.
    let egl = unsafe { EGLDisplay::new(gbm.clone()) }
        .map_err(|error| anyhow::anyhow!("EGL display: {error}"))?;
    let context = EGLContext::new(&egl).map_err(|error| anyhow::anyhow!("EGL context: {error}"))?;
    // SAFETY: the context is used on this thread only.
    let renderer = unsafe { GlesRenderer::new(context) }
        .map_err(|error| anyhow::anyhow!("renderer: {error:?}"))?;
    let render_formats: Vec<_> = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect();
    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    // A program's buffer that was made on this card may go to the
    // display as it is (a game that covers it): the card is named here,
    // without which no such buffer is ever handed over.
    let exporter = GbmFramebufferExporter::new(
        gbm.clone(),
        crate::dmabuf::device_of(&renderer, Some(gpu.as_path())),
    );

    // Keyboards, mice and touchpads of the seat.
    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|()| anyhow::anyhow!("libinput could not take seat {seat_name}"))?;

    let hardware = Rc::new(RefCell::new(Hardware {
        session,
        drm,
        renderer,
        gbm,
        allocator,
        exporter,
        render_formats,
        surfaces: Vec::new(),
        libinput: libinput.clone(),
        active: true,
        last_draw: std::time::Instant::now(),
        night: None,
        night_now: gevrek_model::night_light::NEUTRAL,
        night_fade: None,
        capture: Default::default(),
    }));
    // Clients may draw with the graphics card; each buffer they make is
    // tried in the renderer first.
    let import_hardware = Rc::clone(&hardware);
    crate::dmabuf::offer(
        &mut state,
        &mut hardware.borrow_mut().renderer,
        Box::new(move |dmabuf| {
            use smithay::backend::renderer::ImportDma;
            match import_hardware.try_borrow_mut() {
                Ok(mut hardware) => hardware.renderer.import_dmabuf(dmabuf, None).is_ok(),
                Err(_) => true,
            }
        }),
        Some(gpu.as_path()),
    );
    // One output per connected display.
    scan_outputs(&mut hardware.borrow_mut(), &mut state);
    if hardware.borrow().surfaces.is_empty() {
        anyhow::bail!("no connected display");
    }

    // Displays plugged in or out while gevrek runs.
    let hotplug_hardware = Rc::clone(&hardware);
    match UdevBackend::new(&seat_name) {
        Ok(udev) => {
            handle
                .insert_source(udev, move |event, _, state| {
                    if let UdevEvent::Changed { .. } = event {
                        log::info!(
                            "The display device reports a change: looking at its connectors"
                        );
                        let mut hardware = hotplug_hardware.borrow_mut();
                        if hardware.active {
                            scan_outputs(&mut hardware, state);
                        }
                    }
                })
                .map_err(|error| anyhow::anyhow!("udev source: {error}"))?;
        }
        Err(error) => log::warn!("No display hotplug: {error}"),
    }

    // Input. Ctrl+Alt+F<n> asks for another virtual terminal (see
    // `State::vt_request`, set by the key handling).
    let input_hardware = Rc::clone(&hardware);
    handle
        .insert_source(
            LibinputInputBackend::new(libinput),
            move |event, _, state| {
                // Devices are set up as the configuration says when they appear.
                match &event {
                    smithay::backend::input::InputEvent::DeviceAdded { device } => {
                        state.device_added(device.clone());
                    }
                    smithay::backend::input::InputEvent::DeviceRemoved { device } => {
                        state.device_removed(device);
                    }
                    _ => {}
                }
                state.process_input_event(event);
                if let Some(vt) = state.vt_request.take() {
                    if let Err(error) = input_hardware.borrow_mut().session.change_vt(vt) {
                        log::warn!("Could not switch to virtual terminal {vt}: {error}");
                    }
                }
            },
        )
        .map_err(|error| anyhow::anyhow!("input source: {error}"))?;

    // Leaving and coming back to our virtual terminal.
    let session_hardware = Rc::clone(&hardware);
    handle
        .insert_source(session_notifier, move |event, _, state| {
            let mut hardware = session_hardware.borrow_mut();
            match event {
                SessionEvent::PauseSession => {
                    log::info!("The virtual terminal was taken away: pausing");
                    hardware.active = false;
                    hardware.libinput.suspend();
                    hardware.drm.pause();
                }
                SessionEvent::ActivateSession => {
                    log::info!("The virtual terminal is ours again: resuming");
                    // A gevrek that went under its veil before another
                    // took the display comes out from under it.
                    if state.veiled {
                        state.arrive();
                    }
                    if hardware.libinput.resume().is_err() {
                        log::warn!("libinput did not resume");
                    }
                    if let Err(error) = hardware.drm.activate(false) {
                        log::warn!("DRM did not come back: {error}");
                    }
                    for surface in &mut hardware.surfaces {
                        if let Err(error) = surface.compositor.reset_state() {
                            log::warn!("output state: {error}");
                        }
                        surface.pending = false;
                    }
                    hardware.active = true;
                    state.desktop.needs_redraw = true;
                    // A display may have been plugged in or pulled while
                    // another virtual terminal had the device.
                    scan_outputs(&mut hardware, state);
                }
            }
        })
        .map_err(|error| anyhow::anyhow!("session source: {error}"))?;

    // A frame reached the screen: the next one may be drawn.
    let vblank_hardware = Rc::clone(&hardware);
    handle
        .insert_source(drm_notifier, move |event, metadata, state| match event {
            DrmEvent::VBlank(crtc) => {
                let mut hardware = vblank_hardware.borrow_mut();
                if let Some(surface) = hardware
                    .surfaces
                    .iter_mut()
                    .find(|surface| surface.crtc == crtc)
                {
                    match surface.compositor.frame_submitted() {
                        // The programs that asked are told when their
                        // frame reached the screen: the display's own
                        // time for it where the driver gives one.
                        Ok(Some(Some(mut feedback))) => {
                            use smithay::backend::drm::DrmEventTime;
                            use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
                            use smithay::utils::{Clock, Monotonic, Time};
                            use smithay::wayland::presentation::Refresh;
                            let (time, sequence, flags): (Time<Monotonic>, u64, Kind) =
                                match metadata.as_ref() {
                                    Some(metadata) => match metadata.time {
                                        DrmEventTime::Monotonic(time) => (
                                            time.into(),
                                            u64::from(metadata.sequence),
                                            Kind::Vsync | Kind::HwClock | Kind::HwCompletion,
                                        ),
                                        DrmEventTime::Realtime(_) => (
                                            Clock::<Monotonic>::new().now(),
                                            u64::from(metadata.sequence),
                                            Kind::Vsync,
                                        ),
                                    },
                                    None => (Clock::<Monotonic>::new().now(), 0, Kind::Vsync),
                                };
                            let hertz = f64::from(surface.mode.vrefresh().max(1));
                            let frame = std::time::Duration::from_secs_f64(1.0 / hertz);
                            let refresh = if surface.compositor.vrr_enabled() {
                                Refresh::variable(frame)
                            } else {
                                Refresh::fixed(frame)
                            };
                            feedback.presented::<_, Monotonic>(time, refresh, sequence, flags);
                        }
                        Ok(_) => {}
                        Err(error) => log::warn!("frame: {error:?}"),
                    }
                    surface.pending = false;
                }
                // The frame is on the screen: the next one is drawn now, so
                // a display that is being drawn to runs at its own rate.
                draw(&mut hardware, state, false);
            }
            DrmEvent::Error(error) => log::error!("DRM: {error}"),
        })
        .map_err(|error| anyhow::anyhow!("DRM source: {error}"))?;

    // The socket for clients, Xwayland, and the host's tick: the same for
    // every backend.
    crate::session::open(&event_loop, display, &mut state, ready)?;

    // Drawing: every output that has no frame on its way to the screen.
    let draw_hardware = Rc::clone(&hardware);
    handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            let mut hardware = draw_hardware.borrow_mut();
            // (Kept for later while another virtual terminal has the device.)
            if hardware.active && std::mem::take(&mut state.output_rescan) {
                apply_output_changes(&mut hardware, state);
            }
            night_light(&mut hardware, state);
            // While something moves, a frame is looked for at this pace
            // (the display's own pace takes over as soon as one is on its
            // way). Otherwise this is only what brings the safety redraw:
            // a frame that is asked for is drawn right after the event
            // that asked for it, below.
            if draw(&mut hardware, state, true)
                || state.frames_owed
                || hardware.night_fade.is_some()
            {
                TimeoutAction::ToDuration(FRAME_INTERVAL)
            } else {
                TimeoutAction::ToDuration(IDLE_FRAME_INTERVAL)
            }
        })
        .map_err(|error| anyhow::anyhow!("frame source: {error}"))?;

    let asked_hardware = Rc::clone(&hardware);
    let paced_handle = handle.clone();
    event_loop.run(None, &mut state, move |state| {
        // Whatever was just handled (a client's commit, input, a command)
        // may have asked for a frame: it is drawn now and not at the next
        // look of a timer, and a desktop on which nothing happens is not
        // looked at 250 times a second.
        // A picture a program holds back for a time of its own: when that
        // time has come it is let go by a look at the displays, and until
        // then something wakes the loop at that time (nothing else may).
        match state.paced_wait() {
            Some(wait) if wait.is_zero() => state.desktop.needs_redraw = true,
            Some(wait) if !state.paced_wake => {
                state.paced_wake = true;
                let armed = paced_handle.insert_source(
                    Timer::from_duration(wait.min(std::time::Duration::from_millis(100))),
                    |_, _, state| {
                        state.paced_wake = false;
                        TimeoutAction::Drop
                    },
                );
                if armed.is_err() {
                    state.paced_wake = false;
                }
            }
            _ => {}
        }
        if state.desktop.needs_redraw || state.output_rescan {
            if let Ok(mut hardware) = asked_hardware.try_borrow_mut() {
                if hardware.active && std::mem::take(&mut state.output_rescan) {
                    apply_output_changes(&mut hardware, state);
                }
                draw(&mut hardware, state, true);
            }
        }
        if let Err(error) = state.display.flush_clients() {
            log::warn!("flush: {error}");
        }
        state.tidy_popups();
    })?;
    state.shutdown();
    Ok(())
}

/// Bring the outputs in line with the displays that are connected now: a
/// display that appeared gets an output to the right of the others, at its
/// preferred mode; one that is gone loses its output, and the engine moves
/// its workspaces and windows to an output that is left.
fn scan_outputs(hardware: &mut Hardware, state: &mut State) {
    let Ok(resources) = hardware.drm.resource_handles() else {
        log::warn!("Could not read the display device's connectors");
        return;
    };
    let mut connected = Vec::new();
    for handle in resources.connectors() {
        if let Ok(info) = hardware.drm.get_connector(*handle, false) {
            if info.state() == connector::State::Connected {
                connected.push((*handle, info));
            }
        }
    }

    // Displays that went away.
    let gone: Vec<usize> = hardware
        .surfaces
        .iter()
        .enumerate()
        .filter(|(_, surface)| {
            !connected
                .iter()
                .any(|(handle, _)| *handle == surface.connector)
        })
        .map(|(index, _)| index)
        .collect();
    for index in gone.into_iter().rev() {
        let surface = hardware.surfaces.remove(index);
        log::info!("A display was unplugged: output {:?}", surface.output);
        state.remove_output(surface.output);
    }

    // Displays that appeared.
    for (handle, info) in connected {
        if hardware
            .surfaces
            .iter()
            .any(|surface| surface.connector == handle)
        {
            continue;
        }
        let connector_name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        let profile =
            gevrek_model::outputs::profile_of(&state.config.desktop.outputs, &connector_name)
                .cloned();
        if profile.as_ref().is_some_and(|profile| !profile.enabled) {
            log::info!("{connector_name} is switched off in the configuration");
            continue;
        }
        let Some(mode) = pick_mode(&info, profile.as_ref(), &connector_name) else {
            continue;
        };
        log::info!(
            "{connector_name}: {}x{} at {} Hz",
            mode.size().0,
            mode.size().1,
            mode.vrefresh()
        );
        // A CRTC this connector can use and no other output has taken.
        let used: Vec<crtc::Handle> = hardware
            .surfaces
            .iter()
            .map(|surface| surface.crtc)
            .collect();
        let crtc = info
            .encoders()
            .iter()
            .filter_map(|encoder| hardware.drm.get_encoder(*encoder).ok())
            .flat_map(|encoder| resources.filter_crtcs(encoder.possible_crtcs()))
            .find(|crtc| !used.contains(crtc));
        let Some(crtc) = crtc else {
            log::warn!("No CRTC left for {:?}", info.interface());
            continue;
        };
        let (width, height) = mode.size();
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        // To the right of the outputs that are there.
        let next_x = state
            .desktop
            .outputs
            .iter()
            .map(|output| output.rect.x + output.rect.width)
            .max()
            .unwrap_or(0);
        let drm_surface = match hardware.drm.create_surface(crtc, mode, &[handle]) {
            Ok(surface) => surface,
            Err(error) => {
                log::warn!("{name}: {error}");
                continue;
            }
        };
        // The engine works in logical pixels: the mode divided by the scale.
        let scale = state.configured_scale(&name);
        let output = state.add_scaled_output(
            &name,
            Rect::new(
                next_x,
                0,
                (f64::from(width) / scale).round() as i32,
                (f64::from(height) / scale).round() as i32,
            ),
            scale,
        );
        // A display that stands on its side or on its head: the picture
        // is turned for it (and is then as wide as the display is high).
        // (`add_scaled_output` has turned it already.)
        let transform = state.configured_transform(&name);
        // Programs are told the display's real refresh rate and size.
        let (width_mm, height_mm) = info.size().unwrap_or((0, 0));
        state.describe_output(
            output,
            (mode.vrefresh() as i32) * 1000,
            (width_mm as i32, height_mm as i32),
        );
        tell_modes(state, output, &info, &mode);
        let wl_output = state
            .outputs
            .iter()
            .find(|(id, _)| *id == output)
            .map(|(_, wl_output)| wl_output.clone())
            .expect("the output was just added");
        // What is drawn reaches the compositor in device pixels already
        // (`scaled_elements`), so it works at scale 1 whatever the output's is.
        let _ = &wl_output;
        let compositor = DrmCompositor::new(
            OutputModeSource::Static {
                size: (i32::from(width), i32::from(height)).into(),
                scale: smithay::utils::Scale::from(1.0),
                transform: crate::desktop::turn_of(transform),
            },
            drm_surface,
            None,
            hardware.allocator.clone(),
            hardware.exporter.clone(),
            [Fourcc::Argb8888, Fourcc::Xrgb8888],
            hardware.render_formats.iter().copied(),
            hardware.drm.cursor_size(),
            Some(hardware.gbm.clone()),
        );
        match compositor {
            Ok(compositor) => {
                log::info!("Output {name}: {width}x{height}");
                hardware.surfaces.push(Surface {
                    output,
                    connector: handle,
                    crtc,
                    compositor,
                    mode,
                    transform,
                    pending: false,
                    off: false,
                });
                if let Some(surface) = hardware.surfaces.last_mut() {
                    follow_refresh(surface, state, &name);
                }
                state.output_added(output);
            }
            Err(error) => {
                log::warn!("{name}: {error:?}");
                state.remove_output(output);
            }
        }
    }
}

/// The night light: at its hours the displays are given a colour ramp
/// with less blue (and green) in it, and their own again afterwards.
/// Looked at with every pass of the frame timer; something is done only
/// when what is wanted, or the displays, are other than last time.
fn night_light(hardware: &mut Hardware, state: &mut State) {
    use chrono::Timelike;
    use gevrek_model::night_light::NEUTRAL;
    use smithay::reexports::drm::control::Device as _;
    // A temperature that is only shown to be looked at comes first, while
    // its time lasts (`DispatchCommand::NightLightPreview`).
    let shown = state
        .night_preview
        .filter(|(_, until)| std::time::Instant::now() < *until)
        .map(|(kelvin, _)| kelvin);
    if shown.is_none() {
        state.night_preview = None;
    }
    let config = &state.config.desktop.night_light;
    let now = chrono::Local::now();
    let wanted = shown
        .filter(|kelvin| *kelvin != NEUTRAL)
        .or_else(|| {
            (shown.is_none() && config.on_at(now.hour() * 60 + now.minute()))
                .then(|| config.kelvin())
        });
    let displays = hardware.surfaces.len();
    if !hardware.active
        || (hardware.night == Some((wanted, displays)) && hardware.night_fade.is_none())
    {
        return;
    }
    // Nothing was ever changed and nothing is wanted: leave the displays.
    if wanted.is_none() && hardware.night.is_none() {
        hardware.night = Some((None, displays));
        return;
    }
    let target = wanted.unwrap_or(NEUTRAL);
    // The picture turns warm, and back, over a moment and not at once
    // (asked for by the person, 2026-11-15). Only a display that comes
    // or goes while the light is as it is wanted gets its colours at once.
    let same_light = hardware.night.is_some_and(|(settled, _)| settled == wanted);
    if !same_light && hardware.night_fade.is_none_or(|(_, to, _)| to != target) {
        hardware.night_fade = Some((hardware.night_now, target, std::time::Instant::now()));
    }
    let kelvin = match hardware.night_fade {
        Some((from, to, began)) => {
            let part = began.elapsed().as_secs_f32() / NIGHT_LIGHT_FADE.as_secs_f32();
            if part >= 1.0 {
                hardware.night_fade = None;
            }
            gevrek_model::night_light::between(from, to, part)
        }
        None => target,
    };
    let fading = hardware.night_fade.is_some();
    if fading && kelvin == hardware.night_now {
        return;
    }
    let (red, green, blue) = gevrek_model::night_light::factors(kelvin);
    let mut given = 0;
    for surface in &hardware.surfaces {
        let size = hardware
            .drm
            .get_crtc(surface.crtc)
            .map(|crtc| crtc.gamma_length() as usize)
            .unwrap_or(0);
        if size == 0 {
            continue;
        }
        let ramp = |factor: f32| gevrek_model::night_light::ramp(size, factor);
        match hardware
            .drm
            .set_gamma(surface.crtc, &ramp(red), &ramp(green), &ramp(blue))
        {
            Ok(()) => given += 1,
            Err(error) => log::debug!("a display did not take a colour ramp: {error}"),
        }
    }
    hardware.night_now = kelvin;
    state.desktop.needs_redraw = true;
    // No display takes it: there is nothing to go on with.
    if given == 0 {
        hardware.night_fade = None;
    }
    if hardware.night_fade.is_some() {
        return;
    }
    let impossible = wanted.is_some() && displays > 0 && given == 0;
    if impossible != state.night_light_impossible {
        state.night_light_impossible = impossible;
        if impossible {
            log::warn!("The night light is asked for and no display takes a colour ramp");
        }
    }
    match wanted {
        Some(kelvin) if given > 0 => {
            log::info!("Night light: {given} of {displays} displays at {kelvin} K")
        }
        None if given > 0 => log::info!("Night light off: the displays have their own colours"),
        _ => {}
    }
    hardware.night = Some((wanted, displays));
}

/// A refresh rate that follows the program (`variable_refresh`): switched
/// on where the configuration asks for it and the display and its driver
/// can, off otherwise. What could not be done is told as a notice.
fn follow_refresh(surface: &mut Surface, state: &mut State, name: &str) {
    use smithay::backend::drm::VrrSupport;
    let wanted = gevrek_model::outputs::profile_of(&state.config.desktop.outputs, name)
        .is_some_and(|profile| profile.variable_refresh);
    let can = matches!(
        surface.compositor.vrr_supported(surface.connector),
        Ok(VrrSupport::Supported | VrrSupport::RequiresModeset)
    );
    let on = wanted && can && surface.compositor.use_vrr(true).is_ok();
    if !on && surface.compositor.vrr_enabled() {
        let _ = surface.compositor.use_vrr(false);
    }
    let was = state.vrr_on.contains(name);
    if on != was {
        log::info!(
            "{name}: the refresh rate {}",
            if on {
                "follows the program"
            } else {
                "is the mode's"
            }
        );
        if on {
            state.vrr_on.insert(name.to_string());
        } else {
            state.vrr_on.remove(name);
        }
        state.desktop.needs_redraw = true;
    } else if wanted && !can && !was {
        log::debug!("{name}: a refresh rate that follows the program is not possible here");
    }
}

/// Tell the rest of gevrek what modes a display offers and which it is in.
fn tell_modes(
    state: &mut State,
    output: MonitorHandle,
    info: &connector::Info,
    current: &smithay::reexports::drm::control::Mode,
) {
    let mut modes: Vec<gevrek_engine::outputs::OutputMode> = info
        .modes()
        .iter()
        .map(|mode| gevrek_engine::outputs::OutputMode {
            width: i32::from(mode.size().0),
            height: i32::from(mode.size().1),
            refresh_millihertz: mode.vrefresh() as i32 * 1000,
            preferred: mode.mode_type().contains(ModeTypeFlags::PREFERRED),
            current: mode == current,
        })
        .collect();
    // The same size and rate is listed once (displays repeat modes that
    // differ in timings only).
    modes.sort_by_key(|mode| {
        (
            -mode.width,
            -mode.height,
            -mode.refresh_millihertz,
            !mode.current,
            !mode.preferred,
        )
    });
    modes.dedup_by_key(|mode| (mode.width, mode.height, mode.refresh_millihertz));
    state.set_output_modes(
        output,
        gevrek_engine::outputs::OutputDetail {
            refresh_millihertz: current.vrefresh() as i32 * 1000,
            modes,
        },
    );
}

/// The mode a display gets: the one the configuration asks for (that size,
/// at the refresh rate nearest to the one named, the highest when none
/// is), else the display's preferred mode.
fn pick_mode(
    info: &connector::Info,
    profile: Option<&gevrek_model::outputs::OutputProfile>,
    connector_name: &str,
) -> Option<smithay::reexports::drm::control::Mode> {
    let wanted = profile.and_then(|profile| profile.mode);
    let configured = wanted.and_then(|wanted| {
        info.modes()
            .iter()
            .filter(|mode| {
                let (width, height) = mode.size();
                (u32::from(width), u32::from(height)) == (wanted.width, wanted.height)
            })
            .min_by_key(|mode| {
                let hertz = f64::from(mode.vrefresh());
                if wanted.refresh > 0.0 {
                    ((hertz - wanted.refresh).abs() * 1000.0) as i64
                } else {
                    -(hertz * 1000.0) as i64
                }
            })
            .copied()
    });
    if wanted.is_some() && configured.is_none() {
        log::warn!(
            "{connector_name} has no mode like the configured one; using its preferred mode"
        );
    }
    configured.or_else(|| {
        info.modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied()
    })
}

/// `desktop.outputs` changed while gevrek runs: a display that is switched
/// off now loses its output, one whose mode changed gets the new mode in
/// place (its windows stay), and one that is switched on is set up.
fn apply_output_changes(hardware: &mut Hardware, state: &mut State) {
    let mut switched_off = Vec::new();
    for index in 0..hardware.surfaces.len() {
        let surface = &hardware.surfaces[index];
        let Some(name) = state
            .desktop
            .outputs
            .iter()
            .find(|record| record.id == surface.output)
            .map(|record| record.name.clone())
        else {
            continue;
        };
        let profile =
            gevrek_model::outputs::profile_of(&state.config.desktop.outputs, &name).cloned();
        if profile.as_ref().is_some_and(|profile| !profile.enabled) {
            switched_off.push(index);
            continue;
        }
        let Ok(info) = hardware.drm.get_connector(surface.connector, false) else {
            continue;
        };
        let Some(mode) = pick_mode(&info, profile.as_ref(), &name) else {
            continue;
        };
        follow_refresh(&mut hardware.surfaces[index], state, &name);
        let surface = &hardware.surfaces[index];
        // The picture turned another way: the display keeps its mode, and
        // what is drawn for it is turned from now on.
        let transform = state.configured_transform(&name);
        if transform != surface.transform {
            let (width, height) = surface.mode.size();
            log::info!("{name}: the picture is turned ({transform:?})");
            let surface = &mut hardware.surfaces[index];
            surface
                .compositor
                .set_output_mode_source(OutputModeSource::Static {
                    size: (i32::from(width), i32::from(height)).into(),
                    scale: smithay::utils::Scale::from(1.0),
                    transform: crate::desktop::turn_of(transform),
                });
            surface.compositor.reset_buffers();
            surface.transform = transform;
            state.desktop.needs_redraw = true;
        }
        let surface = &hardware.surfaces[index];
        if mode == surface.mode {
            continue;
        }
        let (width, height) = mode.size();
        log::info!(
            "{name}: changing to {width}x{height} at {} Hz",
            mode.vrefresh()
        );
        let surface = &mut hardware.surfaces[index];
        if let Err(error) = surface.compositor.use_mode(mode) {
            log::warn!("{name}: the display did not take the mode: {error:?}");
            continue;
        }
        surface
            .compositor
            .set_output_mode_source(OutputModeSource::Static {
                size: (i32::from(width), i32::from(height)).into(),
                scale: smithay::utils::Scale::from(1.0),
                transform: crate::desktop::turn_of(surface.transform),
            });
        surface.mode = mode;
        let output = surface.output;
        state.resize_output(output, i32::from(width), i32::from(height));
        let (width_mm, height_mm) = info.size().unwrap_or((0, 0));
        state.describe_output(
            output,
            (mode.vrefresh() as i32) * 1000,
            (width_mm as i32, height_mm as i32),
        );
        tell_modes(state, output, &info, &mode);
    }
    for index in switched_off.into_iter().rev() {
        let surface = hardware.surfaces.remove(index);
        log::info!(
            "A display was switched off in the configuration: output {:?}",
            surface.output
        );
        state.remove_output(surface.output);
    }
    // Displays that are switched on now, and the places of all of them.
    scan_outputs(hardware, state);
    state.apply_output_config();
    state.desktop.needs_redraw = true;
}

/// Draw the outputs that are ready for a new frame. Returns whether
/// something is moving (an animation is to be advanced at the next frame).
/// Draw what is asked for. Returns whether something is still moving.
///
/// Telling the programs that a frame was shown lets go of the pictures
/// they held back until then (`wp-fifo`, `wp-commit-timing`: the graphics
/// drivers pace a panel's and a login's frames with them as much as a
/// game's). Such a picture comes in at that moment and asks for a frame of
/// its own, and nothing else may happen for a while that would have the
/// request looked at: it is looked at here, at once. (Left to the next
/// event, a program that drew steadily was shown a few times a second
/// while nothing else went on, and smoothly while the mouse moved.)
fn draw(hardware: &mut Hardware, state: &mut State, from_timer: bool) -> bool {
    let (mut animating, mut let_go) = draw_once(hardware, state, from_timer);
    // (A few times at most: a display that got its frame in a pass is
    // drawn to again at its vertical blank, which looks at everything
    // anew, and a pass that finds a frame on its way lets nothing more go.)
    for _ in 0..3 {
        if !let_go {
            break;
        }
        let (again, more) = draw_once(hardware, state, true);
        animating |= again;
        let_go = more;
    }
    animating
}

/// One pass over the displays. Returns whether something is still moving,
/// and whether a picture that was let go by this pass asks for a frame.
fn draw_once(hardware: &mut Hardware, state: &mut State, from_timer: bool) -> (bool, bool) {
    if !hardware.active {
        return (false, false);
    }
    let animating = state.desktop.tick_animations(std::time::Instant::now());
    // From the timer: only when something asked to be drawn (or now and
    // then regardless). Nothing changing means nothing is done.
    if from_timer {
        let overdue = hardware.last_draw.elapsed() >= SAFETY_REDRAW;
        if !(state.desktop.needs_redraw || animating || overdue || state.frames_owed) {
            return (animating, false);
        }
    }
    // Switched off: every display is cleared once (which switches it off)
    // and nothing is drawn until input switches them on again.
    if state.displays_off.is_some() {
        for surface in hardware.surfaces.iter_mut().filter(|surface| !surface.off) {
            match surface.compositor.clear() {
                Ok(()) => {
                    surface.off = true;
                    surface.pending = false;
                }
                Err(error) => log::warn!("a display could not be switched off: {error:?}"),
            }
        }
        state.desktop.needs_redraw = false;
        return (animating, false);
    }
    for surface in hardware.surfaces.iter_mut().filter(|surface| surface.off) {
        // On again: the next frame is a whole one, and it switches the
        // display on.
        surface.off = false;
        surface.pending = false;
        if let Err(error) = surface.compositor.reset_state() {
            log::warn!("output state: {error:?}");
        }
        surface.compositor.reset_buffers();
        state.desktop.needs_redraw = true;
    }
    hardware.last_draw = std::time::Instant::now();
    // A frame is on its way to a display: the programs are told of it
    // when it is there (at the display's vertical blank), not before.
    let in_flight = hardware.surfaces.iter().any(|surface| surface.pending);
    let Hardware {
        renderer,
        surfaces,
        capture,
        ..
    } = hardware;
    let mut drew = false;
    // An output with a frame on its way is not drawn to now: what was
    // asked for is still owed to it, and is drawn at its vertical blank.
    let mut owed = false;
    let mut changed = Vec::new();
    for surface in surfaces.iter_mut() {
        if surface.pending {
            owed = true;
            continue;
        }
        let elements = state.scaled_elements(renderer, surface.output);
        // (A fullscreen X11 window that asked never to go past the
        // compositor: no window's own picture on the display's plane.)
        let flags = if state.x11_keeps_composited() {
            FRAME_FLAGS.difference(FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY)
        } else {
            FRAME_FLAGS
        };
        match surface
            .compositor
            .render_frame(renderer, &elements, BACKGROUND, flags)
        {
            Ok(result) => {
                // Nothing changed on this output: nothing to show.
                if result.is_empty {
                    continue;
                }
                // Counted for `gevrek health`: a window's own picture on
                // the display (a game that covers it), or a frame drawn here.
                let direct = matches!(
                    result.primary_element,
                    smithay::backend::drm::compositor::PrimaryPlaneElement::Element(_)
                );
                let shown = state.frames_shown.entry(surface.output).or_default();
                if direct {
                    shown.0 += 1;
                } else {
                    shown.1 += 1;
                }
                shown.2 = direct;
                match surface
                    .compositor
                    .queue_frame(state.presentation_feedback(surface.output))
                {
                    Ok(()) => {
                        surface.pending = true;
                        drew = true;
                        changed.push(surface.output);
                    }
                    Err(error) => log::warn!("queue frame: {error:?}"),
                }
            }
            Err(error) => log::warn!("render: {error:?}"),
        }
    }
    // Pictures for other programs (screenshots, recorders).
    let asked = !state.captures.is_empty();
    state.serve_captures::<_, smithay::backend::renderer::gles::GlesTexture>(
        renderer,
        capture,
        smithay::backend::allocator::Fourcc::Abgr8888,
        Some(&changed),
    );
    // What lies behind the see-through surfaces, soft, for the next frame.
    state.refresh_soft::<_, smithay::backend::renderer::gles::GlesTexture>(
        renderer,
        smithay::backend::allocator::Fourcc::Abgr8888,
        Some(&changed),
    );
    // Every output was looked at: nothing is owed, also when nothing on
    // them had changed (which used to leave the request standing, and the
    // outputs looked at again and again).
    if drew || animating || asked || !owed {
        state.desktop.needs_redraw = false;
    }
    // Clients draw their next frame when told the last one was shown:
    // once for each frame of the display, and not at every look at the
    // displays in between. (Told at every look, a program that draws as
    // fast as it is told drew several pictures for each one that was
    // shown: 228 "frames" a second on a display of 75, and the graphics
    // card busy with pictures nobody saw.)
    //  - at a vertical blank, that is the frame; and with nothing on its
    //    way (nothing changed on the displays) there is no frame to wait
    //    for: both no more often than a fast display shows frames (two
    //    displays have two vertical blanks for each frame), with a look
    //    owed for when that time has passed;
    //  - with a frame on its way: not now, its vertical blank tells (or
    //    this does after a tenth of a second, should that never come).
    let since = state.frames_told.elapsed();
    let at_a_frame = !from_timer || !in_flight;
    let tell = if at_a_frame {
        since >= TOLD_APART
    } else {
        since >= TOLD_WITHOUT_A_BLANK
    };
    if !tell {
        state.frames_owed = at_a_frame;
        return (animating, false);
    }
    state.frames_owed = false;
    state.frames_told = std::time::Instant::now();
    let asked = state.desktop.needs_redraw;
    state.send_frame_callbacks();
    (animating, !asked && state.desktop.needs_redraw)
}
