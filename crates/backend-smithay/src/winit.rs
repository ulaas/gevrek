//! gevrek as a window inside another Wayland (or X11) session: the output
//! is that window. For development and for trying a configuration.

use crate::render::BACKGROUND;
use crate::state::State;
use gevrek_engine::Rect;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::{Display, DisplayHandle};

// What fails as a frame is drawn fails again at the next one: said once
// when it begins and once when it ends.
static RENDER: crate::failing::Failing =
    crate::failing::Failing::at("drawing a frame into the window", log::Level::Warn);
static EMPTY_FRAME: crate::failing::Failing =
    crate::failing::Failing::at("drawing an empty frame into the window", log::Level::Warn);
static SUBMIT: crate::failing::Failing =
    crate::failing::Failing::at("showing a frame in the window", log::Level::Warn);

/// Draw the frame once more into a texture of its own and write that to a
/// PPM file (the simplest picture format: a short text header, then RGB
/// bytes). The window's own framebuffer is not read: doing that broke the
/// next buffer swap.
fn save_picture(
    renderer: &mut GlesRenderer,
    state: &State,
    output: gevrek_engine::outputs::MonitorHandle,
    size: (i32, i32),
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use smithay::backend::allocator::Fourcc;
    use smithay::backend::renderer::gles::GlesTexture;
    use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
    use std::io::Write;

    let (width, height) = size;
    let mut texture: GlesTexture = renderer
        .create_buffer(Fourcc::Abgr8888, (width, height).into())
        .map_err(|error| anyhow::anyhow!("texture: {error:?}"))?;
    let elements = state.scaled_elements(renderer, output);
    let mut framebuffer = renderer
        .bind(&mut texture)
        .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
    let mut tracker =
        OutputDamageTracker::new((width, height), 1.0, smithay::utils::Transform::Normal);
    tracker
        .render_output(renderer, &mut framebuffer, 0, &elements, BACKGROUND)
        .map_err(|error| anyhow::anyhow!("render: {error:?}"))?;
    let region = smithay::utils::Rectangle::from_size((width, height).into());
    let mapping = renderer
        .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
        .map_err(|error| anyhow::anyhow!("read back: {error:?}"))?;
    let pixels = renderer
        .map_texture(&mapping)
        .map_err(|error| anyhow::anyhow!("map: {error:?}"))?;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(file, "P6\n{width} {height}\n255\n")?;
    for pixel in pixels.as_chunks::<4>().0 {
        file.write_all(&pixel[..3])?;
    }
    file.flush()?;
    log::info!("Wrote a picture of the frame to {}", path.display());
    Ok(())
}

/// The window size asked for with `--size` (`GEVREK_WL_SIZE`), as
/// `<width>x<height>` in device pixels.
fn wanted_size() -> Option<(f64, f64)> {
    parse_size(&std::env::var("GEVREK_WL_SIZE").ok()?)
}

/// `1920x1080` as a width and a height; `None` for anything else.
pub fn parse_size(text: &str) -> Option<(f64, f64)> {
    let (width, height) = text.split_once(['x', 'X'])?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    (width > 0 && height > 0).then_some((f64::from(width), f64::from(height)))
}

/// Run the compositor until its window is closed or the host asks it to
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

    // The size of the window in device pixels: `--size 1920x1080`. Without
    // it the window is 1280x800 logical pixels.
    let (width, height) = wanted_size().unwrap_or((1280.0, 800.0));
    let attributes = smithay::reexports::winit::window::Window::default_attributes()
        .with_title("gevrek")
        .with_visible(true);
    // A name of its own for the host: a rule there can tell this window
    // from every other (float it, say), also when the host is gevrek.
    let attributes = {
        use smithay::reexports::winit::platform::wayland::WindowAttributesExtWayland;
        attributes.with_name(NESTED_APP_ID, "gevrek")
    };
    let attributes = if wanted_size().is_some() {
        attributes.with_inner_size(smithay::reexports::winit::dpi::PhysicalSize::new(
            width, height,
        ))
    } else {
        attributes.with_inner_size(smithay::reexports::winit::dpi::LogicalSize::new(
            width, height,
        ))
    };
    let (mut backend, winit) = winit::init_from_attributes::<GlesRenderer>(attributes)
        .map_err(|error| anyhow::anyhow!("could not open the window: {error}"))?;
    let size = backend.window_size();
    // The scale of the host's output, or the one asked for (to try a
    // configuration at another scale).
    let scale = std::env::var("GEVREK_WL_SCALE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| (0.5..=4.0).contains(value))
        .unwrap_or_else(|| {
            // The configuration's scale, unless it leaves it at 1: then the
            // host's.
            let configured = state.configured_scale("winit");
            if configured != 1.0 {
                configured
            } else {
                backend.scale_factor()
            }
        });
    let output_id = state.add_scaled_output(
        "winit",
        Rect::new(
            0,
            0,
            (f64::from(size.w) / scale).round() as i32,
            (f64::from(size.h) / scale).round() as i32,
        ),
        scale,
    );
    let output = state.outputs[0].1.clone();
    // Clients may draw with the graphics card. A buffer the renderer
    // cannot show after all is not drawn (it is tried when it is drawn).
    crate::dmabuf::offer(&mut state, backend.renderer(), Box::new(|_| true), None);
    // The window's framebuffer has its origin at the bottom: without this
    // the picture is upside down.
    output.change_current_state(
        None,
        Some(smithay::utils::Transform::Flipped180),
        None,
        None,
    );
    // Elements reach the tracker in device pixels already (see
    // `scaled_elements`), so it works at scale 1.
    let mut capture = crate::capture::CaptureTarget::default();
    let mut damage_tracker =
        OutputDamageTracker::new((size.w, size.h), 1.0, smithay::utils::Transform::Flipped180);

    // Every key for gevrek, also those the host uses for its own shortcuts
    // (`--host-shortcuts` leaves them to the host).
    let mut host_shortcuts = if std::env::var_os("GEVREK_WL_HOST_SHORTCUTS").is_some() {
        None
    } else {
        match crate::host_keys::HostShortcuts::inhibit(backend.window()) {
            Ok(inhibitor) => Some(inhibitor),
            Err(error) => {
                log::warn!(
                    "The host keeps its own shortcuts: {error}. Bindings that use the \
                     same keys will not reach gevrek"
                );
                None
            }
        }
    };
    let mut report_host_shortcuts = move || match host_shortcuts
        .as_mut()
        .and_then(|inhibitor| inhibitor.poll())
    {
        Some(true) => {
            log::info!("The host leaves its shortcuts to gevrek while this window has the keyboard")
        }
        Some(false) => log::info!("The host uses its own shortcuts again"),
        None => {}
    };
    report_host_shortcuts();

    // The host session draws the pointer; with this set gevrek draws its
    // own arrow as it does on real hardware (to look at it, and to test it).
    state.draw_own_cursor = std::env::var_os("GEVREK_WL_OWN_CURSOR").is_some();
    // (A window in another session has no colour ramp of its own: the
    // night light is told of as not shown when it is asked for.)
    state.night_light_impossible = true;
    // The socket for clients, Xwayland, and the host's tick: the same for
    // every backend.
    crate::session::open(&event_loop, display, &mut state, ready)?;

    let signal = event_loop.get_signal();
    handle
        .insert_source(winit, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                state.resize_output(output_id, size.w, size.h);
                damage_tracker = OutputDamageTracker::new(
                    (size.w, size.h),
                    1.0,
                    smithay::utils::Transform::Flipped180,
                );
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                report_host_shortcuts();
                // How old the window's buffer is, asked before anything
                // is drawn apart from it (a picture for a program, what
                // lies behind the see-through things): that leaves the
                // renderer with another target, and the age can only be
                // asked while the window's own surface is the current one.
                let age = backend.buffer_age().unwrap_or(0);
                // Transitions move on with every frame.
                state.desktop.tick_animations(std::time::Instant::now());
                // A picture of the frame, when one was asked for.
                if let Some(path) = state.screenshot.take() {
                    let size = output
                        .current_mode()
                        .map(|mode| mode.size)
                        .unwrap_or_default();
                    if let Err(error) = save_picture(
                        backend.renderer(),
                        state,
                        output_id,
                        (size.w, size.h),
                        &path,
                    ) {
                        log::warn!("screenshot: {error}");
                    }
                }
                // Before the window's own frame, for the reason given at
                // `save_picture`.
                state.serve_captures::<_, smithay::backend::renderer::gles::GlesTexture>(
                    backend.renderer(),
                    &mut capture,
                    smithay::backend::allocator::Fourcc::Abgr8888,
                    None,
                );
                state.refresh_soft::<_, smithay::backend::renderer::gles::GlesTexture>(
                    backend.renderer(),
                    smithay::backend::allocator::Fourcc::Abgr8888,
                    None,
                );
                let damage = match backend.bind() {
                    Ok((renderer, mut framebuffer)) => {
                        let elements = state.scaled_elements(renderer, output_id);
                        match damage_tracker.render_output(
                            renderer,
                            &mut framebuffer,
                            age,
                            &elements,
                            BACKGROUND,
                        ) {
                            Ok(result) => {
                                RENDER.worked();
                                result.damage.cloned()
                            }
                            Err(error) => {
                                RENDER.failed(error);
                                None
                            }
                        }
                    }
                    Err(error) => {
                        RENDER.failed(error);
                        None
                    }
                };
                // Nothing was drawn (no damage): the last thing the renderer
                // did may have been taking in a client's new buffer, which
                // leaves its context current without the window's surface.
                // Presenting then fails, and the surface cannot be made
                // again. An empty frame makes the surface current.
                if damage.is_none() {
                    use smithay::backend::renderer::{Frame, Renderer};
                    let size = backend.window_size();
                    if let Ok((renderer, mut framebuffer)) = backend.bind() {
                        let made_current = renderer
                            .render(
                                &mut framebuffer,
                                size,
                                smithay::utils::Transform::Flipped180,
                            )
                            .and_then(|frame| frame.finish());
                        if let Err(error) = made_current {
                            EMPTY_FRAME.failed(error);
                        } else {
                            EMPTY_FRAME.worked();
                        }
                    }
                }
                match backend.submit(damage.as_deref()) {
                    Ok(()) => SUBMIT.worked(),
                    Err(error) => SUBMIT.failed(error),
                }
                // While a client's own pointer picture is shown, or gevrek's
                // arrow, the host's pointer is hidden over the window.
                let own = state.draw_own_cursor
                    || matches!(
                        state.cursor_status,
                        smithay::input::pointer::CursorImageStatus::Surface(_)
                            | smithay::input::pointer::CursorImageStatus::Hidden
                    );
                backend.window().set_cursor_visible(!own);
                state.desktop.needs_redraw = false;
                state.send_frame_callbacks();
                state.presented_now();
                // The host compositor paces the next frame.
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => {
                state.running = false;
                signal.stop();
            }
            WinitEvent::Focus(_) => {}
        })
        .map_err(|error| anyhow::anyhow!("winit source: {error}"))?;

    event_loop.run(None, &mut state, |state| {
        if let Err(error) = state.display.flush_clients() {
            log::warn!("flush: {error}");
        }
        state.tidy_popups();
    })?;
    state.shutdown();
    Ok(())
}

/// The application id of the window gevrek runs in when it is nested.
pub const NESTED_APP_ID: &str = "gevrek-nested";
