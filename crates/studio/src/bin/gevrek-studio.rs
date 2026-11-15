//! gevrek-studio: the settings of gevrek in a window.
//!
//!     gevrek-studio                    the running window manager
//!     gevrek-studio --address ...      another one (an instance, a machine)
//!     gevrek-studio --file config.json a file, with no window manager
//!     gevrek-studio --page appearance  open on a page

// A program with a window, not one for a terminal: started from the
// launcher or the Start menu, Windows opens no terminal beside it. (What
// it says still goes to the terminal it was started from: `main`.)
#![cfg_attr(windows, windows_subsystem = "windows")]

use clap::Parser;
use gevrek_studio::app::{Options, Source, Step, Studio};
use gevrek_studio::pages::Page;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gevrek-studio", about = "The settings of gevrek", version)]
struct Args {
    /// The window manager to talk to (default: the one of this session)
    #[arg(long)]
    address: Option<String>,
    /// Edit this file instead of a running window manager's configuration
    #[arg(long)]
    file: Option<PathBuf>,
    /// The page to open on (overview, displays, appearance, bindings, ...)
    #[arg(long)]
    page: Option<String>,
    /// Steps to take without a person, for checks:
    /// "set /desktop/bar/height 44; apply; quit" (also: undo, live on|off,
    /// page NAME, wait MILLISECONDS)
    #[arg(long)]
    script: Option<String>,
    /// Show only the list of the window manager's shortcuts (what the
    /// window manager opens on Super+F1)
    #[arg(long)]
    shortcuts: bool,
    /// Be a sample window of this name and nothing else (what the
    /// sandbox opens to have something to arrange)
    #[arg(long, value_name = "NAME")]
    sample: Option<String>,
    /// Be a practice window of the tutorial of this name and nothing
    /// else (what the tutorial opens to practise on)
    #[arg(long, value_name = "NAME")]
    practice: Option<String>,
    /// Show the welcome: what a person sees the first time gevrek runs
    #[arg(long)]
    welcome: bool,
}

fn main() -> anyhow::Result<()> {
    // Started from a terminal, what the program says is said there (its
    // help, its log); started with its output sent elsewhere (a check, a
    // file), it goes there as before.
    #[cfg(windows)]
    gevrek_system::every_key::say_to_the_terminal_it_was_started_from();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    if let Some(name) = &args.sample {
        return gevrek_studio::sample::run(name);
    }
    if let Some(name) = &args.practice {
        return gevrek_studio::sample::practice(name);
    }
    if args.shortcuts || args.welcome {
        let address = args
            .address
            .unwrap_or_else(|| gevrek_protocol::DEFAULT_WM_ENDPOINT.to_string());
        let appearance = gevrek_model::paths::config_file()
            .parent()
            .map(|folder| folder.join("studio-appearance"))
            .unwrap_or_else(|| PathBuf::from("studio-appearance"));
        if args.welcome {
            if gevrek_studio::welcome::run(address, &appearance)? {
                std::process::exit(gevrek_tutorial::TOUR_ASKED);
            }
            return Ok(());
        }
        return gevrek_studio::shortcuts::run(address, &appearance);
    }
    // `gevrek stop` of another instance must not take this one for its own.
    gevrek_model::paths::claim_process("editor");
    let page = match args.page.as_deref() {
        None => Page::Overview,
        Some(key) => Page::from_key(key).ok_or_else(|| {
            let known: Vec<&str> = Page::ALL.iter().map(|page| page.key()).collect();
            anyhow::anyhow!("no page {key}; there are: {}", known.join(", "))
        })?,
    };
    let source = match args.file {
        Some(path) => Source::File(path),
        None => Source::WindowManager(
            args.address
                .unwrap_or_else(|| gevrek_protocol::DEFAULT_WM_ENDPOINT.to_string()),
        ),
    };
    let appearance = gevrek_model::paths::config_file()
        .parent()
        .map(|folder| folder.join("studio-appearance"))
        .unwrap_or_else(|| PathBuf::from("studio-appearance"));
    let script = match args.script.as_deref() {
        Some(text) => {
            Step::parse_all(text).map_err(|error| anyhow::anyhow!("--script: {error}"))?
        }
        None => Vec::new(),
    };
    let options = Options {
        source,
        page,
        appearance,
        script,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id("gevrek-studio")
        .with_title("gevrek studio")
        // (Its top bar is its title bar: moved by it, with the window's
        // buttons at its end, made larger and smaller by its edges.)
        .with_decorations(false)
        .with_inner_size([1280.0, 860.0])
        .with_min_inner_size([760.0, 480.0]);
    // gevrek's icon on the window (where a window has one of its own: on
    // Wayland the desktop takes it from the application's entry).
    if let Some((rgba, width, height)) = gevrek_ui::brand::window_icon() {
        viewport = viewport.with_icon(egui::IconData {
            rgba,
            width,
            height,
        });
    }
    eframe::run_native(
        "gevrek studio",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |creation| Ok(Box::new(Studio::new(&creation.egui_ctx, options)))),
    )
    .map_err(|error| anyhow::anyhow!("the window could not be opened: {error}"))
}
