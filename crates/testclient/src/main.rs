//! A Wayland window for gevrek's end-to-end checks.
//!
//! It draws one colour with shared memory (no graphics card), does what it
//! is told on its standard input, and prints what the compositor sent it as
//! one JSON object per line, so a check can ask "did the menu get the
//! click?" and "what size was the window told to be?".
//!
//! ```text
//! gevrek-testclient [--title T] [--app-id ID] [--size WxH] [--color RRGGBB]
//!                   [--min-size WxH] [--max-size WxH]
//! gevrek-testclient --list-globals       (what the compositor offers)
//! ```
//!
//! Commands, one per line: `fullscreen`, `unfullscreen`, `maximize`,
//! `unmaximize`, `minimize`, `popup X Y W H` (a menu at that place of the
//! window, with the grab of the last button press when there was one),
//! `close-popup`, `title TEXT`, `quit`; `lock` (take the session lock and
//! cover every output with one colour) and `unlock`.
//!
//! Lines printed: `configure` (size, states), `popup_configure` (place and
//! size), `popup_done`, `enter`/`leave`/`button`/`motion` with the surface
//! (`window`, `popup` or `lock`) and place, `key`, `focus`, `closed`,
//! `locked`, `lock_surface` (the size an output's lock surface was given),
//! `lock_finished` (the compositor refused or ended the lock).

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gevrek-testclient is a Wayland client: Linux only.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    client::run();
}

/// The window's picture is of a format with an alpha part (`--with-alpha`).
#[cfg(target_os = "linux")]
static WITH_ALPHA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
mod client {
    use smithay_client_toolkit::reexports::calloop::channel::{self, Channel};
    use smithay_client_toolkit::reexports::calloop::EventLoop;
    use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
    use smithay_client_toolkit::{
        compositor::{CompositorHandler, CompositorState},
        delegate_compositor, delegate_keyboard, delegate_output, delegate_pointer,
        delegate_registry, delegate_seat, delegate_shm, delegate_xdg_popup, delegate_xdg_shell,
        delegate_xdg_window,
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
        registry_handlers,
        seat::{
            keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
            pointer::{PointerEvent, PointerEventKind, PointerHandler},
            touch::TouchHandler,
            Capability, SeatHandler, SeatState,
        },
        session_lock::{
            SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
            SessionLockSurfaceConfigure,
        },
        shell::{
            xdg::{
                popup::{Popup, PopupConfigure, PopupHandler},
                window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
                XdgPositioner, XdgShell, XdgSurface,
            },
            WaylandSurface,
        },
        shm::{slot::SlotPool, Shm, ShmHandler},
    };
    use std::io::BufRead;
    use std::time::Duration;
    use wayland_client::{
        globals::registry_queue_init,
        protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
        Connection, QueueHandle,
    };
    use wayland_protocols::wp::fifo::v1::client::{wp_fifo_manager_v1, wp_fifo_v1};
    use wayland_protocols::wp::presentation_time::client::{
        wp_presentation, wp_presentation_feedback,
    };
    use wayland_protocols::wp::text_input::zv3::client::{
        zwp_text_input_manager_v3::{self, ZwpTextInputManagerV3},
        zwp_text_input_v3::{self, ZwpTextInputV3},
    };
    use wayland_protocols::xdg::shell::client::xdg_positioner::{
        Anchor, ConstraintAdjustment, Gravity,
    };
    use wayland_protocols::xdg::toplevel_icon::v1::client::{
        xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1,
        xdg_toplevel_icon_v1::XdgToplevelIconV1,
    };

    fn size_argument(text: &str) -> Option<(u32, u32)> {
        let (width, height) = text.split_once(['x', 'X'])?;
        Some((width.parse().ok()?, height.parse().ok()?))
    }

    fn colour_argument(text: &str) -> Option<[u8; 3]> {
        let value = u32::from_str_radix(text.trim_start_matches('#'), 16).ok()?;
        Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
    }

    /// Print one line of JSON. Values are numbers, booleans or the few
    /// plain words this program uses, so no escaping is needed beyond
    /// quotes.
    fn say(event: &str, fields: &[(&str, String)]) {
        let mut line = format!("{{\"event\":\"{event}\"");
        for (name, value) in fields {
            line.push_str(&format!(",\"{name}\":{value}"));
        }
        line.push('}');
        println!("{line}");
    }

    fn word(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }

    mod capture;
    mod capture_ext;
    mod input_method;
    mod outputs;
    mod pointer;
    mod toplevels;
    mod workspaces;

    pub fn run() {
        let mut close_after: Option<Option<std::time::Duration>> = None;
        let mut title = "gevrek test window".to_string();
        let mut app_id = "gevrek-testclient".to_string();
        let mut size = (640u32, 480u32);
        let mut colour = [0x30u8, 0x60, 0x90];
        let mut min_size = None;
        let mut at_least: Option<(u32, u32)> = None;
        let mut max_size = None;
        let mut list_globals = false;
        let mut displays = false;
        let mut windows = false;
        let mut input_method: Option<String> = None;
        let mut candidates: Option<[u8; 3]> = None;
        let mut text_input = false;
        let mut icon: Option<[u8; 3]> = None;
        let mut pointer_steps: Option<String> = None;
        let mut pointer_extent = (1u32, 1u32);
        let mut workspaces = false;
        let mut workspace_show: Option<String> = None;
        let mut window_ask: Option<String> = None;
        let mut newer = false;
        let mut pointer_alone = false;
        let mut card = false;
        let mut newer_window: Option<String> = None;
        let mut newer_frames = 1u32;
        let mut display_set: Option<String> = None;
        let mut capture: Option<capture::Request> = None;
        let mut arguments = std::env::args().skip(1);
        while let Some(argument) = arguments.next() {
            let mut value = || arguments.next().unwrap_or_default();
            match argument.as_str() {
                // How a close request is answered: after so many
                // milliseconds (a program takes its moment), or never
                // (`stay`: a program that asks the person first).
                "--close-after" => {
                    let said = value();
                    close_after = if said == "stay" {
                        Some(None)
                    } else {
                        Some(Some(std::time::Duration::from_millis(
                            said.parse().expect("--close-after MS or stay"),
                        )))
                    };
                }
                "--with-alpha" => {
                    super::WITH_ALPHA.store(true, std::sync::atomic::Ordering::Relaxed)
                }
                "--title" => title = value(),
                "--app-id" => app_id = value(),
                "--size" => size = size_argument(&value()).expect("--size WxH"),
                "--color" => colour = colour_argument(&value()).expect("--color RRGGBB"),
                "--min-size" => min_size = size_argument(&value()),
                // As a program that cannot draw itself smaller: asked for
                // less, it stays this size (and says so as its smallest).
                "--at-least" => {
                    at_least = size_argument(&value());
                    min_size = at_least;
                }
                "--max-size" => max_size = size_argument(&value()),
                "--list-globals" => list_globals = true,
                // What a display tool does: --displays [--set NAME:what,…]
                "--displays" => displays = true,
                // What another panel's taskbar does: --windows [--do TITLE:what]
                "--windows" => windows = true,
                "--input-method" => input_method = Some(value()),
                "--candidates" => {
                    candidates = Some(colour_argument(&value()).expect("--candidates RRGGBB"))
                }
                "--text-input" => text_input = true,
                "--icon" => icon = Some(colour_argument(&value()).expect("--icon RRGGBB")),
                // What a program that moves the pointer does:
                // --move-pointer "to=X,Y;click;wheel=-1" [--extent WxH]
                "--move-pointer" => pointer_steps = Some(value()),
                "--extent" => pointer_extent = size_argument(&value()).expect("--extent WxH"),
                // What another panel's workspace list does:
                // --workspaces [--show DISPLAY:NAME]
                "--workspaces" => workspaces = true,
                "--show" => workspace_show = Some(value()),
                "--do" => window_ask = Some(value()),
                "--set" => display_set = Some(value()),
                // A picture of an output, as a screenshot tool takes it:
                // --capture FILE [--output N] [--region X,Y,W,H] [--pointer]
                "--capture" => {
                    capture = Some(capture::Request {
                        path: value(),
                        output: 0,
                        region: None,
                        pointer: false,
                    })
                }
                "--output" => {
                    let index = value().parse().expect("--output N");
                    capture.as_mut().expect("--capture first").output = index;
                }
                // The newer way of asking (ext-image-copy-capture): of one
                // window by its title, or of the output; N frames of one
                // session.
                "--window" => newer_window = Some(value()),
                "--newer" => newer = true,
                // The pointer by itself (its place, its hot spot, its
                // picture), the newer way.
                "--pointer-alone" => pointer_alone = true,
                // Into a buffer of the graphics card, shown in a window.
                "--card" => card = true,
                "--frames" => newer_frames = value().parse().expect("--frames N"),
                "--region" => {
                    let numbers: Vec<i32> = value()
                        .split(',')
                        .filter_map(|part| part.parse().ok())
                        .collect();
                    let [x, y, width, height] = numbers[..] else {
                        panic!("--region X,Y,W,H")
                    };
                    capture.as_mut().expect("--capture first").region = Some((x, y, width, height));
                }
                "--pointer" => capture.as_mut().expect("--capture first").pointer = true,
                other => panic!("unknown argument {other}"),
            }
        }

        let connection = Connection::connect_to_env().expect("no Wayland display");
        if let (Some(request), true) = (
            &capture,
            newer || pointer_alone || card || newer_window.is_some(),
        ) {
            let request = capture_ext::Request {
                path: request.path.clone(),
                window: newer_window,
                output: request.output,
                frames: newer_frames,
                pointer_alone,
                card,
                older: card && !newer,
            };
            std::process::exit(if capture_ext::run(&connection, request) {
                0
            } else {
                1
            });
        }
        if let Some(request) = capture {
            std::process::exit(if capture::run(&connection, request) {
                0
            } else {
                1
            });
        }
        if let Some(steps) = pointer_steps {
            std::process::exit(if pointer::run(&connection, &steps, pointer_extent) {
                0
            } else {
                1
            });
        }
        if workspaces {
            std::process::exit(if workspaces::run(&connection, workspace_show) {
                0
            } else {
                1
            });
        }
        if let Some(text) = input_method {
            std::process::exit(if input_method::run(&connection, text, candidates) {
                0
            } else {
                1
            });
        }
        if windows {
            std::process::exit(if toplevels::run(&connection, window_ask) {
                0
            } else {
                1
            });
        }
        if displays {
            std::process::exit(if outputs::run(&connection, display_set) {
                0
            } else {
                1
            });
        }
        let (globals, event_queue) = registry_queue_init(&connection).expect("registry");
        if list_globals {
            // What the compositor offers, one interface a line, and done.
            globals.contents().with_list(|list| {
                for global in list {
                    say(
                        "global",
                        &[
                            ("interface", word(&global.interface)),
                            ("version", global.version.to_string()),
                        ],
                    );
                }
            });
            return;
        }
        let queue = event_queue.handle();
        let mut event_loop: EventLoop<Client> = EventLoop::try_new().expect("event loop");
        WaylandSource::new(connection.clone(), event_queue)
            .insert(event_loop.handle())
            .expect("wayland source");

        let compositor = CompositorState::bind(&globals, &queue).expect("wl_compositor");
        let xdg_shell = XdgShell::bind(&globals, &queue).expect("xdg-shell");
        let shm = Shm::bind(&globals, &queue).expect("wl_shm");

        let surface = compositor.create_surface(&queue);
        let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &queue);
        window.set_title(title);
        // (`GEVREK_TEST_FULLSCREEN`: fullscreen asked for before the
        // window is first shown, as games do it.)
        if std::env::var_os("GEVREK_TEST_FULLSCREEN").is_some() {
            window.set_fullscreen(None);
        }
        window.set_app_id(app_id);
        window.set_min_size(min_size);
        window.set_max_size(max_size);
        window.commit();

        // With `--icon` the window is given a picture of its own, of that
        // colour (xdg-toplevel-icon), and the picture's memory is let go
        // of after the icon, as a browser does: `icon` says that it was
        // done, or that the compositor has no such thing.
        if let Some(colour) = icon {
            let given = (|| {
                use std::io::Write;
                use std::os::fd::AsFd;
                const SIDE: i32 = 64;
                let manager: XdgToplevelIconManagerV1 = globals.bind(&queue, 1..=1, ()).ok()?;
                let bytes = (SIDE * SIDE * 4) as usize;
                let file = input_method::memory(bytes)?;
                let mut pixels = Vec::with_capacity(bytes);
                for _ in 0..SIDE * SIDE {
                    pixels.extend_from_slice(&[colour[2], colour[1], colour[0], 255]);
                }
                (&file).write_all(&pixels).ok()?;
                let memory = shm
                    .wl_shm()
                    .create_pool(file.as_fd(), bytes as i32, &queue, ());
                let picture = memory.create_buffer(
                    0,
                    SIDE,
                    SIDE,
                    SIDE * 4,
                    wl_shm::Format::Argb8888,
                    &queue,
                    IconPicture,
                );
                let described = manager.create_icon(&queue, ());
                described.add_buffer(&picture, 1);
                manager.set_icon(window.xdg_toplevel(), Some(&described));
                // The icon first, then what it was made from.
                described.destroy();
                picture.destroy();
                memory.destroy();
                Some(())
            })();
            say(
                "icon",
                &[(
                    "given",
                    if given.is_some() { "true" } else { "false" }.to_string(),
                )],
            );
        }

        let pool = SlotPool::new((size.0 * size.1 * 4) as usize, &shm).expect("shm pool");

        // Commands arrive on standard input, from a thread of their own.
        let (sender, commands): (channel::Sender<String>, Channel<String>) = channel::channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        event_loop
            .handle()
            .insert_source(commands, |event, _, client: &mut Client| {
                if let channel::Event::Msg(line) = event {
                    client.command(&line);
                }
            })
            .expect("command source");

        let mut client = Client {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &queue),
            output_state: OutputState::new(&globals, &queue),
            compositor,
            xdg_shell,
            shm,
            queue: queue.clone(),
            pool,
            window,
            popup: None,
            popup_size: (0, 0),
            popup_drawn: false,
            wanted: size,
            at_least: at_least.unwrap_or((1, 1)),
            width: size.0,
            height: size.1,
            colour,
            configured: false,
            keyboard: None,
            pointer: None,
            touch: None,
            seat: None,
            last_press: None,
            exit: false,
            close_after,
            closing_at: None,
            session_lock_state: SessionLockState::new(&globals, &queue),
            // (Where the compositor offers it: asked of with `present`.)
            presentation: globals.bind(&queue, 1..=1, ()).ok(),
            fifo_manager: globals.bind(&queue, 1..=1, ()).ok(),
            text_manager: text_input
                .then(|| globals.bind(&queue, 1..=1, ()).ok())
                .flatten(),
            text_field: None,
            fifo: None,
            paced: None,
            session_lock: None,
            lock_surfaces: Vec::new(),
        };
        while !client.exit {
            event_loop
                .dispatch(Duration::from_millis(50), &mut client)
                .expect("dispatch");
            if client
                .closing_at
                .is_some_and(|at| std::time::Instant::now() >= at)
            {
                client.exit = true;
            }
        }
    }

    struct Client {
        registry_state: RegistryState,
        seat_state: SeatState,
        output_state: OutputState,
        compositor: CompositorState,
        xdg_shell: XdgShell,
        shm: Shm,
        queue: QueueHandle<Client>,
        pool: SlotPool,
        window: Window,
        popup: Option<Popup>,
        popup_size: (u32, u32),
        popup_drawn: bool,
        /// The size to take when the compositor leaves it to the client.
        wanted: (u32, u32),
        at_least: (u32, u32),
        width: u32,
        height: u32,
        colour: [u8; 3],
        configured: bool,
        keyboard: Option<wl_keyboard::WlKeyboard>,
        pointer: Option<wl_pointer::WlPointer>,
        touch: Option<wayland_client::protocol::wl_touch::WlTouch>,
        seat: Option<wl_seat::WlSeat>,
        /// The serial of the last button press, for a popup's grab.
        last_press: Option<u32>,
        exit: bool,
        /// How a close request is answered (`--close-after`).
        close_after: Option<Option<std::time::Duration>>,
        closing_at: Option<std::time::Instant>,
        session_lock_state: SessionLockState,
        presentation: Option<wp_presentation::WpPresentation>,
        /// Frames paced by the display (`wp-fifo`), where it is offered,
        /// and this window's own part of it once `paced` asked for it.
        fifo_manager: Option<wp_fifo_manager_v1::WpFifoManagerV1>,
        fifo: Option<wp_fifo_v1::WpFifoV1>,
        /// A run of paced frames: how many are asked for, how many were
        /// shown, and when it began.
        paced: Option<(u32, u32, std::time::Instant)>,
        session_lock: Option<SessionLock>,
        lock_surfaces: Vec<SessionLockSurface>,
        /// With `--text-input` the window is a text field (text-input v3):
        /// what an input method hands it is said (`text_enter`, `preedit`,
        /// `commit_string`, `text_done`, `text_leave`).
        text_manager: Option<ZwpTextInputManagerV3>,
        text_field: Option<ZwpTextInputV3>,
    }

    impl Client {
        fn fill(
            &mut self,
            surface: &wl_surface::WlSurface,
            width: u32,
            height: u32,
            colour: [u8; 3],
        ) {
            let (width, height) = (width.max(1), height.max(1));
            let stride = width as i32 * 4;
            let needed = (stride * height as i32) as usize;
            if self.pool.len() < needed {
                self.pool.resize(needed).expect("resize pool");
            }
            let (buffer, canvas) = self
                .pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    // (`--with-alpha`: a picture that says it may be
                    // see-through, as most programs' are. The renderer
                    // in memory blends only such a one with what is
                    // behind it when the window is drawn less than solid.)
                    if super::WITH_ALPHA.load(std::sync::atomic::Ordering::Relaxed) {
                        wl_shm::Format::Argb8888
                    } else {
                        wl_shm::Format::Xrgb8888
                    },
                )
                .expect("buffer");
            for pixel in canvas.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[colour[2], colour[1], colour[0], 0xff]);
            }
            surface.damage_buffer(0, 0, width as i32, height as i32);
            buffer.attach_to(surface).expect("attach");
            surface.commit();
        }

        /// One frame of a paced run: it waits for the frame before it,
        /// is itself to be waited for, and asks to be told when shown.
        fn paced_frame(&mut self) {
            let surface = self.window.wl_surface().clone();
            if let Some(fifo) = &self.fifo {
                fifo.wait_barrier();
                fifo.set_barrier();
            }
            surface.frame(&self.queue, surface.clone());
            let (width, height, colour) = (self.width, self.height, self.colour);
            self.fill(&surface, width, height, colour);
        }

        fn draw_window(&mut self) {
            let surface = self.window.wl_surface().clone();
            let (width, height, colour) = (self.width, self.height, self.colour);
            self.fill(&surface, width, height, colour);
        }

        fn command(&mut self, line: &str) {
            let mut words = line.split_whitespace();
            match words.next() {
                Some("fullscreen") => self.window.set_fullscreen(None),
                Some("unfullscreen") => self.window.unset_fullscreen(),
                Some("maximize") => self.window.set_maximized(),
                Some("unmaximize") => self.window.unset_maximized(),
                Some("minimize") => self.window.set_minimized(),
                Some("title") => self.window.set_title(words.collect::<Vec<_>>().join(" ")),
                Some("popup") => {
                    let numbers: Vec<i32> = words.filter_map(|word| word.parse().ok()).collect();
                    if let [x, y, width, height] = numbers[..] {
                        self.open_popup(x, y, width, height);
                    } else {
                        say("error", &[("message", word("popup X Y W H"))]);
                    }
                }
                Some("close-popup") => {
                    self.popup = None;
                    say("popup_closed", &[]);
                }
                Some("lock") => match self.session_lock_state.lock(&self.queue) {
                    Ok(lock) => self.session_lock = Some(lock),
                    Err(_) => say(
                        "error",
                        &[("message", word("the compositor has no session lock"))],
                    ),
                },
                Some("unlock") => {
                    self.lock_surfaces.clear();
                    if let Some(lock) = self.session_lock.take() {
                        lock.unlock();
                    }
                    say("unlocked", &[]);
                }
                // Draw a frame and say when the compositor tells that it
                // reached the screen (presentation-time).
                Some("present") => match self.presentation.clone() {
                    Some(presentation) => {
                        presentation.feedback(self.window.wl_surface(), &self.queue, ());
                        self.draw_window();
                    }
                    None => say("error", &[("message", word("no wp_presentation"))]),
                },
                // So many frames, each held back until the one before
                // was on the display for a frame (`wp-fifo`), and always
                // one sent ahead of the one that is shown, as a graphics
                // driver does it: how long that took is said at the end.
                Some("paced") => {
                    let frames = words.next().and_then(|n| n.parse().ok()).unwrap_or(60);
                    match self.fifo_manager.clone() {
                        Some(manager) => {
                            if self.fifo.is_none() {
                                self.fifo = Some(manager.get_fifo(
                                    self.window.wl_surface(),
                                    &self.queue,
                                    (),
                                ));
                            }
                            self.paced = Some((frames, 0, std::time::Instant::now()));
                            self.paced_frame();
                            self.paced_frame();
                        }
                        None => say("error", &[("message", word("no wp_fifo_manager_v1"))]),
                    }
                }
                Some("quit") => self.exit = true,
                Some(other) => say(
                    "error",
                    &[("message", word(&format!("unknown command {other}")))],
                ),
                None => {}
            }
        }

        fn open_popup(&mut self, x: i32, y: i32, width: i32, height: i32) {
            let positioner = XdgPositioner::new(&self.xdg_shell).expect("positioner");
            positioner.set_size(width, height);
            // Anchored to one point of the window, growing down and right,
            // and allowed to slide or flip to stay on the output.
            positioner.set_anchor_rect(x, y, 1, 1);
            positioner.set_anchor(Anchor::TopLeft);
            positioner.set_gravity(Gravity::BottomRight);
            positioner.set_constraint_adjustment(
                ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY,
            );
            // The grab must be asked for before the popup's first commit.
            let surface = self.compositor.create_surface(&self.queue);
            let popup = Popup::from_surface(
                Some(self.window.xdg_surface()),
                &positioner,
                &self.queue,
                surface,
                &self.xdg_shell,
            )
            .expect("popup");
            if let (Some(seat), Some(serial)) = (&self.seat, self.last_press) {
                popup.xdg_popup().grab(seat, serial);
            }
            popup.wl_surface().commit();
            self.popup_size = (width as u32, height as u32);
            self.popup_drawn = false;
            self.popup = Some(popup);
        }

        fn surface_name(&self, surface: &wl_surface::WlSurface) -> &'static str {
            if self
                .popup
                .as_ref()
                .is_some_and(|popup| popup.wl_surface() == surface)
            {
                "popup"
            } else if self
                .lock_surfaces
                .iter()
                .any(|lock| lock.wl_surface() == surface)
            {
                "lock"
            } else {
                "window"
            }
        }
    }

    impl CompositorHandler for Client {
        fn scale_factor_changed(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: i32,
        ) {
        }
        fn transform_changed(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: wl_output::Transform,
        ) {
        }
        fn frame(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: u32,
        ) {
            // A frame of a paced run was shown: the next one is sent, or
            // the run is over.
            let Some((wanted, shown, began)) = self.paced.as_mut() else {
                return;
            };
            *shown += 1;
            if *shown >= *wanted {
                let took = began.elapsed().as_millis();
                say(
                    "paced",
                    &[
                        ("frames", shown.to_string()),
                        ("milliseconds", took.to_string()),
                    ],
                );
                self.paced = None;
            } else {
                self.paced_frame();
            }
        }
        fn surface_enter(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: &wl_output::WlOutput,
        ) {
        }
        fn surface_leave(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: &wl_output::WlOutput,
        ) {
        }
    }

    impl OutputHandler for Client {
        fn output_state(&mut self) -> &mut OutputState {
            &mut self.output_state
        }
        fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
        fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        }
        fn output_destroyed(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: wl_output::WlOutput,
        ) {
        }
    }

    impl WindowHandler for Client {
        fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
            say("closed", &[]);
            match self.close_after {
                None => self.exit = true,
                // Later; or not at all.
                Some(Some(wait)) => self.closing_at = Some(std::time::Instant::now() + wait),
                Some(None) => {}
            }
        }

        fn configure(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &Window,
            configure: WindowConfigure,
            _serial: u32,
        ) {
            // The size the compositor asks for, or the client's own.
            self.width = configure
                .new_size
                .0
                .map_or(self.wanted.0, |width| width.get());
            self.height = configure
                .new_size
                .1
                .map_or(self.wanted.1, |height| height.get());
            self.width = self.width.max(self.at_least.0);
            self.height = self.height.max(self.at_least.1);
            let mut states = Vec::new();
            for (name, on) in [
                ("fullscreen", configure.is_fullscreen()),
                ("maximized", configure.is_maximized()),
                ("activated", configure.is_activated()),
                ("resizing", configure.is_resizing()),
                ("tiled", configure.is_tiled()),
            ] {
                if on {
                    states.push(word(name));
                }
            }
            say(
                "configure",
                &[
                    ("width", self.width.to_string()),
                    ("height", self.height.to_string()),
                    ("asked", configure.new_size.0.is_some().to_string()),
                    ("states", format!("[{}]", states.join(","))),
                ],
            );
            self.configured = true;
            self.draw_window();
        }
    }

    impl PopupHandler for Client {
        fn configure(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            popup: &Popup,
            configure: PopupConfigure,
        ) {
            say(
                "popup_configure",
                &[
                    ("x", configure.position.0.to_string()),
                    ("y", configure.position.1.to_string()),
                    ("width", configure.width.to_string()),
                    ("height", configure.height.to_string()),
                ],
            );
            let surface = popup.wl_surface().clone();
            let (width, height) = (
                configure.width.max(1) as u32,
                configure.height.max(1) as u32,
            );
            self.popup_size = (width, height);
            // A colour of its own, so a picture tells menu from window.
            self.fill(&surface, width, height, [0xe0, 0x40, 0xd0]);
            self.popup_drawn = true;
        }

        fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Popup) {
            say("popup_done", &[]);
            self.popup = None;
        }
    }

    impl SeatHandler for Client {
        fn seat_state(&mut self) -> &mut SeatState {
            &mut self.seat_state
        }
        fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
        fn new_capability(
            &mut self,
            _: &Connection,
            queue: &QueueHandle<Self>,
            seat: wl_seat::WlSeat,
            capability: Capability,
        ) {
            self.seat = Some(seat.clone());
            if capability == Capability::Keyboard && self.keyboard.is_none() {
                self.keyboard = self.seat_state.get_keyboard(queue, &seat, None).ok();
            }
            if let (Some(manager), None) = (&self.text_manager, &self.text_field) {
                self.text_field = Some(manager.get_text_input(&seat, queue, ()));
            }
            if capability == Capability::Pointer && self.pointer.is_none() {
                self.pointer = self.seat_state.get_pointer(queue, &seat).ok();
            }
            if capability == Capability::Touch && self.touch.is_none() {
                self.touch = self.seat_state.get_touch(queue, &seat).ok();
            }
        }
        fn remove_capability(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: wl_seat::WlSeat,
            _: Capability,
        ) {
        }
        fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    }

    impl KeyboardHandler for Client {
        fn enter(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_keyboard::WlKeyboard,
            surface: &wl_surface::WlSurface,
            _: u32,
            _: &[u32],
            _: &[Keysym],
        ) {
            say(
                "focus",
                &[
                    ("surface", word(self.surface_name(surface))),
                    ("on", "true".to_string()),
                ],
            );
        }
        fn leave(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_keyboard::WlKeyboard,
            surface: &wl_surface::WlSurface,
            _: u32,
        ) {
            say(
                "focus",
                &[
                    ("surface", word(self.surface_name(surface))),
                    ("on", "false".to_string()),
                ],
            );
        }
        fn press_key(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_keyboard::WlKeyboard,
            _: u32,
            event: KeyEvent,
        ) {
            say(
                "key",
                &[
                    ("pressed", "true".to_string()),
                    ("keysym", event.keysym.raw().to_string()),
                    ("text", word(event.utf8.as_deref().unwrap_or(""))),
                ],
            );
        }
        fn release_key(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_keyboard::WlKeyboard,
            _: u32,
            event: KeyEvent,
        ) {
            say(
                "key",
                &[
                    ("pressed", "false".to_string()),
                    ("keysym", event.keysym.raw().to_string()),
                ],
            );
        }
        fn update_modifiers(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_keyboard::WlKeyboard,
            _: u32,
            _: Modifiers,
            _: u32,
        ) {
        }
    }

    impl PointerHandler for Client {
        fn pointer_frame(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_pointer::WlPointer,
            events: &[PointerEvent],
        ) {
            for event in events {
                let surface = word(self.surface_name(&event.surface));
                let place = [
                    ("surface", surface),
                    ("x", format!("{:.0}", event.position.0)),
                    ("y", format!("{:.0}", event.position.1)),
                ];
                match event.kind {
                    PointerEventKind::Enter { .. } => say("enter", &place),
                    PointerEventKind::Leave { .. } => say("leave", &place),
                    PointerEventKind::Motion { .. } => say("motion", &place),
                    PointerEventKind::Press { button, serial, .. } => {
                        self.last_press = Some(serial);
                        let mut fields = place.to_vec();
                        fields.push(("button", button.to_string()));
                        fields.push(("pressed", "true".to_string()));
                        say("button", &fields);
                    }
                    PointerEventKind::Release { button, .. } => {
                        let mut fields = place.to_vec();
                        fields.push(("button", button.to_string()));
                        fields.push(("pressed", "false".to_string()));
                        say("button", &fields);
                    }
                    PointerEventKind::Axis { .. } => say("axis", &place),
                }
            }
        }
    }

    impl SessionLockHandler for Client {
        fn locked(&mut self, _: &Connection, queue: &QueueHandle<Self>, lock: SessionLock) {
            say("locked", &[]);
            for output in self.output_state.outputs() {
                let surface = self.compositor.create_surface(queue);
                self.lock_surfaces
                    .push(lock.create_lock_surface(surface, &output, queue));
            }
        }

        fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
            say("lock_finished", &[]);
            self.lock_surfaces.clear();
            self.session_lock = None;
        }

        fn configure(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            surface: SessionLockSurface,
            configure: SessionLockSurfaceConfigure,
            _serial: u32,
        ) {
            let (width, height) = configure.new_size;
            say(
                "lock_surface",
                &[("width", width.to_string()), ("height", height.to_string())],
            );
            let surface = surface.wl_surface().clone();
            // A colour of its own, so a picture tells the lock from the rest.
            self.fill(&surface, width, height, [0x10, 0x10, 0x80]);
        }
    }

    impl ShmHandler for Client {
        fn shm_state(&mut self) -> &mut Shm {
            &mut self.shm
        }
    }

    impl wayland_client::Dispatch<wp_fifo_manager_v1::WpFifoManagerV1, ()> for Client {
        fn event(
            _: &mut Self,
            _: &wp_fifo_manager_v1::WpFifoManagerV1,
            _: wp_fifo_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl wayland_client::Dispatch<wp_fifo_v1::WpFifoV1, ()> for Client {
        fn event(
            _: &mut Self,
            _: &wp_fifo_v1::WpFifoV1,
            _: wp_fifo_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl wayland_client::Dispatch<wp_presentation::WpPresentation, ()> for Client {
        fn event(
            _: &mut Self,
            _: &wp_presentation::WpPresentation,
            _: wp_presentation::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl wayland_client::Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for Client {
        fn event(
            _: &mut Self,
            _: &wp_presentation_feedback::WpPresentationFeedback,
            event: wp_presentation_feedback::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                wp_presentation_feedback::Event::Presented {
                    tv_sec_lo,
                    tv_nsec,
                    refresh,
                    ..
                } => say(
                    "presented",
                    &[
                        ("seconds", tv_sec_lo.to_string()),
                        ("nanoseconds", tv_nsec.to_string()),
                        ("refresh", refresh.to_string()),
                    ],
                ),
                wp_presentation_feedback::Event::Discarded => say("discarded", &[]),
                _ => {}
            }
        }
    }

    /// The memory of the picture a window's icon is made from.
    struct IconPicture;

    macro_rules! unheard {
        ($interface:ty, $data:ty) => {
            impl wayland_client::Dispatch<$interface, $data> for Client {
                fn event(
                    _: &mut Self,
                    _: &$interface,
                    _: <$interface as wayland_client::Proxy>::Event,
                    _: &$data,
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        };
    }
    unheard!(XdgToplevelIconManagerV1, ());
    unheard!(XdgToplevelIconV1, ());
    unheard!(wayland_client::protocol::wl_shm_pool::WlShmPool, ());
    unheard!(wayland_client::protocol::wl_buffer::WlBuffer, IconPicture);

    /// Where the text field says its text cursor is, from the corner of
    /// the window's surface: a place, a width and a height.
    const TEXT_CURSOR: (i32, i32, i32, i32) = (40, 60, 2, 20);

    impl wayland_client::Dispatch<ZwpTextInputV3, ()> for Client {
        fn event(
            _: &mut Self,
            field: &ZwpTextInputV3,
            event: zwp_text_input_v3::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                // The window has the keyboard and an input method is
                // there: the field is one that text can be put into.
                zwp_text_input_v3::Event::Enter { .. } => {
                    field.enable();
                    field.set_cursor_rectangle(
                        TEXT_CURSOR.0,
                        TEXT_CURSOR.1,
                        TEXT_CURSOR.2,
                        TEXT_CURSOR.3,
                    );
                    field.commit();
                    say("text_enter", &[]);
                }
                zwp_text_input_v3::Event::Leave { .. } => {
                    field.disable();
                    field.commit();
                    say("text_leave", &[]);
                }
                zwp_text_input_v3::Event::PreeditString { text, .. } => {
                    say("preedit", &[("text", word(&text.unwrap_or_default()))]);
                }
                zwp_text_input_v3::Event::CommitString { text } => {
                    say(
                        "commit_string",
                        &[("text", word(&text.unwrap_or_default()))],
                    );
                }
                zwp_text_input_v3::Event::Done { .. } => say("text_done", &[]),
                _ => {}
            }
        }
    }

    impl wayland_client::Dispatch<ZwpTextInputManagerV3, ()> for Client {
        fn event(
            _: &mut Self,
            _: &ZwpTextInputManagerV3,
            _: zwp_text_input_manager_v3::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    /// A finger on a touch screen, as the window is told of it:
    /// `touch_down` and `touch_motion` with the finger and its place on
    /// the surface, `touch_up`, `touch_cancel`.
    impl TouchHandler for Client {
        fn down(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wayland_client::protocol::wl_touch::WlTouch,
            _: u32,
            _: u32,
            surface: wl_surface::WlSurface,
            id: i32,
            position: (f64, f64),
        ) {
            say(
                "touch_down",
                &[
                    ("surface", word(self.surface_name(&surface))),
                    ("finger", id.to_string()),
                    ("x", format!("{:.0}", position.0)),
                    ("y", format!("{:.0}", position.1)),
                ],
            );
        }

        fn up(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wayland_client::protocol::wl_touch::WlTouch,
            _: u32,
            _: u32,
            id: i32,
        ) {
            say("touch_up", &[("finger", id.to_string())]);
        }

        fn motion(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wayland_client::protocol::wl_touch::WlTouch,
            _: u32,
            id: i32,
            position: (f64, f64),
        ) {
            say(
                "touch_motion",
                &[
                    ("finger", id.to_string()),
                    ("x", format!("{:.0}", position.0)),
                    ("y", format!("{:.0}", position.1)),
                ],
            );
        }

        fn shape(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wayland_client::protocol::wl_touch::WlTouch,
            _: i32,
            _: f64,
            _: f64,
        ) {
        }

        fn orientation(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wayland_client::protocol::wl_touch::WlTouch,
            _: i32,
            _: f64,
        ) {
        }

        fn cancel(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wayland_client::protocol::wl_touch::WlTouch,
        ) {
            say("touch_cancel", &[]);
        }
    }
    smithay_client_toolkit::delegate_touch!(Client);

    delegate_compositor!(Client);
    delegate_output!(Client);
    delegate_shm!(Client);
    delegate_seat!(Client);
    delegate_keyboard!(Client);
    delegate_pointer!(Client);
    delegate_xdg_shell!(Client);
    delegate_xdg_window!(Client);
    delegate_xdg_popup!(Client);
    smithay_client_toolkit::delegate_session_lock!(Client);
    delegate_registry!(Client);

    impl ProvidesRegistryState for Client {
        fn registry(&mut self) -> &mut RegistryState {
            &mut self.registry_state
        }
        registry_handlers![OutputState, SeatState];
    }
}
