//! The few Core Foundation calls the macOS halves of this crate make,
//! declared by hand, and an owned reference that lets go when dropped.

#![allow(non_upper_case_globals, dead_code)]

use std::ffi::c_void;

pub type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFRunLoopDefaultMode: CFTypeRef;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
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
    ) -> CFTypeRef;
    pub fn CFStringGetLength(s: CFTypeRef) -> isize;
    pub fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
    pub fn CFStringGetCString(s: CFTypeRef, buf: *mut u8, size: isize, encoding: u32) -> bool;
    pub fn CFNumberGetValue(n: CFTypeRef, kind: isize, out: *mut c_void) -> bool;
    pub fn CFArrayGetCount(a: CFTypeRef) -> isize;
    pub fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
    pub fn CFArrayCreate(
        alloc: CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        callbacks: *const c_void,
    ) -> CFTypeRef;
    pub static kCFTypeArrayCallBacks: c_void;
    pub fn CFDictionaryGetValue(d: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    pub fn CFRunLoopGetCurrent() -> CFTypeRef;
    pub fn CFRunLoopAddSource(run_loop: CFTypeRef, source: CFTypeRef, mode: CFTypeRef);
    pub fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, after_source: bool) -> i32;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXIsProcessTrusted() -> bool;
    pub fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    pub fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    pub fn AXUIElementPerformAction(element: CFTypeRef, action: CFTypeRef) -> i32;
    pub fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

/// An attribute of an accessibility element (owned), if it has one.
///
/// # Safety
/// `element` is a valid AXUIElement.
pub unsafe fn attribute(element: CFTypeRef, name: &str) -> Option<Cf> {
    let name = Cf::string(name);
    let mut value: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(element, name.as_ptr(), &mut value) != 0 {
        return None;
    }
    Cf::owned(value)
}

/// The items of a CFArray, each retained.
///
/// # Safety
/// `array` is a valid CFArray.
pub unsafe fn items(array: CFTypeRef) -> Vec<Cf> {
    if CFGetTypeID(array) != CFArrayGetTypeID() {
        return Vec::new();
    }
    (0..CFArrayGetCount(array))
        .filter_map(|index| {
            let item = CFArrayGetValueAtIndex(array, index);
            (!item.is_null()).then(|| Cf(CFRetain(item)))
        })
        .collect()
}

const UTF8: u32 = 0x0800_0100;

/// One reference to a Core Foundation object, let go when dropped.
pub struct Cf(CFTypeRef);

// (Core Foundation's objects may be passed between threads.)
unsafe impl Send for Cf {}

impl Cf {
    /// Take over a reference the caller owns (from a `Create` or `Copy`
    /// call); `None` for a null pointer.
    ///
    /// # Safety
    /// `raw` is null or an owned reference to a Core Foundation object.
    pub unsafe fn owned(raw: CFTypeRef) -> Option<Self> {
        (!raw.is_null()).then_some(Self(raw))
    }

    pub fn string(text: &str) -> Self {
        // SAFETY: valid UTF-8 for the length given; an owned result.
        Self(unsafe {
            CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), text.len() as isize, UTF8, false)
        })
    }

    pub fn as_ptr(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        // SAFETY: self owns one reference.
        unsafe { CFRelease(self.0) }
    }
}

/// The text of a CFString (`None` for a null pointer or another type).
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
    let end = buffer.iter().position(|&byte| byte == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
}

/// The value of a CFNumber.
///
/// # Safety
/// `cf` is null or a valid Core Foundation object.
pub unsafe fn number_of(cf: CFTypeRef) -> Option<i64> {
    if cf.is_null() || CFGetTypeID(cf) != CFNumberGetTypeID() {
        return None;
    }
    let mut value = 0i64;
    // (4: kCFNumberSInt64Type.)
    CFNumberGetValue(cf, 4, &mut value as *mut i64 as *mut c_void).then_some(value)
}

/// The value of a CFBoolean.
///
/// # Safety
/// `cf` is null or a valid Core Foundation object.
pub unsafe fn bool_of(cf: CFTypeRef) -> Option<bool> {
    (!cf.is_null() && CFGetTypeID(cf) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(cf))
}

/// A value of a dictionary, by its key's text (lent, as the dictionary
/// holds it).
///
/// # Safety
/// `dictionary` is a valid CFDictionary.
pub unsafe fn value(dictionary: CFTypeRef, key: &str) -> CFTypeRef {
    let key = Cf::string(key);
    CFDictionaryGetValue(dictionary, key.as_ptr())
}
