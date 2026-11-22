//! gevrek-wm: the window manager for Windows and macOS with its gRPC
//! server. On Linux the window manager is `gevrek-wl`.

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("gevrek-wm runs on Windows and macOS; on Linux use gevrek-wl.");
    std::process::exit(1);
}

/// On macOS the window manager runs on the main thread (AppKit's screens
/// are read there, the Accessibility observers' events come on its run
/// loop), and the server in a thread of its own, as `gevrek-wl` does.
#[cfg(target_os = "macos")]
mod macos_main {
    use clap::Parser;
    use gevrek_host::config_store::config_channel;
    use gevrek_host::wm::HostLinks;
    use gevrek_protocol::DEFAULT_WM_SOCKET;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Parser, Debug)]
    #[command(name = "gevrek-wm")]
    #[command(about = "gevrek window manager runtime and gRPC server")]
    struct Args {
        /// Where the gRPC server listens: `local` (a socket only this
        /// user's programs can open; the default), `local:<instance>`, or
        /// `ip:port` for TCP, which anything on the machine can connect to.
        #[arg(long, default_value = DEFAULT_WM_SOCKET)]
        address: String,

        /// Configuration file to load and update. Without it: the standard
        /// file (`gevrek config-path` prints it).
        #[arg(long)]
        config: Option<PathBuf>,
    }

    pub fn main() -> anyhow::Result<()> {
        let args = Args::parse();
        gevrek_logging::Logging::new("gevrek-wm", log::LevelFilter::Info)
            .config(Some(gevrek_model::paths::resolve_config(args.config.clone()).0))
            .init();
        log::info!("=== Gevrek Window Manager Runtime Starting (macOS) ===");
        gevrek_model::paths::claim_process("wm");

        let address = gevrek_protocol::Endpoint::parse(&args.address);
        gevrek_host::shortcuts::set_server_address(address.address());
        let (config_path, legacy) = gevrek_model::paths::resolve_config(args.config);
        if legacy {
            log::warn!(
                "Using config.json from the working directory. The standard place is {}; move the file there, or name it with --config",
                gevrek_model::paths::config_file().display()
            );
        }
        let (config, has_config_file, config_load_error) = gevrek_server::load_config(&config_path);

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

        // The server is made inside the runtime it runs in (it starts a task
        // there that tells editors of configuration changes).
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
        .with_backend(gevrek_macos::backend_info());
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
        // Ctrl+C and SIGTERM stop the WM through its ordinary path: the
        // loop ends, parked windows come back, the server goes down.
        let signal_stop = Arc::clone(&stop_flag);
        if let Err(error) = ctrlc::set_handler(move || {
            *signal_stop
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        }) {
            log::warn!("No signal handler: {error}");
        }

        let links = HostLinks {
            config_receiver,
            command_receiver,
            config_publisher,
            config_path: Some(config_path),
            overlay_data,
            pause_flag,
            stop_flag: Arc::clone(&stop_flag),
        };
        let ready_healthy = Arc::clone(&healthy);
        let outcome = gevrek_macos::runtime::run(
            config,
            links,
            Some(log_callback),
            Some(node_callback),
            move || ready_healthy.store(true, Ordering::Release),
        );
        healthy.store(false, Ordering::Release);
        if let Err(error) = &outcome {
            log::error!("Window manager runtime failed: {error}");
            *runtime_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error.to_string());
        }
        *stop_flag
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        let _ = server_thread.join();
        log::info!("WM Server shutting down");
        outcome
    }
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    macos_main::main()
}

#[cfg(windows)]
mod windows_main {
    use anyhow::Result;
    use clap::Parser;
    use gevrek_host::config_store::config_channel;
    use gevrek_protocol::DEFAULT_WM_SOCKET;
    use gevrek_server::WMServer;
    use gevrek_win32::runtime::Runtime;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Parser, Debug)]
    #[command(name = "gevrek-wm")]
    #[command(about = "gevrek window manager runtime and gRPC server")]
    struct Args {
        /// Where the gRPC server listens: `local` (a named pipe only this
        /// user's programs can open; the default), `local:<instance>`, or
        /// `ip:port` for TCP, which anything on the machine can connect to.
        #[arg(long, default_value = DEFAULT_WM_SOCKET)]
        address: String,

        /// Configuration file to load and update. Without it: the standard
        /// file (`gevrek config-path` prints it).
        #[arg(long)]
        config: Option<PathBuf>,
    }

    /// Make the WM per-monitor DPI aware (v2) before any window is touched, so
    /// window, monitor, and DWM frame rects are all reported in physical pixels.
    /// The embedded manifest (see `build.rs`) normally sets this already; in that
    /// case the call fails with ACCESS_DENIED, which is expected and harmless.
    fn enable_per_monitor_dpi_awareness() {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // SAFETY: process-wide setting with a constant argument; no pointers involved.
        if let Err(e) =
            unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        {
            log::debug!("SetProcessDpiAwarenessContext not applied: {e}");
        }
    }

    #[tokio::main]
    pub async fn main() -> Result<()> {
        enable_per_monitor_dpi_awareness();
        let args = Args::parse();

        gevrek_logging::Logging::new("gevrek-wm", log::LevelFilter::Info)
            .config(Some(gevrek_model::paths::resolve_config(args.config.clone()).0))
            .init();

        log::info!("=== Gevrek Window Manager Runtime Starting ===");
        // `gevrek stop` of another instance must not take this one for its own.
        gevrek_model::paths::claim_process("wm");

        let (config_path, legacy) = gevrek_model::paths::resolve_config(args.config);
        if legacy {
            log::warn!(
                "Using config.json from the working directory. The standard place is {}; move the file there, or name it with --config",
                gevrek_model::paths::config_file().display()
            );
        }
        let (config, has_config_file, config_load_error) = gevrek_server::load_config(&config_path);

        // Create runtime communication channels and flags
        let (config_sender, config_receiver) = std::sync::mpsc::channel();
        let (manual_command_sender, manual_command_receiver) = std::sync::mpsc::channel();
        let stop_flag = Arc::new(Mutex::new(false));
        gevrek_win32::runtime::process_guard::install(Arc::clone(&stop_flag));
        // Another gevrek-wm of this instance that is running has its
        // windows hidden on purpose: this one must not show them (the
        // journal is the same file) before it finds out, further down,
        // that the place is taken.
        if let gevrek_protocol::Endpoint::Local(instance) =
            gevrek_protocol::Endpoint::parse(&args.address)
        {
            let pipe = gevrek_protocol::endpoint::pipe_name(&instance);
            // (Busy counts as there: 231 is ERROR_PIPE_BUSY.)
            let answered = match tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe)
            {
                Ok(_) => true,
                Err(error) => error.raw_os_error() == Some(231),
            };
            if answered {
                anyhow::bail!(
                    "another gevrek already answers on {pipe}; stop it first (`gevrek stop`)"
                );
            }
        }
        // Unhide anything a crashed or killed previous instance left hidden,
        // before the runtime enumerates windows so they get adopted again.
        if let Some(journal_path) = gevrek_win32::dwm::hidden_journal::default_path() {
            let recovered = gevrek_win32::dwm::hidden_journal::recover_orphans(journal_path);
            if recovered > 0 {
                log::warn!("Recovered {recovered} window(s) left hidden by a previous run");
            }
        }
        let execution_paused = Arc::new(Mutex::new(false));
        let log_entries = Arc::new(Mutex::new(Vec::new()));

        // Create shared overlay data that Runtime updates and gRPC server reads
        let overlay_data = Arc::new(Mutex::new(None));
        let runtime_overlay_data = Arc::clone(&overlay_data);

        // Create event broadcaster for log messages and execution events
        let (event_sender, _) = tokio::sync::broadcast::channel(1000);
        let event_broadcaster = Arc::new(event_sender);

        // What the runtime logs and what its nodes do reaches subscribers.
        let (log_broadcast_callback, node_event_callback) =
            gevrek_server::event_callbacks(&event_broadcaster);

        // Start the runtime in a separate OS thread (not tokio task, because it needs blocking ops)
        let runtime_config = config.clone();
        let runtime_stop_flag = Arc::clone(&stop_flag);
        let runtime_pause_flag = Arc::clone(&execution_paused);
        let runtime_log_entries = Arc::clone(&log_entries);
        let runtime_healthy = Arc::new(AtomicBool::new(false));
        let runtime_error = Arc::new(Mutex::new(None));
        let thread_runtime_healthy = Arc::clone(&runtime_healthy);
        let thread_runtime_error = Arc::clone(&runtime_error);

        log::info!("Starting Window Manager Runtime...");
        let runtime_config_path = config_path.clone();
        // The runtime owns the running configuration; the server reads it.
        let (config_publisher, config_watch) = config_channel(config.clone());
        let runtime_thread = std::thread::spawn(move || {
            match Runtime::new_with_manual_commands(
                runtime_config,
                gevrek_win32::runtime::RuntimeLines {
                    stop_flag: Arc::clone(&runtime_stop_flag),
                    pause_flag: runtime_pause_flag,
                    log_entries: runtime_log_entries,
                    config_receiver,
                    manual_command_receiver,
                    debug_overlay_data: Some(runtime_overlay_data),
                    log_broadcast: Some(log_broadcast_callback),
                    node_event_broadcast: Some(node_event_callback),
                },
            ) {
                Ok(mut runtime) => {
                    runtime.set_config_watch(runtime_config_path);
                    runtime.set_config_publisher(config_publisher);
                    log::info!("Runtime initialized successfully");
                    // From here on a loop that does not pass is reported by
                    // `health`, including one stuck in startup.
                    gevrek_win32::runtime::process_guard::beat();
                    thread_runtime_healthy.store(true, Ordering::Release);
                    // A panic must not leave the server reporting a healthy
                    // runtime. Dropping `runtime` at the end of this arm runs its
                    // Drop guard, which restores every hidden window.
                    let outcome =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.run()));
                    thread_runtime_healthy.store(false, Ordering::Release);
                    let failure = match outcome {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(format!("Window manager runtime failed: {}", e)),
                        Err(panic) => {
                            let message = panic
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| panic.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            Some(format!("Window manager runtime panicked: {}", message))
                        }
                    };
                    if let Some(failure) = failure {
                        log::error!("{}", failure);
                        *thread_runtime_error
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(failure);
                        // A server without a runtime manages nothing and would
                        // save configurations it can never apply. Stop the
                        // process (as a failed start does); it exits with the
                        // error, and whatever started it can start it again.
                        *runtime_stop_flag
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
                    }
                }
                Err(e) => {
                    log::error!("Failed to initialize runtime: {}", e);
                    *thread_runtime_error
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(format!("Failed to initialize runtime: {}", e));
                    *runtime_stop_flag
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
                }
            }
        });

        // Start gRPC server
        let addr = gevrek_protocol::Endpoint::parse(&args.address);
        gevrek_host::shortcuts::set_server_address(addr.address());
        let grpc_stop_flag = Arc::clone(&stop_flag);
        let wm_server = WMServer::new(
            gevrek_server::ConfigSource {
                config: config_watch,
                config_path,
                has_config_file,
                config_load_error,
            },
            gevrek_server::RuntimeLinks {
                runtime_config_sender: config_sender,
                manual_command_sender,
                execution_paused,
                stop_flag: grpc_stop_flag,
                overlay_data,
                event_broadcaster,
                runtime_healthy,
                runtime_error: Arc::clone(&runtime_error),
            },
        )
        .with_backend(gevrek_win32::backend_info());

        gevrek_server::serve_on(addr, wm_server, Arc::clone(&stop_flag)).await?;

        // Signal runtime to stop
        *stop_flag.lock().unwrap_or_else(|p| p.into_inner()) = true;
        let runtime_joined = runtime_thread.join();
        // A console close waits for this before letting Windows end the process.
        gevrek_win32::runtime::process_guard::shutdown_complete();
        if runtime_joined.is_err() {
            anyhow::bail!("Window manager runtime thread panicked during shutdown");
        }
        if let Some(error) = runtime_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            anyhow::bail!(error);
        }
        log::info!("WM Server shutting down");

        Ok(())
    }
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_main::main()
}
