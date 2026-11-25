//! gevrek on Wayland.
//!
//!     gevrek-wl [--config <file>] [--address local[:<instance>]|<ip:port>]
//!               [--backend winit|drm|headless] [--outputs <w>x<h>[,...]]
//!               [--no-xwayland] [--size <width>x<height>] [--host-shortcuts] [--test-input]
//!               [--session] [--safe]
//!               [--drag-modifier super|alt|ctrl[,...]]
//!               [--ends-with-program] [-- <program> [args...]]
//!
//! `--session` is for gevrek as the session a user logs into (`resources/`
//! has the files): it tells systemd and D-Bus the display and says when it
//! is ready. `--safe` ignores the configuration file. Ctrl+Alt+Backspace
//! twice ends the session whatever the configuration says.
//! `headless` has no display: outputs in memory (`--outputs`), for checks.
//! Runs as a window inside the current session (`winit`), or on the
//! hardware of the seat when started from a bare virtual terminal (`drm`;
//! written but not run yet, see `udev.rs`), with the same gRPC server
//! as gevrek-wm on Windows: the `gevrek` CLI and the editor talk to it.
//! `--size` is the size of that window in device pixels (1280x800 logical
//! pixels without it). As a window gevrek asks the host session to send it
//! every key, so its bindings work although the host has shortcuts on the
//! same keys; `--host-shortcuts` leaves them to the host. With Super held the
//! left mouse button moves the window under the pointer and the right one
//! resizes it; `--drag-modifier` names another modifier (or several). The program after `--` is started as a client once gevrek is ready;
//! with `--ends-with-program` gevrek ends when that program does (the
//! login's greeter is run so).

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gevrek-wl runs on Linux only; on Windows use gevrek-wm.");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use gevrek_host::config_store::config_channel;
    use gevrek_smithay::host::HostLinks;
    use gevrek_smithay::State;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let mut arguments = std::env::args().skip(1);
    let mut config_path: Option<std::path::PathBuf> = None;
    let mut address = gevrek_protocol::DEFAULT_WM_SOCKET.to_string();
    let mut program: Vec<String> = Vec::new();
    let mut ends_with_program = false;
    let mut backend: Option<String> = None;
    let mut test_input = false;
    let mut session = false;
    let mut safe = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--config" => {
                config_path = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--config needs a file"))?
                        .into(),
                );
            }
            "--address" => {
                address = arguments.next().ok_or_else(|| {
                    anyhow::anyhow!("--address needs local, local:<instance> or ip:port")
                })?;
            }
            "--no-xwayland" => std::env::set_var("GEVREK_NO_XWAYLAND", "1"),
            "--drag-modifier" => {
                let names = arguments
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--drag-modifier needs super, alt or ctrl"))?;
                if gevrek_smithay::input::drag_modifiers(&names).is_none() {
                    anyhow::bail!("--drag-modifier {names}: expected super, alt or ctrl (several with commas)");
                }
                std::env::set_var("GEVREK_WL_DRAG_MODIFIER", names);
            }
            "--test-input" => test_input = true,
            "--session" => session = true,
            "--safe" => safe = true,
            "--outputs" => {
                let sizes = arguments
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--outputs needs <width>x<height>[,...]"))?;
                if sizes
                    .split(',')
                    .any(|size| gevrek_smithay::winit::parse_size(size).is_none())
                {
                    anyhow::bail!("--outputs {sizes}: expected sizes such as 1920x1080,1280x720");
                }
                std::env::set_var("GEVREK_WL_OUTPUTS", sizes);
            }
            "--host-shortcuts" => std::env::set_var("GEVREK_WL_HOST_SHORTCUTS", "1"),
            "--size" => {
                let size = arguments
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--size needs <width>x<height>"))?;
                if gevrek_smithay::winit::parse_size(&size).is_none() {
                    anyhow::bail!("--size {size}: expected <width>x<height>, such as 1920x1080");
                }
                std::env::set_var("GEVREK_WL_SIZE", size);
            }
            "--backend" => {
                backend = Some(
                    arguments
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--backend needs winit or drm"))?,
                );
            }
            "--ends-with-program" => ends_with_program = true,
            "--" => program = arguments.by_ref().collect(),
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    // (After the arguments: the setting is read from the file they name.)
    // (smithay sends the keymap again with every key a program is given,
    // in a span its log tells of at the level of information: a line a
    // key.)
    let logging = gevrek_logging::Logging::new("gevrek-wl", log::LevelFilter::Info)
        .quiet("smithay::input::keyboard", log::LevelFilter::Warn);
    if safe {
        logging.without_setting().init();
    } else {
        logging
            .config(Some(
                gevrek_model::paths::resolve_config(config_path.clone()).0,
            ))
            .init();
    }
    if test_input {
        log::warn!("--test-input: this window manager takes made-up keys and mouse input from its control endpoint");
    }
    let address = gevrek_protocol::Endpoint::parse(&address);
    gevrek_host::shortcuts::set_server_address(address.address());

    let (config_path, legacy) = gevrek_model::paths::resolve_config(config_path);
    if legacy {
        log::warn!(
            "Using config.json from the working directory. The standard place is {}; move the file there, or name it with --config",
            gevrek_model::paths::config_file().display()
        );
    }
    let (config, has_config_file, config_load_error) = if safe {
        // The user's configuration is left alone and not read: gevrek runs
        // with the one it starts with, and nothing is written.
        log::warn!("--safe: the configuration file is ignored; changes are not saved");
        (
            gevrek_model::examples::starting_configuration(),
            false,
            None,
        )
    } else {
        gevrek_server::load_config(&config_path)
    };
    let session_settings = config.desktop.session.clone();

    // What the server and the WM loop share.
    let (config_sender, config_receiver) = std::sync::mpsc::channel();
    let (command_sender, command_receiver) = std::sync::mpsc::channel();
    let stop_flag = Arc::new(Mutex::new(false));
    let pause_flag = Arc::new(Mutex::new(false));
    let overlay_data = Arc::new(Mutex::new(None));
    let (event_sender, _) = tokio::sync::broadcast::channel(1000);
    let event_broadcaster = Arc::new(event_sender);
    let (log_callback, node_callback) = gevrek_server::event_callbacks(&event_broadcaster);
    let (config_publisher, config_watch) = config_channel(config.clone());
    let healthy = Arc::new(AtomicBool::new(false));
    let runtime_error = Arc::new(Mutex::new(None));

    // The server is made inside the runtime it will run in: it starts a
    // task there that announces configuration changes it did not make itself
    // (a hand edit of the file, a toggle) to the editors that are connected.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow::anyhow!("IPC runtime: {error}"))?;
    let in_runtime = runtime.enter();
    let server = gevrek_server::WMServer::new(
        gevrek_server::ConfigSource {
            config: config_watch,
            config_path: config_path.clone(),
            has_config_file,
            config_load_error,
        },
        gevrek_server::RuntimeLinks {
            runtime_config_sender: config_sender,
            manual_command_sender: command_sender,
            execution_paused: Arc::clone(&pause_flag),
            stop_flag: Arc::clone(&stop_flag),
            overlay_data: Arc::clone(&overlay_data),
            event_broadcaster,
            runtime_healthy: Arc::clone(&healthy),
            runtime_error: Arc::clone(&runtime_error),
        },
    )
    .with_backend(gevrek_smithay::backend_info())
    .with_input_injection(test_input);
    drop(in_runtime);
    let server_stop = Arc::clone(&stop_flag);
    let server_thread = std::thread::Builder::new()
        .name("gevrek-ipc".to_string())
        .spawn(move || {
            if let Err(error) =
                runtime.block_on(gevrek_server::serve_on(address, server, server_stop))
            {
                log::error!("IPC server stopped: {error}");
            }
        })?;

    let links = HostLinks {
        config_receiver,
        command_receiver,
        config_publisher,
        config_path: Some(config_path),
        overlay_data,
        pause_flag,
        stop_flag: Arc::clone(&stop_flag),
    };
    // Ctrl+C, SIGTERM and SIGHUP stop the WM through its ordinary path:
    // the loop ends, started programs are stopped, the server goes down.
    let signal_stop = Arc::clone(&stop_flag);
    if let Err(error) = ctrlc::set_handler(move || {
        *signal_stop
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }) {
        log::warn!("No signal handler: {error}");
    }
    let loop_healthy = Arc::clone(&healthy);
    // Inside another session gevrek is a window; on a bare virtual
    // terminal it takes the hardware.
    let nested =
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let headless = backend.as_deref() == Some("headless");
    let on_hardware = match backend.as_deref() {
        Some("drm") => true,
        Some("winit") | Some("headless") => false,
        Some(other) => anyhow::bail!("unknown backend {other} (winit, drm or headless)"),
        // The session of a login is on the hardware whatever it inherited:
        // the user's service manager can still name the display of a
        // session that has ended.
        None => !nested || session,
    };
    if on_hardware {
        // Nothing gevrek starts is to look for that other display.
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("DISPLAY");
    }
    let build = move |display| {
        State::with_callbacks(display, config, Some(log_callback), Some(node_callback))
    };
    let ready = move |state: &mut State, socket: &str| {
        state.attach_host(links);
        loop_healthy.store(true, Ordering::Release);
        // Programs started from here on are clients of gevrek.
        std::env::set_var("WAYLAND_DISPLAY", socket);
        gevrek_smithay::session_env::set_own(&state.config.desktop.session);
        state.session_mode = session;
        if session {
            // The desktop's first frames come out from under the veil
            // the login went under.
            state.arrive_when(false);
            gevrek_smithay::session_env::export(&state.config.desktop.session);
            gevrek_smithay::session_env::notify("READY=1");
        }
        // Started with the startup programs, once everything a client
        // needs is up.
        state.ends_with_program = ends_with_program;
        if ends_with_program {
            // (The display manager asks the login's gevrek to go under
            // its veil before the desktop's console is shown.)
            gevrek_smithay::lock::listen_for_leave();
            // And it begins in black, as a boot splash before it ends.
            state.begin_in_black();
        }
        if let Some((name, args)) = program.split_first() {
            state.pending_program = Some(gevrek_model::settings::LaunchProfile {
                program: name.clone(),
                args: args.to_vec(),
                ..Default::default()
            });
        }
    };
    let outcome = if headless {
        gevrek_smithay::headless::run(build, ready)
    } else if on_hardware {
        gevrek_smithay::udev::run(build, ready)
    } else {
        gevrek_smithay::winit::run(build, ready)
    };
    if session {
        gevrek_smithay::session_env::notify("STOPPING=1");
        gevrek_smithay::session_env::unexport(&session_settings);
    }
    healthy.store(false, Ordering::Release);
    if let Err(error) = &outcome {
        *runtime_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.to_string());
    }
    // The server stops with the WM.
    *stop_flag
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    // It is given a moment to say goodbye, and no more: it waits for the
    // programs that listen to it (the shell, the studio) to hang up, and
    // those wait for the window manager to go. The process ending is
    // what ends them all.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    while !server_thread.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if server_thread.is_finished() {
        let _ = server_thread.join();
    } else {
        log::info!("Programs are still listening; ending without waiting for them");
    }
    outcome
}
