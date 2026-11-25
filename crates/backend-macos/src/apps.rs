//! The programs that run, and who each is.
//!
//! macOS names a program by its bundle identifier (`com.apple.finder`):
//! what a Wayland window's app id is, and what the window rules match as
//! the window's application and class. The file it runs from is its
//! executable; the path of its bundle is its path.
//!
//! The programs are read from `NSWorkspace`, which keeps its list up to
//! date while the main thread's run loop runs. Only programs of the regular
//! kind (those with a Dock icon and windows of their own) are a window
//! manager's: an accessory program (a menu bar item) may show panels, which
//! are not tiled.

use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};

/// One program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub pid: i32,
    /// `com.apple.finder`; empty for a program that has no bundle.
    pub bundle_id: String,
    /// The name the system shows (`Finder`).
    pub name: String,
    /// The file it runs from, by its name alone (`Finder`).
    pub executable: String,
    /// The bundle's path (`/System/Library/CoreServices/Finder.app`), or the
    /// executable's for a program without one.
    pub path: String,
    /// A program with windows of its own (not a menu bar item or a helper).
    pub regular: bool,
}

impl App {
    /// What the window rules know the program as: its bundle identifier,
    /// or its name where it has none.
    pub fn app_id(&self) -> String {
        if self.bundle_id.is_empty() {
            gevrek_engine::contract::app_id_of_exe(&self.executable)
        } else {
            self.bundle_id.to_ascii_lowercase()
        }
    }
}

fn read(app: &NSRunningApplication) -> App {
    let text = |value: Option<objc2::rc::Retained<objc2_foundation::NSString>>| {
        value.map(|text| text.to_string()).unwrap_or_default()
    };
    let path_of = |url: Option<objc2::rc::Retained<objc2_foundation::NSURL>>| {
        text(url.and_then(|url| url.path()))
    };
    let executable = path_of(app.executableURL());
    let bundle = path_of(app.bundleURL());
    App {
        pid: app.processIdentifier(),
        bundle_id: text(app.bundleIdentifier()),
        name: text(app.localizedName()),
        executable: executable
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string(),
        path: if bundle.is_empty() {
            executable
        } else {
            bundle
        },
        regular: app.activationPolicy() == NSApplicationActivationPolicy::Regular,
    }
}

/// The program with this process id, if it runs.
pub fn app(pid: i32) -> Option<App> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    (!app.isTerminated()).then(|| read(&app))
}

/// Every program that runs, of every kind.
pub fn running() -> Vec<App> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|app| !app.isTerminated())
        .map(|app| read(&app))
        .collect()
}

/// The process id of the program in front (the one with the menu bar).
pub fn frontmost() -> Option<i32> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

/// This process's own id: its windows (none, so far) are never managed.
pub fn own_pid() -> i32 {
    std::process::id() as i32
}
