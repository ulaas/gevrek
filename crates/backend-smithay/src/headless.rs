//! gevrek with no display at all: outputs that exist only in memory, drawn
//! by the software renderer. For automated checks, here and in CI, where
//! there is neither a graphics card nor a session to open a window in.
//!
//! ```text
//! gevrek-wl --backend headless --outputs 1920x1080,1920x1080
//! ```
//!
//! Everything above the backend is as on real hardware: the same clients,
//! bindings, layouts, protocols. Clients draw with shared memory (no
//! graphics-card buffers are offered). A picture of all outputs side by
//! side is written on request, as in the nested backend
//! (`GEVREK_WL_SCREENSHOT`). Outputs can be added and removed while it
//! runs, through the test input path (`inject.rs`).

use crate::render::BACKGROUND;
use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use gevrek_engine::Rect;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::pixman;
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use std::collections::HashMap;

/// How often the outputs are looked at: frame callbacks go out at this
/// rate, and what changed is drawn.
const FRAME: std::time::Duration = std::time::Duration::from_millis(16);

/// What one output is drawn into.
struct Target {
    image: pixman::Image<'static, 'static>,
    tracker: OutputDamageTracker,
    size: (i32, i32),
    /// Nothing was drawn into the image yet.
    fresh: bool,
}

/// The sizes asked for with `--outputs` (`GEVREK_WL_OUTPUTS`), in device
/// pixels: `1920x1080,1280x720`. One 1920x1080 output without it.
pub fn wanted_outputs() -> Vec<(i32, i32)> {
    let parsed: Vec<(i32, i32)> = std::env::var("GEVREK_WL_OUTPUTS")
        .unwrap_or_default()
        .split(',')
        .filter_map(crate::winit::parse_size)
        .map(|(width, height)| (width as i32, height as i32))
        .collect();
    if parsed.is_empty() {
        vec![(1920, 1080)]
    } else {
        parsed
    }
}

/// Run the compositor until the host asks it to stop.
pub fn run(
    build: impl FnOnce(DisplayHandle) -> State,
    ready: impl FnOnce(&mut State, &str),
) -> anyhow::Result<()> {
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    let display: Display<State> = Display::new()?;
    let mut state = build(display.handle());
    state.headless = true;
    state.live_outputs = true;
    // (No display here takes a colour ramp: the night light is told of
    // as not shown when it is asked for.)
    state.night_light_impossible = true;
    state.draw_own_cursor = std::env::var_os("GEVREK_WL_OWN_CURSOR").is_some();

    for (width, height) in wanted_outputs() {
        state.add_virtual_output(width, height);
    }
    let mut renderer =
        PixmanRenderer::new().map_err(|error| anyhow::anyhow!("software renderer: {error:?}"))?;
    let mut targets: HashMap<MonitorHandle, Target> = HashMap::new();
    let mut capture = crate::capture::CaptureTarget::default();

    crate::session::open(&event_loop, display, &mut state, ready)?;

    handle
        .insert_source(Timer::immediate(), move |_, _, state| {
            state.desktop.tick_animations(std::time::Instant::now());
            if std::mem::take(&mut state.output_rescan) {
                state.apply_virtual_modes();
            }
            let picture = state.screenshot.take();
            if state.desktop.needs_redraw || picture.is_some() {
                state.desktop.needs_redraw = false;
                if let Err(error) = draw(&mut renderer, &mut targets, state) {
                    log::warn!("draw: {error}");
                }
                state.serve_captures::<_, pixman::Image<'static, 'static>>(
                    &mut renderer,
                    &mut capture,
                    Fourcc::Argb8888,
                    None,
                );
                state.refresh_soft::<_, pixman::Image<'static, 'static>>(
                    &mut renderer,
                    Fourcc::Argb8888,
                    None,
                );
            }
            if let Some(path) = picture {
                match save_picture(&mut renderer, &mut targets, state, &path) {
                    Ok(()) => log::info!("Wrote a picture of the frame to {}", path.display()),
                    Err(error) => log::warn!("screenshot: {error}"),
                }
            }
            state.send_frame_callbacks();
            state.presented_now();
            TimeoutAction::ToDuration(FRAME)
        })
        .map_err(|error| anyhow::anyhow!("frame timer: {error}"))?;

    event_loop.run(None, &mut state, |state| {
        if let Err(error) = state.display.flush_clients() {
            log::warn!("flush: {error}");
        }
        state.tidy_popups();
    })?;
    state.shutdown();
    Ok(())
}

/// The outputs there are now, with their size in device pixels.
fn output_sizes(state: &State) -> Vec<(MonitorHandle, Rect, (i32, i32))> {
    state
        .desktop
        .outputs
        .iter()
        .map(|output| {
            let device = (
                (f64::from(output.rect.width) * output.scale).round() as i32,
                (f64::from(output.rect.height) * output.scale).round() as i32,
            );
            (output.id, output.rect, device)
        })
        .collect()
}

/// Draw every output into its image. An output that is new, or whose size
/// changed, gets a new image; one that is gone loses its own.
fn draw(
    renderer: &mut PixmanRenderer,
    targets: &mut HashMap<MonitorHandle, Target>,
    state: &State,
) -> anyhow::Result<()> {
    let outputs = output_sizes(state);
    targets.retain(|id, _| outputs.iter().any(|(output, ..)| output == id));
    for (id, _, size) in outputs {
        if size.0 <= 0 || size.1 <= 0 {
            continue;
        }
        if targets.get(&id).map(|target| target.size) != Some(size) {
            let image = renderer
                .create_buffer(Fourcc::Argb8888, size.into())
                .map_err(|error| anyhow::anyhow!("output image: {error:?}"))?;
            targets.insert(
                id,
                Target {
                    image,
                    tracker: OutputDamageTracker::new(size, 1.0, smithay::utils::Transform::Normal),
                    size,
                    fresh: true,
                },
            );
        }
        let target = targets.get_mut(&id).expect("just inserted");
        let elements = state.scaled_elements(renderer, id);
        // The image keeps what was drawn before: only what changed is
        // drawn again (age 1), all of it the first time (age 0).
        let age = if std::mem::take(&mut target.fresh) {
            0
        } else {
            1
        };
        let mut framebuffer = renderer
            .bind(&mut target.image)
            .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
        target
            .tracker
            .render_output(renderer, &mut framebuffer, age, &elements, BACKGROUND)
            .map_err(|error| anyhow::anyhow!("render: {error:?}"))?;
    }
    Ok(())
}

/// Write all outputs, as they are arranged, into one PPM picture (a short
/// text header, then RGB bytes). Places between outputs are black.
fn save_picture(
    renderer: &mut PixmanRenderer,
    targets: &mut HashMap<MonitorHandle, Target>,
    state: &State,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use std::io::Write;
    let outputs = output_sizes(state);
    // The picture is in device pixels; an output's place in it is its
    // logical place times its own scale, which is exact for outputs side
    // by side at one scale and good enough for looking at otherwise.
    let scale_of = |id: MonitorHandle| {
        state
            .desktop
            .outputs
            .iter()
            .find(|output| output.id == id)
            .map_or(1.0, |output| output.scale)
    };
    // (Each output: its place and its size in the picture.)
    type Placed = (MonitorHandle, (i32, i32), (i32, i32));
    let placed: Vec<Placed> = outputs
        .iter()
        .map(|(id, rect, size)| {
            let scale = scale_of(*id);
            (
                *id,
                (
                    (f64::from(rect.x) * scale).round() as i32,
                    (f64::from(rect.y) * scale).round() as i32,
                ),
                *size,
            )
        })
        .collect();
    let left = placed.iter().map(|(_, at, _)| at.0).min().unwrap_or(0);
    let top = placed.iter().map(|(_, at, _)| at.1).min().unwrap_or(0);
    let width = placed
        .iter()
        .map(|(_, at, size)| at.0 + size.0)
        .max()
        .unwrap_or(0)
        - left;
    let height = placed
        .iter()
        .map(|(_, at, size)| at.1 + size.1)
        .max()
        .unwrap_or(0)
        - top;
    anyhow::ensure!(width > 0 && height > 0, "there is no output");
    let mut picture = vec![0u8; width as usize * height as usize * 3];
    for (id, at, size) in placed {
        let Some(target) = targets.get_mut(&id) else {
            continue;
        };
        let framebuffer = renderer
            .bind(&mut target.image)
            .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
        let region = smithay::utils::Rectangle::from_size(size.into());
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Abgr8888)
            .map_err(|error| anyhow::anyhow!("read back: {error:?}"))?;
        let pixels = renderer
            .map_texture(&mapping)
            .map_err(|error| anyhow::anyhow!("map: {error:?}"))?;
        for row in 0..size.1 as usize {
            let from = &pixels[row * size.0 as usize * 4..(row + 1) * size.0 as usize * 4];
            let start =
                ((at.1 - top) as usize + row) * width as usize * 3 + (at.0 - left) as usize * 3;
            for (index, pixel) in from.as_chunks::<4>().0.iter().enumerate() {
                picture[start + index * 3..start + index * 3 + 3].copy_from_slice(&pixel[..3]);
            }
        }
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(file, "P6\n{width} {height}\n255\n")?;
    file.write_all(&picture)?;
    file.flush()?;
    Ok(())
}

impl State {
    /// Add an output that exists only in memory, to the right of those
    /// there are. Its name is `HEADLESS-<n>`; its scale is the one the
    /// configuration has for that name.
    pub fn add_virtual_output(&mut self, width: i32, height: i32) -> MonitorHandle {
        self.virtual_outputs += 1;
        let name = format!("HEADLESS-{}", self.virtual_outputs);
        let scale = self.configured_scale(&name);
        let x = self
            .desktop
            .outputs
            .iter()
            .map(|output| output.rect.x + output.rect.width)
            .max()
            .unwrap_or(0);
        let id = self.add_scaled_output(
            &name,
            Rect::new(
                x,
                0,
                (f64::from(width) / scale).round() as i32,
                (f64::from(height) / scale).round() as i32,
            ),
            scale,
        );
        self.virtual_sizes.push((id, (width, height)));
        self.tell_virtual_modes(id, (width, height));
        self.desktop.needs_redraw = true;
        id
    }

    /// An output in memory offers the size it was made with and a few
    /// common ones, so that what sets displays up has something to choose.
    fn tell_virtual_modes(&mut self, id: MonitorHandle, current: (i32, i32)) {
        let made = self
            .virtual_sizes
            .iter()
            .find(|(known, _)| *known == id)
            .map_or(current, |(_, size)| *size);
        let mut sizes = vec![made, (1920, 1080), (1280, 720), (1024, 768)];
        if !sizes.contains(&current) {
            sizes.push(current);
        }
        sizes.sort_by_key(|size| (-size.0, -size.1));
        sizes.dedup();
        let modes = sizes
            .into_iter()
            .map(|size| gevrek_engine::outputs::OutputMode {
                width: size.0,
                height: size.1,
                refresh_millihertz: 60_000,
                preferred: size == made,
                current: size == current,
            })
            .collect();
        self.set_output_modes(
            id,
            gevrek_engine::outputs::OutputDetail {
                refresh_millihertz: 60_000,
                modes,
            },
        );
    }

    /// An output in memory has the mode `desktop.outputs` gives it, else
    /// the size it was made with.
    fn apply_virtual_modes(&mut self) {
        for (id, made) in self.virtual_sizes.clone() {
            let Some(record) = self.desktop.outputs.iter().find(|record| record.id == id) else {
                continue;
            };
            let wanted =
                gevrek_model::outputs::profile_of(&self.config.desktop.outputs, &record.name)
                    .and_then(|profile| profile.mode)
                    .map_or(made, |mode| (mode.width as i32, mode.height as i32));
            let now = (
                (f64::from(record.rect.width) * record.scale).round() as i32,
                (f64::from(record.rect.height) * record.scale).round() as i32,
            );
            if wanted != now && wanted.0 > 0 && wanted.1 > 0 {
                log::info!("{}: {}x{}", record.name, wanted.0, wanted.1);
                self.resize_output(id, wanted.0, wanted.1);
                self.tell_virtual_modes(id, wanted);
            }
        }
        self.pack_outputs();
        self.publish_outputs();
        self.desktop.needs_redraw = true;
    }
}
