//! What happens to a program's windows, from the Accessibility API.
//!
//! One `Observer` watches one program. What concerns the program as a
//! whole (a window opened, the focus went to another of its windows, it
//! became the active program, it was hidden) is asked of the program's
//! element; what concerns one window (it moved, was resized, minimized,
//! retitled, closed) of the window's element, once it is known
//! ([`Observer::watch_window`]). Asking for one on both gives it twice.
//!
//! Each registration carries what it is for: the window's id is known from
//! the registration, so that a window that is gone (whose element no
//! longer says which window it was) is still named.
//!
//! The events come on the thread whose run loop the observer was added to,
//! while that run loop runs: the window manager's main thread. An observer
//! stays on that thread (it is not `Send`). The window manager's own moves
//! come back as events too, as on Windows: the engine tells them from the
//! user's (`Desktop::is_animation_event_suppressed`).

use crate::ax::{AxError, Element};
use crate::ffi::{self, CGWindowID, Cf};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::mpsc::Sender;

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxEventKind {
    WindowCreated,
    /// A window was closed (or any element the observer was told of went).
    Destroyed,
    FocusedWindowChanged,
    Moved,
    Resized,
    Minimized,
    Unminimized,
    TitleChanged,
    /// The program became the active one.
    Activated,
    Deactivated,
    /// The program was hidden (all its windows), or shown again.
    Hidden,
    Shown,
    Other(String),
}

impl AxEventKind {
    fn of(name: &str) -> Self {
        match name {
            "AXWindowCreated" => Self::WindowCreated,
            "AXUIElementDestroyed" => Self::Destroyed,
            "AXFocusedWindowChanged" => Self::FocusedWindowChanged,
            "AXWindowMoved" => Self::Moved,
            "AXWindowResized" => Self::Resized,
            "AXWindowMiniaturized" => Self::Minimized,
            "AXWindowDeminiaturized" => Self::Unminimized,
            "AXTitleChanged" => Self::TitleChanged,
            "AXApplicationActivated" => Self::Activated,
            "AXApplicationDeactivated" => Self::Deactivated,
            "AXApplicationHidden" => Self::Hidden,
            "AXApplicationShown" => Self::Shown,
            other => Self::Other(other.to_string()),
        }
    }
}

/// One thing that happened to a program or one of its windows.
#[derive(Debug, Clone)]
pub struct AxEvent {
    pub pid: i32,
    pub kind: AxEventKind,
    /// The window it happened to, where it is one.
    pub window: Option<CGWindowID>,
    /// The element it happened to (for a new window: the window, to be
    /// watched).
    pub element: Option<Element>,
}

/// What is asked of a program's element.
pub const PROGRAM_NOTIFICATIONS: &[&str] = &[
    "AXWindowCreated",
    "AXFocusedWindowChanged",
    "AXApplicationActivated",
    "AXApplicationDeactivated",
    "AXApplicationHidden",
    "AXApplicationShown",
];

/// What is asked of each window's element.
pub const WINDOW_NOTIFICATIONS: &[&str] = &[
    "AXUIElementDestroyed",
    "AXWindowMoved",
    "AXWindowResized",
    "AXWindowMiniaturized",
    "AXWindowDeminiaturized",
    "AXTitleChanged",
];

/// What one registration is for: where its events go, and the window it
/// is about (none for the program's own).
struct Registration {
    pid: i32,
    sender: Sender<AxEvent>,
    window: Option<CGWindowID>,
}

/// One program, watched.
pub struct Observer {
    pid: i32,
    observer: Cf,
    run_loop: ffi::CFTypeRef,
    source: ffi::CFTypeRef,
    sender: Sender<AxEvent>,
    program: Element,
    /// Boxed: the observer holds a pointer to each while it is registered.
    own: Box<Registration>,
    windows: Vec<(CGWindowID, Element, Box<Registration>)>,
    /// Stays on the thread whose run loop it is on.
    _thread: PhantomData<*const ()>,
}

impl Observer {
    /// Watch the program with this process id, on the current thread's run
    /// loop. Its windows are watched one by one with `watch_window`.
    pub fn new(pid: i32, sender: Sender<AxEvent>) -> Result<Self, AxError> {
        let mut raw: ffi::AXObserverRef = std::ptr::null();
        // SAFETY: a valid callback and a place for the observer, which is
        // owned (a Create call).
        AxError::check(unsafe { ffi::AXObserverCreate(pid, callback, &mut raw) })?;
        let observer = unsafe { Cf::owned(raw) }.ok_or(AxError(-25200))?;
        // SAFETY: the run loop source is the observer's (lent, alive as
        // long as it is); the current run loop is this thread's.
        let (run_loop, source) = unsafe {
            let run_loop = ffi::CFRunLoopGetCurrent();
            let source = ffi::AXObserverGetRunLoopSource(observer.as_ptr());
            ffi::CFRunLoopAddSource(run_loop, source, ffi::kCFRunLoopDefaultMode);
            (run_loop, source)
        };
        let program = Element::application(pid);
        let own = Box::new(Registration {
            pid,
            sender: sender.clone(),
            window: None,
        });
        let this = Self {
            pid,
            observer,
            run_loop,
            source,
            sender,
            program,
            own,
            windows: Vec::new(),
            _thread: PhantomData,
        };
        let refcon = &*this.own as *const Registration as *mut c_void;
        let program = this.program.clone();
        let added = this.add(&program, PROGRAM_NOTIFICATIONS, refcon);
        if added == 0 {
            // A program that takes none of them (one that has no windows of
            // its own, or does not answer) is not watched.
            return Err(AxError(-25207));
        }
        Ok(this)
    }

    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Ask for each notification on the element; how many it took.
    fn add(&self, element: &Element, names: &[&str], refcon: *mut c_void) -> usize {
        // A program that does not answer in time is not asked for the
        // rest: each would wait as long again, on the thread that hears
        // the keys. (-25204: it could not be completed.)
        let mut unanswered = false;
        names
            .iter()
            .filter(|name| {
                if unanswered {
                    return false;
                }
                let name = Cf::string(name);
                // SAFETY: a valid observer, element and name; refcon points
                // into a box this observer keeps until the registration is
                // removed or the observer is dropped.
                let code = unsafe {
                    ffi::AXObserverAddNotification(
                        self.observer.as_ptr(),
                        element.as_ptr(),
                        name.as_ptr(),
                        refcon,
                    )
                };
                unanswered = code == -25204;
                // (Already registered counts as taken.)
                code == 0 || code == -25209
            })
            .count()
    }

    /// Watch one window of the program. Nothing happens for a window that
    /// is watched already, or an element that is no window.
    pub fn watch_window(&mut self, element: &Element) -> Result<CGWindowID, AxError> {
        let id = element.window_id().ok_or(AxError::INVALID_ELEMENT)?;
        if self.windows.iter().any(|(watched, ..)| *watched == id) {
            return Ok(id);
        }
        let registration = Box::new(Registration {
            pid: self.pid,
            sender: self.sender.clone(),
            window: Some(id),
        });
        let refcon = &*registration as *const Registration as *mut c_void;
        if self.add(element, WINDOW_NOTIFICATIONS, refcon) == 0 {
            return Err(AxError(-25207));
        }
        self.windows.push((id, element.clone(), registration));
        Ok(id)
    }

    /// Stop watching a window (it was closed, or is no longer gevrek's).
    pub fn unwatch_window(&mut self, id: CGWindowID) {
        let Some(at) = self.windows.iter().position(|(watched, ..)| *watched == id) else {
            return;
        };
        let (_, element, registration) = self.windows.remove(at);
        self.remove(&element, WINDOW_NOTIFICATIONS);
        drop(registration);
    }

    fn remove(&self, element: &Element, names: &[&str]) {
        for name in names {
            let name = Cf::string(name);
            // SAFETY: a valid observer, element and name. (An element that
            // is gone answers with an error, which changes nothing.)
            unsafe {
                ffi::AXObserverRemoveNotification(
                    self.observer.as_ptr(),
                    element.as_ptr(),
                    name.as_ptr(),
                )
            };
        }
    }

    /// The windows watched.
    pub fn watched(&self) -> impl Iterator<Item = CGWindowID> + '_ {
        self.windows.iter().map(|(id, ..)| *id)
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        for (_, element, _) in &self.windows {
            self.remove(element, WINDOW_NOTIFICATIONS);
        }
        self.remove(&self.program, PROGRAM_NOTIFICATIONS);
        // SAFETY: the source was added to this run loop in `new`, on this
        // thread (the observer is not Send). After it is removed no
        // callback can come, and the registrations may go.
        unsafe {
            ffi::CFRunLoopRemoveSource(self.run_loop, self.source, ffi::kCFRunLoopDefaultMode)
        };
    }
}

extern "C" fn callback(
    _observer: ffi::AXObserverRef,
    element: ffi::AXUIElementRef,
    name: ffi::CFStringRef,
    refcon: *mut c_void,
) {
    if refcon.is_null() {
        return;
    }
    // SAFETY: refcon is the registration the notification was asked with,
    // which the observer keeps while it can be called (see `Drop`).
    let registration = unsafe { &*(refcon as *const Registration) };
    // SAFETY: the name and element are lent for the call.
    let kind = AxEventKind::of(&unsafe { ffi::string_of(name) }.unwrap_or_default());
    let element = unsafe { Element::retained(element) };
    let window = registration
        .window
        .or_else(|| element.as_ref().and_then(Element::window_id));
    // A receiver that is gone: the window manager is stopping.
    let _ = registration.sender.send(AxEvent {
        pid: registration.pid,
        kind,
        window,
        element,
    });
}

/// Run the current thread's run loop for up to `seconds`, handing out the
/// events that come meanwhile.
pub fn run_for(seconds: f64) {
    // SAFETY: the default mode constant; this thread's run loop.
    unsafe { ffi::CFRunLoopRunInMode(ffi::kCFRunLoopDefaultMode, seconds, false) };
}
