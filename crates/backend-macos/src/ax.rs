//! A window or a program as the Accessibility API has it.
//!
//! Every call here is a message to the other program, which answers it on
//! its own main thread: a program that is busy answers late or not at all.
//! Each element is given a short timeout ([`TIMEOUT`]) so that one hung
//! program does not hold the window manager up for the system's six
//! seconds.
//!
//! Measured on a Finder window: setting a position takes about a quarter
//! of a millisecond, setting a size about 8 (the program lays its window
//! out again). A program keeps its windows at least as big as it says;
//! what it made of a size is read back, not assumed.

use crate::ffi::{self, CGPoint, CGRect, CGSize, CGWindowID, Cf};
use gevrek_model::geometry::Rect;
use std::ffi::c_void;
use std::fmt;

/// How long a program is waited for, per call.
pub const TIMEOUT: f32 = 0.25;

/// An error the Accessibility API answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxError(pub i32);

impl AxError {
    /// The program did not answer in time, or is busy.
    pub const CANNOT_COMPLETE: Self = Self(-25204);
    /// The element is gone (its window was closed).
    pub const INVALID_ELEMENT: Self = Self(-25202);
    /// This process may not use the Accessibility API.
    pub const API_DISABLED: Self = Self(-25211);
    /// The element has no value for the attribute.
    pub const NO_VALUE: Self = Self(-25212);

    pub(crate) fn check(code: i32) -> Result<(), Self> {
        if code == 0 {
            Ok(())
        } else {
            Err(Self(code))
        }
    }
}

impl fmt::Display for AxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.0 {
            -25200 => "failure",
            -25201 => "illegal argument",
            -25202 => "the element is gone",
            -25203 => "invalid observer",
            -25204 => "the program did not answer in time",
            -25205 => "attribute not supported",
            -25206 => "action not supported",
            -25207 => "notification not supported",
            -25208 => "not implemented",
            -25209 => "notification already registered",
            -25210 => "notification not registered",
            -25211 => "no Accessibility permission",
            -25212 => "no value",
            _ => "error",
        };
        write!(f, "{what} ({})", self.0)
    }
}

impl std::error::Error for AxError {}

/// Whether this process may use the Accessibility API. With `prompt`, the
/// system asks the user to allow it (once; System Settings, Privacy &
/// Security, Accessibility) where it may not yet.
///
/// The permission goes to the program the system holds responsible for
/// this process: an app bundle, or the terminal a bare binary was started
/// from.
pub fn trusted(prompt: bool) -> bool {
    if !prompt {
        // SAFETY: no arguments.
        return unsafe { ffi::AXIsProcessTrusted() };
    }
    // SAFETY: a one-entry dictionary of a CFString key and a CFBoolean
    // value, with the standard callbacks; released after the call.
    unsafe {
        let keys = [ffi::kAXTrustedCheckOptionPrompt];
        let values = [ffi::kCFBooleanTrue];
        let options = ffi::CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &ffi::kCFTypeDictionaryKeyCallBacks as *const c_void,
            &ffi::kCFTypeDictionaryValueCallBacks as *const c_void,
        );
        let Some(options) = Cf::owned(options) else {
            return ffi::AXIsProcessTrusted();
        };
        ffi::AXIsProcessTrustedWithOptions(options.as_ptr())
    }
}

/// One Accessibility element: a program or one of its windows.
#[derive(Debug, Clone)]
pub struct Element(Cf);

impl Element {
    /// The program with this process id.
    pub fn application(pid: i32) -> Self {
        // SAFETY: a Create call; the reference is owned.
        let raw = unsafe { ffi::AXUIElementCreateApplication(pid) };
        let element =
            Self(unsafe { Cf::owned(raw) }.expect("AXUIElementCreateApplication returned null"));
        element.set_timeout(TIMEOUT);
        element
    }

    /// Wrap an element the caller was lent (an observer's callback).
    ///
    /// # Safety
    /// `raw` is a valid AXUIElement.
    pub(crate) unsafe fn retained(raw: ffi::AXUIElementRef) -> Option<Self> {
        Cf::retained(raw).map(Self)
    }

    pub(crate) fn as_ptr(&self) -> ffi::AXUIElementRef {
        self.0.as_ptr()
    }

    fn set_timeout(&self, seconds: f32) {
        // SAFETY: a valid element.
        unsafe { ffi::AXUIElementSetMessagingTimeout(self.as_ptr(), seconds) };
    }

    /// The value of an attribute, owned.
    fn attribute(&self, name: &str) -> Result<Cf, AxError> {
        let name = Cf::string(name);
        let mut value: ffi::CFTypeRef = std::ptr::null();
        // SAFETY: a valid element and attribute name; a Copy call, so the
        // value is owned.
        AxError::check(unsafe {
            ffi::AXUIElementCopyAttributeValue(self.as_ptr(), name.as_ptr(), &mut value)
        })?;
        unsafe { Cf::owned(value) }.ok_or(AxError::NO_VALUE)
    }

    fn set_attribute(&self, name: &str, value: ffi::CFTypeRef) -> Result<(), AxError> {
        let name = Cf::string(name);
        // SAFETY: a valid element, attribute name and value.
        AxError::check(unsafe {
            ffi::AXUIElementSetAttributeValue(self.as_ptr(), name.as_ptr(), value)
        })
    }

    fn set_bool(&self, name: &str, value: bool) -> Result<(), AxError> {
        // SAFETY: the two CFBoolean constants are valid for the process.
        let value = unsafe {
            if value {
                ffi::kCFBooleanTrue
            } else {
                ffi::kCFBooleanFalse
            }
        };
        self.set_attribute(name, value)
    }

    fn perform(&self, action: &str) -> Result<(), AxError> {
        let action = Cf::string(action);
        // SAFETY: a valid element and action name.
        AxError::check(unsafe { ffi::AXUIElementPerformAction(self.as_ptr(), action.as_ptr()) })
    }

    /// The process the element belongs to.
    pub fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        // SAFETY: a valid element and a place for the answer.
        (unsafe { ffi::AXUIElementGetPid(self.as_ptr(), &mut pid) } == 0).then_some(pid)
    }

    /// The window server's id of the window this element is; `None` for an
    /// element that is no window, or one that is gone.
    pub fn window_id(&self) -> Option<CGWindowID> {
        let mut id = 0;
        // SAFETY: a valid element and a place for the answer.
        (unsafe { ffi::_AXUIElementGetWindow(self.as_ptr(), &mut id) } == 0 && id != 0)
            .then_some(id)
    }

    /// The windows of a program element, as the program lists them.
    pub fn windows(&self) -> Result<Vec<Element>, AxError> {
        self.elements("AXWindows")
    }

    /// The element an attribute names (`AXExtrasMenuBar`).
    pub fn element(&self, name: &str) -> Result<Element, AxError> {
        self.attribute(name).map(Element)
    }

    /// The elements an attribute names (`AXChildren`, `AXWindows`).
    pub fn elements(&self, name: &str) -> Result<Vec<Element>, AxError> {
        let list = self.attribute(name)?;
        Ok(list
            .array_items()
            .into_iter()
            .map(|item| {
                let element = Element(item);
                element.set_timeout(TIMEOUT);
                element
            })
            .collect())
    }

    /// The window of a program that has its keyboard focus.
    pub fn focused_window(&self) -> Result<Element, AxError> {
        self.attribute("AXFocusedWindow").map(Element)
    }

    pub fn string(&self, name: &str) -> Option<String> {
        self.attribute(name).ok()?.to_string_lossy()
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        self.attribute(name).ok()?.to_bool()
    }

    pub fn title(&self) -> String {
        self.string("AXTitle").unwrap_or_default()
    }

    /// `AXWindow`, `AXSheet`, ...
    pub fn role(&self) -> String {
        self.string("AXRole").unwrap_or_default()
    }

    /// `AXStandardWindow`, `AXDialog`, `AXFloatingWindow`, `AXSystemDialog`,
    /// ...: what kind of window it is, which says whether it is tiled.
    pub fn subrole(&self) -> String {
        self.string("AXSubrole").unwrap_or_default()
    }

    pub fn is_minimized(&self) -> bool {
        self.bool("AXMinimized").unwrap_or(false)
    }

    pub fn is_fullscreen(&self) -> bool {
        self.bool("AXFullScreen").unwrap_or(false)
    }

    /// Where the window is and how big, in points from the top-left of the
    /// main display.
    pub fn frame(&self) -> Result<Rect, AxError> {
        let mut rect = CGRect::default();
        let position = self.attribute("AXPosition")?;
        let size = self.attribute("AXSize")?;
        // SAFETY: AXPosition holds an AXValue of a CGPoint, AXSize one of a
        // CGSize; each is written into a value of that type.
        unsafe {
            ffi::AXValueGetValue(
                position.as_ptr(),
                ffi::kAXValueCGPointType,
                &mut rect.origin as *mut CGPoint as *mut c_void,
            );
            ffi::AXValueGetValue(
                size.as_ptr(),
                ffi::kAXValueCGSizeType,
                &mut rect.size as *mut CGSize as *mut c_void,
            );
        }
        Ok(rect_of(rect))
    }

    pub fn set_position(&self, x: i32, y: i32) -> Result<(), AxError> {
        let point = CGPoint {
            x: f64::from(x),
            y: f64::from(y),
        };
        // SAFETY: an AXValue made of a CGPoint, owned (a Create call).
        let value = unsafe {
            Cf::owned(ffi::AXValueCreate(
                ffi::kAXValueCGPointType,
                &point as *const CGPoint as *const c_void,
            ))
        }
        .ok_or(AxError(-25200))?;
        self.set_attribute("AXPosition", value.as_ptr())
    }

    pub fn set_size(&self, width: i32, height: i32) -> Result<(), AxError> {
        let size = CGSize {
            width: f64::from(width),
            height: f64::from(height),
        };
        // SAFETY: an AXValue made of a CGSize, owned (a Create call).
        let value = unsafe {
            Cf::owned(ffi::AXValueCreate(
                ffi::kAXValueCGSizeType,
                &size as *const CGSize as *const c_void,
            ))
        }
        .ok_or(AxError(-25200))?;
        self.set_attribute("AXSize", value.as_ptr())
    }

    /// Place the window and give it its size: the position first, so that
    /// the size is measured against the display it is going to; then the
    /// size; then the position again, since a program that would not take
    /// the size may have moved the window to keep it on its display.
    /// Returns where the window stands afterwards, which is what the
    /// program made of it (it keeps its smallest size).
    ///
    /// A window whose size cannot be changed (Calculator's: the size is
    /// refused with a failure) is still placed: only the position failing
    /// is an error.
    pub fn set_frame(&self, rect: Rect) -> Result<Rect, AxError> {
        self.set_position(rect.x, rect.y)?;
        if let Err(error) = self.set_size(rect.width, rect.height) {
            log::debug!("size {}x{} refused: {error}", rect.width, rect.height);
        }
        self.set_position(rect.x, rect.y)?;
        self.frame()
    }

    /// Whether the window's size can be changed at all.
    pub fn is_resizable(&self) -> bool {
        let name = Cf::string("AXSize");
        let mut settable = 0u8;
        // SAFETY: a valid element and attribute name, and a place for the
        // answer.
        let code = unsafe {
            ffi::AXUIElementIsAttributeSettable(self.as_ptr(), name.as_ptr(), &mut settable)
        };
        code == 0 && settable != 0
    }

    /// Put the window above the program's others.
    pub fn raise(&self) -> Result<(), AxError> {
        self.perform("AXRaise")
    }

    pub fn set_minimized(&self, minimized: bool) -> Result<(), AxError> {
        self.set_bool("AXMinimized", minimized)
    }

    /// Native full screen: the window gets a Space of its own.
    pub fn set_fullscreen(&self, fullscreen: bool) -> Result<(), AxError> {
        self.set_bool("AXFullScreen", fullscreen)
    }

    /// Ask the window to close, as its close button does (a program may
    /// ask first whether to save).
    pub fn close(&self) -> Result<(), AxError> {
        self.attribute("AXCloseButton")
            .map(Element)?
            .perform("AXPress")
    }

    /// Give the window the keyboard: make its program the frontmost one,
    /// the window its main window, and raise it.
    pub fn focus(&self) -> Result<(), AxError> {
        let pid = self.pid().ok_or(AxError::INVALID_ELEMENT)?;
        Element::application(pid).set_bool("AXFrontmost", true)?;
        self.set_bool("AXMain", true)?;
        self.raise()
    }

    /// Whether two elements are the same one.
    pub fn same_as(&self, other: &Element) -> bool {
        // SAFETY: two valid objects.
        unsafe { ffi::CFEqual(self.as_ptr(), other.as_ptr()) }
    }
}

/// The window server's rectangle as the engine's, rounded to whole points.
pub(crate) fn rect_of(rect: CGRect) -> Rect {
    Rect::new(
        rect.origin.x.round() as i32,
        rect.origin.y.round() as i32,
        rect.size.width.round() as i32,
        rect.size.height.round() as i32,
    )
}
