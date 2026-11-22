//! What the macOS backend sees, for trying it on a Mac:
//!
//!     cargo run -p gevrek-macos --example probe -- trust [--prompt]
//!     cargo run -p gevrek-macos --example probe -- displays
//!     cargo run -p gevrek-macos --example probe -- list
//!     cargo run -p gevrek-macos --example probe -- move <window> x y w h
//!     cargo run -p gevrek-macos --example probe -- park <window>
//!     cargo run -p gevrek-macos --example probe -- close <window>
//!     cargo run -p gevrek-macos --example probe -- watch [seconds]
//!     cargo run -p gevrek-macos --example probe -- key <code> [option,shift,control,command]
//!
//! `move` and `park` put the window back where it was after a moment.
//! Run from a terminal that may use the Accessibility API (System
//! Settings, Privacy & Security, Accessibility).

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("the probe is for macOS");
}

#[cfg(target_os = "macos")]
fn main() {
    mac::main()
}

#[cfg(target_os = "macos")]
mod mac {
    use gevrek_macos::{ax, displays, observer, park, window_list};
    use gevrek_model::geometry::Rect;
    use objc2::MainThreadMarker;
    use std::time::{Duration, Instant};

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let number = |at: usize| args.get(at).and_then(|text| text.parse::<i32>().ok());
        let mtm = MainThreadMarker::new().expect("the probe runs on the main thread");
        match args.first().map(String::as_str) {
            Some("trust") => {
                println!(
                    "may use the Accessibility API: {}",
                    ax::trusted(args.iter().any(|a| a == "--prompt"))
                )
            }
            Some("displays") => show_displays(mtm),
            Some("list") | None => list(),
            Some("move") => match (number(1), number(2), number(3), number(4), number(5)) {
                (Some(id), Some(x), Some(y), Some(width), Some(height)) => {
                    move_and_back(id as u32, Rect::new(x, y, width, height))
                }
                _ => eprintln!("move <window> x y width height"),
            },
            Some("park") => match number(1) {
                Some(id) => park_and_back(id as u32, mtm),
                None => eprintln!("park <window>"),
            },
            Some("close") => match number(1).and_then(|id| window_list::find(id as u32)) {
                Some(window) => println!("close: {:?}", window.element.close()),
                None => eprintln!("close <window>"),
            },
            // Every normal window on the screen, front to back.
            // (`order all`: every level, with its layer.)
            Some("order") => {
                let all = args.get(1).map(String::as_str) == Some("all");
                for window in window_list::server_windows(true)
                    .into_iter()
                    .filter(|window| all || window.layer == window_list::NORMAL_LAYER)
                {
                    println!("{:>7} {:<20} layer {:<4} {:?}", window.id, window.owner, window.layer, window.bounds);
                }
            }
            Some("scroll") => post_scroll(
                args.get(1).map(String::as_str) == Some("up"),
                args.get(2).map(String::as_str).unwrap_or(""),
            ),
            Some("drag") => match (number(1), number(2), number(3), number(4)) {
                (Some(x1), Some(y1), Some(x2), Some(y2)) => post_drag(
                    (x1, y1),
                    (x2, y2),
                    args.get(5).map(String::as_str).unwrap_or(""),
                    args.get(6).map(String::as_str) == Some("right"),
                ),
                _ => eprintln!("drag x1 y1 x2 y2 [modifiers] [right]"),
            },
            Some("click") => match (number(1), number(2)) {
                (Some(x), Some(y)) => post_click(x, y),
                _ => eprintln!("click x y"),
            },
            Some("key") => match number(1) {
                Some(code) => post_key(code as u16, args.get(2).map(String::as_str).unwrap_or("")),
                None => eprintln!("key <code> [option,shift,control,command]"),
            },
            Some("watch") => watch(
                args.get(1)
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(15.0),
            ),
            Some(other) => eprintln!("no command {other}"),
        }
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(
            source: *const std::ffi::c_void,
            code: u16,
            down: bool,
        ) -> *const std::ffi::c_void;
        fn CGEventSetFlags(event: *const std::ffi::c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *const std::ffi::c_void);
        fn CGEventCreateScrollWheelEvent2(
            source: *const std::ffi::c_void,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
            wheel3: i32,
        ) -> *const std::ffi::c_void;
        fn CGEventCreateMouseEvent(
            source: *const std::ffi::c_void,
            kind: u32,
            at: Point,
            button: u32,
        ) -> *const std::ffi::c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const std::ffi::c_void);
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }

    fn flags_of(held: &str) -> u64 {
        held.split(',')
            .map(|name| match name.trim() {
                "shift" => 0x0002_0000,
                "control" => 0x0004_0000,
                "option" => 0x0008_0000,
                "command" => 0x0010_0000,
                _ => 0,
            })
            .fold(0, |flags, flag| flags | flag)
    }

    /// One notch of a mouse's wheel (in lines, so not continuous).
    fn post_scroll(up: bool, held: &str) {
        // SAFETY: an event made, given its flags, posted and released.
        unsafe {
            let event = CGEventCreateScrollWheelEvent2(std::ptr::null(), 1, 1, if up { 1 } else { -1 }, 0, 0);
            CGEventSetFlags(event, flags_of(held));
            CGEventPost(0, event);
            CFRelease(event);
        }
        println!("scrolled {} with {held:?}", if up { "up" } else { "down" });
    }

    /// Press a button at one place, move to another in steps, release.
    fn post_drag(from: (i32, i32), to: (i32, i32), held: &str, right: bool) {
        let (down, dragged, up, button) = if right { (3u32, 7u32, 4u32, 1u32) } else { (1, 6, 2, 0) };
        let flags = flags_of(held);
        let post = |kind: u32, (x, y): (i32, i32)| {
            // SAFETY: an event made, given its flags, posted and released.
            unsafe {
                let event = CGEventCreateMouseEvent(
                    std::ptr::null(),
                    kind,
                    Point { x: f64::from(x), y: f64::from(y) },
                    button,
                );
                CGEventSetFlags(event, flags);
                CGEventPost(0, event);
                CFRelease(event);
            }
        };
        post(down, from);
        std::thread::sleep(Duration::from_millis(60));
        let steps = 20;
        for step in 1..=steps {
            let at = (
                from.0 + (to.0 - from.0) * step / steps,
                from.1 + (to.1 - from.1) * step / steps,
            );
            post(dragged, at);
            std::thread::sleep(Duration::from_millis(25));
        }
        std::thread::sleep(Duration::from_millis(100));
        post(up, to);
        println!("dragged from {from:?} to {to:?} with {held:?}{}", if right { " (right)" } else { "" });
    }

    /// Click the left button at a place on the screen (points from the
    /// top-left of the main display), as the mouse would.
    fn post_click(x: i32, y: i32) {
        let at = Point {
            x: f64::from(x),
            y: f64::from(y),
        };
        // Down (1) and up (2) of the left button (0).
        for kind in [1u32, 2] {
            // SAFETY: an event made, posted and released.
            unsafe {
                let event = CGEventCreateMouseEvent(std::ptr::null(), kind, at, 0);
                CGEventPost(0, event);
                CFRelease(event);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        println!("clicked at {x},{y}");
    }

    /// Press and release one key, as the keyboard would (through the
    /// window server, so taps see it).
    fn post_key(code: u16, held: &str) {
        let mut flags = 0u64;
        for name in held.split(',') {
            flags |= match name.trim() {
                "shift" => 0x0002_0000,
                "control" => 0x0004_0000,
                "option" => 0x0008_0000,
                "command" => 0x0010_0000,
                _ => 0,
            };
        }
        for down in [true, false] {
            // SAFETY: an event made, given its flags, posted and released.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), code, down);
                CGEventSetFlags(event, flags);
                CGEventPost(0, event);
                CFRelease(event);
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        println!("posted key {code:#x} with {held:?}");
    }

    fn show_displays(mtm: MainThreadMarker) {
        for display in displays::displays(mtm) {
            println!(
                "{:>10}  {:<24} {:?}  free {:?}  x{}{}{}  {}",
                display.id,
                display.name,
                display.bounds,
                display.work_area,
                display.scale,
                if display.is_main { "  main" } else { "" },
                if display.is_builtin { "  built in" } else { "" },
                display.stable_id,
            );
        }
    }

    fn list() {
        let started = Instant::now();
        let listing = window_list::list();
        let took = started.elapsed();
        for window in &listing.windows {
            let element = &window.element;
            println!(
                "{:>7} {:>6} {:<20} {:?} {:<18} {:?}",
                window.server.id,
                window.server.pid,
                window.server.owner,
                window.server.bounds,
                element.subrole(),
                element.title(),
            );
        }
        for window in &listing.unmatched {
            println!(
                "{:>7} {:>6} {:<20} (no element)",
                window.id, window.pid, window.owner
            );
        }
        for (pid, error) in &listing.refused {
            println!("program {pid} would not list its windows: {error}");
        }
        println!(
            "\n{} windows matched, {} not, in {took:?}",
            listing.windows.len(),
            listing.unmatched.len()
        );
    }

    fn move_and_back(id: u32, to: Rect) {
        let Some(window) = window_list::find(id) else {
            return eprintln!("no window {id}");
        };
        let element = &window.element;
        let before = element.frame().expect("the window's frame");
        let started = Instant::now();
        let got = element.set_frame(to);
        println!("asked {to:?}, got {got:?} in {:?}", started.elapsed());
        std::thread::sleep(Duration::from_millis(1200));
        println!("back: {:?}", element.set_frame(before));
    }

    fn park_and_back(id: u32, mtm: MainThreadMarker) {
        let Some(window) = window_list::find(id) else {
            return eprintln!("no window {id}");
        };
        let element = &window.element;
        let before = element.frame().expect("the window's frame");
        let all = displays::displays(mtm);
        let centre = (before.x + before.width / 2, before.y + before.height / 2);
        let Some(display) = all.iter().find(|display| {
            let b = display.bounds;
            centre.0 >= b.x
                && centre.0 < b.x + b.width
                && centre.1 >= b.y
                && centre.1 < b.y + b.height
        }) else {
            return eprintln!("the window is on no display");
        };
        let others: Vec<Rect> = all
            .iter()
            .filter(|other| other.id != display.id)
            .map(|other| other.bounds)
            .collect();
        let (corner, at) = park::spot((before.width, before.height), display.bounds, &others);
        let _ = element.set_position(at.x, at.y);
        std::thread::sleep(Duration::from_millis(300));
        let landed = element.frame();
        println!("parked {corner:?}: asked {at:?}, landed {landed:?}");
        if let Ok(landed) = landed {
            println!(
                "looks parked: {}",
                park::looks_parked(landed, display.bounds)
            );
        }
        std::thread::sleep(Duration::from_millis(1200));
        println!("back: {:?}", element.set_frame(before));
    }

    fn watch(seconds: f64) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let listing = window_list::list();
        let mut observers: Vec<observer::Observer> = Vec::new();
        for window in &listing.windows {
            let pid = window.server.pid;
            if !observers.iter().any(|o| o.pid() == pid) {
                match observer::Observer::new(pid, sender.clone()) {
                    Ok(o) => observers.push(o),
                    Err(error) => println!("program {pid} not watched: {error}"),
                }
            }
            if let Some(o) = observers.iter_mut().find(|o| o.pid() == pid) {
                let _ = o.watch_window(&window.element);
            }
        }
        println!(
            "watching {} programs, {} windows, for {seconds}s\n",
            observers.len(),
            observers.iter().map(|o| o.watched().count()).sum::<usize>()
        );
        let started = Instant::now();
        let mut looked = Instant::now();
        while started.elapsed().as_secs_f64() < seconds {
            observer::run_for(0.1);
            // Programs started meanwhile. (The backend will hear of them
            // from NSWorkspace; the probe looks at the window list.)
            if looked.elapsed() > Duration::from_millis(500) {
                looked = Instant::now();
                for window in window_list::server_windows(true) {
                    if window.layer != window_list::NORMAL_LAYER
                        || observers.iter().any(|o| o.pid() == window.pid)
                    {
                        continue;
                    }
                    match observer::Observer::new(window.pid, sender.clone()) {
                        Ok(mut o) => {
                            if let Ok(elements) = window_list::elements_of(window.pid) {
                                for element in elements.values() {
                                    let _ = o.watch_window(element);
                                }
                            }
                            println!(
                                "{:>7.3}s {:>6} now watched ({}, {} windows)",
                                started.elapsed().as_secs_f64(),
                                window.pid,
                                window.owner,
                                o.watched().count()
                            );
                            observers.push(o);
                        }
                        Err(error) => println!("program {} not watched: {error}", window.pid),
                    }
                }
            }
            while let Ok(event) = receiver.try_recv() {
                println!(
                    "{:>7.3}s {:>6} {:<22} {}",
                    started.elapsed().as_secs_f64(),
                    event.pid,
                    format!("{:?}", event.kind),
                    event.window.map_or("-".into(), |id| format!("window {id}"))
                );
                // A new window is watched as the window manager would.
                if event.kind == observer::AxEventKind::WindowCreated {
                    if let (Some(o), Some(element)) = (
                        observers.iter_mut().find(|o| o.pid() == event.pid),
                        &event.element,
                    ) {
                        let _ = o.watch_window(element);
                    }
                }
                if event.kind == observer::AxEventKind::Destroyed {
                    if let (Some(o), Some(id)) = (
                        observers.iter_mut().find(|o| o.pid() == event.pid),
                        event.window,
                    ) {
                        o.unwatch_window(id);
                    }
                }
            }
        }
    }
}
