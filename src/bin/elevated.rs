//! gevrek-elevated: the helper that moves, shows and closes the windows
//! of programs that run as an administrator, for a window manager that
//! does not (`gevrek_win32::elevated_helper` says what it is and what it
//! may be asked).
//!
//! It is started as an administrator (by its task, or with Windows' own
//! question), serves the one window manager that lies beside it, and ends
//! when that one goes, or when none came.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use gevrek_win32::elevated_helper::{pipe_name, server};
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // `--instance <name>`: which gevrek it is for (the default: main).
    let mut arguments = std::env::args().skip(1);
    // `--for <person>`: who it is for, where it runs as another (an
    // administrator's name was given to Windows' question).
    // `--wm <file>`: the window manager it serves, where that does not
    // lie beside it. `--install <person>` / `--remove`: register the task
    // that starts it without a question, or take it away, and end.
    let mut instance = gevrek_model::paths::instance();
    let mut also_for = None;
    let mut window_manager: Option<std::path::PathBuf> = None;
    let mut install = None;
    let mut remove = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--instance" => {
                if let Some(named) = arguments.next() {
                    instance = named;
                }
            }
            "--for" => also_for = arguments.next(),
            "--wm" => window_manager = arguments.next().map(Into::into),
            "--install" => install = arguments.next(),
            "--remove" => remove = true,
            _ => {}
        }
    }
    if let Some(person) = install {
        let served = window_manager.clone().unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("gevrek-wm.exe")
        });
        let done = gevrek_win32::elevated::install_helper_task(&instance, &served, &person);
        if let Err(error) = &done {
            log::error!("the task was not registered: {error:#}");
        }
        std::process::exit(i32::from(done.is_err()));
    }
    if remove {
        let done = gevrek_win32::elevated::remove_helper_task(&instance);
        if let Err(error) = &done {
            log::error!("the task was not removed: {error:#}");
        }
        std::process::exit(i32::from(done.is_err()));
    }
    if !gevrek_win32::elevated::own_elevated() {
        log::error!("gevrek-elevated was started without an administrator's rights and can do nothing: start it with `gevrek elevated start`");
        std::process::exit(2);
    }
    // A helper nobody comes to does not stay for good.
    let came = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let waited_for = std::sync::Arc::clone(&came);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(120));
        if !waited_for.load(std::sync::atomic::Ordering::Acquire) {
            log::info!("no window manager came: the helper ends");
            std::process::exit(0);
        }
    });
    let pipe = pipe_name(&instance);
    log::info!("the helper for elevated windows waits on {pipe}");
    let served = server::serve_once(
        &pipe,
        server::Serve {
            may_ask: |process| {
                came.store(true, std::sync::atomic::Ordering::Release);
                server::is_window_manager(process, window_manager.as_deref())
            },
            hooks: true,
            also_for,
        },
    );
    if let Err(error) = served {
        log::error!("{error:#}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("gevrek-elevated is for Windows: elsewhere no window is beyond gevrek's reach.");
    std::process::exit(2);
}
