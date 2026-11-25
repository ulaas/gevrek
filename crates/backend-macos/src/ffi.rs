//! The Core Foundation, Core Graphics and Accessibility calls this backend
//! makes, declared by hand: the Accessibility API (HIServices) has no
//! binding among the crates gevrek uses, and the few Core Foundation and
//! Core Graphics calls beside it are simpler declared next to it than
//! taken from a second binding.
//!
//! [`Cf`] owns one reference to a Core Foundation object and lets it go
//! when dropped; everything above this module holds those, never a bare
//! pointer.

#![allow(non_upper_case_globals, non_snake_case, dead_code)]

use std::ffi::c_void;

pub type CFTypeRef = *const c_void;
pub type CFStringRef = CFTypeRef;
pub type CFArrayRef = CFTypeRef;
pub type CFDictionaryRef = CFTypeRef;
pub type AXUIElementRef = CFTypeRef;
pub type AXObserverRef = CFTypeRef;
pub type AXValueRef = CFTypeRef;
pub type AXError = i32;
pub type CGWindowID = u32;
pub type CGDirectDisplayID = u32;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

pub type CGEventTapCallback = extern "C" fn(
    proxy: CFTypeRef,
    kind: u32,
    event: CFTypeRef,
    user_info: *mut c_void,
) -> CFTypeRef;

pub type AXObserverCallback = extern "C" fn(
    observer: AXObserverRef,
    element: AXUIElementRef,
    name: CFStringRef,
    refcon: *mut c_void,
);

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFBooleanTrue: CFTypeRef;
    pub static kCFBooleanFalse: CFTypeRef;
    pub static kCFRunLoopDefaultMode: CFStringRef;
    pub static kCFTypeDictionaryKeyCallBacks: c_void;
    pub static kCFTypeDictionaryValueCallBacks: c_void;
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> bool;
    pub fn CFGetTypeID(cf: CFTypeRef) -> usize;
    pub fn CFStringGetTypeID() -> usize;
    pub fn CFNumberGetTypeID() -> usize;
    pub fn CFBooleanGetTypeID() -> usize;
    pub fn CFArrayGetTypeID() -> usize;
    pub fn CFBooleanGetValue(b: CFTypeRef) -> bool;
    pub fn CFStringCreateWithBytes(
        alloc: CFTypeRef,
        bytes: *const u8,
        len: isize,
        encoding: u32,
        external: bool,
    ) -> CFStringRef;
    pub fn CFStringGetLength(s: CFStringRef) -> isize;
    pub fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
    pub fn CFStringGetCString(s: CFStringRef, buf: *mut u8, size: isize, encoding: u32) -> bool;
    pub fn CFNumberGetValue(n: CFTypeRef, kind: isize, out: *mut c_void) -> bool;
    pub fn CFArrayGetCount(a: CFArrayRef) -> isize;
    pub fn CFArrayGetValueAtIndex(a: CFArrayRef, i: isize) -> CFTypeRef;
    pub fn CFDictionaryGetValue(d: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    pub fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
    pub fn CFMachPortCreateRunLoopSource(
        alloc: CFTypeRef,
        port: CFTypeRef,
        order: isize,
    ) -> CFTypeRef;
    pub fn CFRunLoopGetCurrent() -> CFTypeRef;
    pub fn CFRunLoopAddSource(run_loop: CFTypeRef, source: CFTypeRef, mode: CFStringRef);
    pub fn CFRunLoopRemoveSource(run_loop: CFTypeRef, source: CFTypeRef, mode: CFStringRef);
    pub fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source: bool) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub static kCGWindowNumber: CFStringRef;
    pub static kCGWindowOwnerPID: CFStringRef;
    pub static kCGWindowOwnerName: CFStringRef;
    pub static kCGWindowName: CFStringRef;
    pub static kCGWindowLayer: CFStringRef;
    pub static kCGWindowBounds: CFStringRef;
    pub static kCGWindowIsOnscreen: CFStringRef;
    pub fn CGWindowListCopyWindowInfo(option: u32, relative_to: CGWindowID) -> CFArrayRef;
    pub fn CGRectMakeWithDictionaryRepresentation(d: CFDictionaryRef, out: *mut CGRect) -> bool;
    pub fn CGGetActiveDisplayList(max: u32, out: *mut CGDirectDisplayID, count: *mut u32) -> i32;
    pub fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    pub fn CGMainDisplayID() -> CGDirectDisplayID;
    pub fn CGDisplayIsBuiltin(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayVendorNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayModelNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplaySerialNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGEventCreate(source: CFTypeRef) -> CFTypeRef;
    pub fn CGEventGetLocation(event: CFTypeRef) -> CGPoint;
    pub fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
    pub fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallback,
        user_info: *mut c_void,
    ) -> CFTypeRef;
    pub fn CGEventTapEnable(tap: CFTypeRef, enable: bool);
    pub fn CGEventTapIsEnabled(tap: CFTypeRef) -> bool;
    pub fn CGEventGetIntegerValueField(event: CFTypeRef, field: u32) -> i64;
    pub fn CGEventGetFlags(event: CFTypeRef) -> u64;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub static kAXTrustedCheckOptionPrompt: CFStringRef;
    pub fn AXIsProcessTrusted() -> bool;
    pub fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    pub fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    pub fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFStringRef,
        settable: *mut u8,
    ) -> AXError;
    pub fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    pub fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> AXError;
    pub fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXError;
    pub fn AXValueCreate(kind: u32, value: *const c_void) -> AXValueRef;
    pub fn AXValueGetValue(value: AXValueRef, kind: u32, out: *mut c_void) -> bool;
    pub fn AXValueGetTypeID() -> usize;
    pub fn AXObserverCreate(
        pid: i32,
        callback: AXObserverCallback,
        out: *mut AXObserverRef,
    ) -> AXError;
    pub fn AXObserverAddNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        name: CFStringRef,
        refcon: *mut c_void,
    ) -> AXError;
    pub fn AXObserverRemoveNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        name: CFStringRef,
    ) -> AXError;
    pub fn AXObserverGetRunLoopSource(observer: AXObserverRef) -> CFTypeRef;
    /// Private, but what every window manager for macOS stands on: the
    /// window server's id of the window an Accessibility element is. There
    /// is no public way to tell which `CGWindowID` an element is.
    pub fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut CGWindowID) -> AXError;
}

pub const UTF8: u32 = 0x0800_0100;
pub const kAXValueCGPointType: u32 = 1;
pub const kAXValueCGSizeType: u32 = 2;
pub const kCFNumberSInt64Type: isize = 4;
pub const kCGWindowListOptionOnScreenOnly: u32 = 1 << 0;
pub const kCGWindowListOptionAll: u32 = 0;
pub const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;

/// One reference to a Core Foundation object, let go when dropped.
#[derive(Debug)]
pub struct Cf(CFTypeRef);

impl Cf {
    /// Take over a reference the caller owns (from a `Create` or `Copy`
    /// call). `None` for a null pointer.
    ///
    /// # Safety
    /// `raw` is null or an owned reference to a Core Foundation object.
    pub unsafe fn owned(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }

    /// Take a reference of one's own to an object one was only lent (an
    /// element of an array, a value of a dictionary).
    ///
    /// # Safety
    /// `raw` is null or a valid Core Foundation object.
    pub unsafe fn retained(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then(|| Self(CFRetain(raw)))
    }

    /// A string made of `text`.
    pub fn string(text: &str) -> Self {
        // SAFETY: the bytes are valid UTF-8 for the length given; the
        // result is an owned reference (a Create call).
        let raw = unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                text.as_ptr(),
                text.len() as isize,
                UTF8,
                false,
            )
        };
        Self(raw)
    }

    pub fn as_ptr(&self) -> CFTypeRef {
        self.0
    }

    pub fn to_string_lossy(&self) -> Option<String> {
        // SAFETY: self holds a valid object.
        unsafe { string_of(self.0) }
    }

    pub fn to_i64(&self) -> Option<i64> {
        // SAFETY: self holds a valid object.
        unsafe { number_of(self.0) }
    }

    pub fn to_bool(&self) -> Option<bool> {
        // SAFETY: self holds a valid object.
        unsafe { (CFGetTypeID(self.0) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(self.0)) }
    }

    /// The elements of an array, each with a reference of its own. Empty
    /// for anything that is not an array.
    pub fn array_items(&self) -> Vec<Cf> {
        // SAFETY: self holds a valid object; its type is checked before it
        // is read as an array, and each element is retained.
        unsafe {
            if CFGetTypeID(self.0) != CFArrayGetTypeID() {
                return Vec::new();
            }
            (0..CFArrayGetCount(self.0))
                .filter_map(|i| Cf::retained(CFArrayGetValueAtIndex(self.0, i)))
                .collect()
        }
    }
}

impl Clone for Cf {
    fn clone(&self) -> Self {
        // SAFETY: self holds a valid object; the clone owns its own reference.
        Self(unsafe { CFRetain(self.0) })
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: self owns exactly one reference.
        unsafe { CFRelease(self.0) }
    }
}

// Core Foundation objects may be retained, released and passed between
// threads; what may be done with them on which thread is the API's concern
// (the Accessibility API takes any thread).
unsafe impl Send for Cf {}

/// The text of a CFString; `None` for a null pointer or another type.
///
/// # Safety
/// `cf` is null or a valid Core Foundation object.
pub unsafe fn string_of(cf: CFTypeRef) -> Option<String> {
    if cf.is_null() || CFGetTypeID(cf) != CFStringGetTypeID() {
        return None;
    }
    let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(cf), UTF8) + 1;
    let mut buffer = vec![0u8; size as usize];
    if !CFStringGetCString(cf, buffer.as_mut_ptr(), size, UTF8) {
        return None;
    }
    let end = buffer
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

/// The value of a CFNumber; `None` for a null pointer or another type.
///
/// # Safety
/// `cf` is null or a valid Core Foundation object.
pub unsafe fn number_of(cf: CFTypeRef) -> Option<i64> {
    if cf.is_null() || CFGetTypeID(cf) != CFNumberGetTypeID() {
        return None;
    }
    let mut value = 0i64;
    CFNumberGetValue(
        cf,
        kCFNumberSInt64Type,
        &mut value as *mut i64 as *mut c_void,
    )
    .then_some(value)
}
