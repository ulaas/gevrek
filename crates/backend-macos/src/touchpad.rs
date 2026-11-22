//! The touchpad's contacts, for gevrek's own gestures (`desktop.gestures`).
//!
//! macOS tells a program of a touchpad's swipes only as far as they are
//! not its own (three or four fingers switch Spaces and show Mission
//! Control). The fingers themselves are told by MultitouchSupport: not one
//! of the public frameworks, but the one the system's own gestures and
//! every tool that recognises its own read. It is loaded when gestures are
//! on; where it is not there, there are no gestures, and nothing else
//! changes.
//!
//! Each frame of contacts goes, as on Windows, to the engine's recogniser
//! (`gevrek_engine::gesture`), in the loop: the frames come on
//! MultitouchSupport's own thread and wait for it.
//!
//! The system's own gestures go on: a profile with the fingers of one of
//! them (three, sideways) does both, unless that one is turned off in the
//! Trackpad settings.

use gevrek_engine::gesture::{TouchpadContact, TouchpadFrame};
use std::ffi::{c_void, CString};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// A point and its speed, as MultitouchSupport measures them.
#[repr(C)]
#[derive(Clone, Copy)]
struct Readout {
    x: f32,
    y: f32,
    speed_x: f32,
    speed_y: f32,
}

/// One finger on the touchpad (MultitouchSupport's `MTTouch`).
#[repr(C)]
#[derive(Clone, Copy)]
struct Finger {
    frame: i32,
    timestamp: f64,
    identifier: i32,
    state: i32,
    finger: i32,
    hand: i32,
    /// From 0 to 1 across the touchpad, from its left and from its bottom.
    normalized: Readout,
    size: f32,
    zero: i32,
    angle: f32,
    major_axis: f32,
    minor_axis: f32,
    millimetres: Readout,
    zeros: [i32; 2],
    density: f32,
}

/// A finger touches (it is coming down, or down): one that lifts or only
/// hovers is not counted.
fn touches(state: i32) -> bool {
    matches!(state, 3 | 4)
}

type Callback = extern "C" fn(*const c_void, *const Finger, i32, f64, i32) -> i32;

struct Api {
    create_list: unsafe extern "C" fn() -> *const c_void,
    register: unsafe extern "C" fn(*const c_void, Callback),
    unregister: unsafe extern "C" fn(*const c_void, Callback),
    start: unsafe extern "C" fn(*const c_void, i32),
    stop: unsafe extern "C" fn(*const c_void),
}

extern "C" {
    fn dlopen(path: *const i8, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const i8) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFRelease(object: *const c_void);
}

/// The framework's calls, looked up once.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| {
        let path = CString::new(
            "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport",
        )
        .ok()?;
        // SAFETY: the framework loaded and kept; its calls looked up by
        // name, with the signatures every tool that uses them has.
        unsafe {
            let handle = dlopen(path.as_ptr(), 1);
            if handle.is_null() {
                log::info!("no MultitouchSupport: no touchpad gestures");
                return None;
            }
            let call = |name: &std::ffi::CStr| {
                let found = dlsym(handle, name.as_ptr());
                (!found.is_null()).then_some(found)
            };
            Some(Api {
                create_list: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn() -> *const c_void,
                >(call(c"MTDeviceCreateList")?),
                register: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*const c_void, Callback),
                >(call(c"MTRegisterContactFrameCallback")?),
                unregister: std::mem::transmute::<
                    *mut c_void,
                    unsafe extern "C" fn(*const c_void, Callback),
                >(call(c"MTUnregisterContactFrameCallback")?),
                start: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_void, i32)>(
                    call(c"MTDeviceStart")?,
                ),
                stop: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_void)>(
                    call(c"MTDeviceStop")?,
                ),
            })
        }
    })
    .as_ref()
}

/// Where the frames go: the callback is given no place of its own.
static FRAMES: Mutex<Option<Sender<(TouchpadFrame, u64)>>> = Mutex::new(None);

/// The clock the frames and the recogniser's ticks are measured by.
pub fn clock_ms() -> u64 {
    static STARTED: OnceLock<Instant> = OnceLock::new();
    STARTED.get_or_init(Instant::now).elapsed().as_millis() as u64
}

extern "C" fn contacts(
    _device: *const c_void,
    fingers: *const Finger,
    count: i32,
    _at: f64,
    _frame: i32,
) -> i32 {
    let fingers: &[Finger] = if fingers.is_null() || count <= 0 {
        &[]
    } else {
        // SAFETY: as many fingers as the count says, for the call.
        unsafe { std::slice::from_raw_parts(fingers, count as usize) }
    };
    let frame = frame_of(fingers);
    if let Ok(sender) = FRAMES.lock() {
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send((frame, clock_ms()));
        }
    }
    0
}

/// The fingers that touch, as the recogniser takes them: across from the
/// left and down from the top, from 0 to 1 (as Windows' touchpads say).
fn frame_of(fingers: &[Finger]) -> TouchpadFrame {
    TouchpadFrame {
        contacts: fingers
            .iter()
            .filter(|finger| touches(finger.state))
            .map(|finger| TouchpadContact {
                id: finger.identifier as u32,
                x: finger.normalized.x.clamp(0.0, 1.0),
                y: (1.0 - finger.normalized.y).clamp(0.0, 1.0),
            })
            .collect(),
    }
}

/// The touchpads, told of while this lives.
pub struct Touchpad {
    api: &'static Api,
    /// The devices' list (owned), whose devices were started.
    list: *const c_void,
    frames: Receiver<(TouchpadFrame, u64)>,
}

impl Touchpad {
    /// Listen to every touchpad; `None` where there is none, or no
    /// MultitouchSupport.
    pub fn start() -> Option<Self> {
        let api = api()?;
        let (sender, frames) = channel();
        *FRAMES.lock().ok()? = Some(sender);
        // SAFETY: the framework's list of devices (owned, released when
        // this is dropped), each started with the callback.
        unsafe {
            let list = (api.create_list)();
            if list.is_null() || CFArrayGetCount(list) == 0 {
                if !list.is_null() {
                    CFRelease(list);
                }
                log::info!("no touchpad: no touchpad gestures");
                return None;
            }
            let count = CFArrayGetCount(list);
            for index in 0..count {
                let device = CFArrayGetValueAtIndex(list, index);
                (api.register)(device, contacts);
                (api.start)(device, 0);
            }
            log::info!("{count} touchpad(s) listened to for gestures");
            Some(Self { api, list, frames })
        }
    }

    /// The frames since this was last asked, each with when it came
    /// (`clock_ms`).
    pub fn frames(&self) -> Vec<(TouchpadFrame, u64)> {
        self.frames.try_iter().collect()
    }
}

impl Drop for Touchpad {
    fn drop(&mut self) {
        // SAFETY: the devices started in `start`, and its list.
        unsafe {
            for index in 0..CFArrayGetCount(self.list) {
                let device = CFArrayGetValueAtIndex(self.list, index);
                (self.api.unregister)(device, contacts);
                (self.api.stop)(device);
            }
            CFRelease(self.list);
        }
        if let Ok(mut sender) = FRAMES.lock() {
            *sender = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finger(identifier: i32, state: i32, x: f32, y: f32) -> Finger {
        let readout = Readout {
            x,
            y,
            speed_x: 0.0,
            speed_y: 0.0,
        };
        Finger {
            frame: 0,
            timestamp: 0.0,
            identifier,
            state,
            finger: 0,
            hand: 0,
            normalized: readout,
            size: 0.0,
            zero: 0,
            angle: 0.0,
            major_axis: 0.0,
            minor_axis: 0.0,
            millimetres: readout,
            zeros: [0; 2],
            density: 0.0,
        }
    }

    #[test]
    fn a_finger_is_as_the_framework_lays_it_out() {
        // (Its size and the place of its readout, as the framework writes
        // them: a mistake here reads every finger wrong.)
        assert_eq!(std::mem::size_of::<Finger>(), 96);
        assert_eq!(std::mem::offset_of!(Finger, normalized), 32);
        assert_eq!(std::mem::offset_of!(Finger, millimetres), 68);
    }

    #[test]
    fn only_fingers_that_touch_count_and_down_is_from_the_top() {
        let frame = frame_of(&[
            finger(1, 4, 0.25, 0.75),
            finger(2, 2, 0.5, 0.5),
            finger(3, 5, 0.9, 0.1),
        ]);
        assert_eq!(frame.contacts.len(), 1);
        assert_eq!(frame.contacts[0].id, 1);
        assert_eq!(frame.contacts[0].x, 0.25);
        assert_eq!(frame.contacts[0].y, 0.25);
    }
}
