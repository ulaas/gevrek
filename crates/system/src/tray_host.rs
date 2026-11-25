//! The icons programs give the taskbar (its "notification area"), for a
//! panel of gevrek's own.
//!
//! A program tells the taskbar of its icon by a message to the window of
//! the class `Shell_TrayWnd` that it finds first. The host makes a window
//! of that class, of no size and never shown, and keeps it in front of the
//! real one: it hears what is told, keeps its own list of the icons, and
//! passes every message on to the real taskbar unchanged, with the real
//! taskbar's answer as its own. So the taskbar's own tray stays as it would
//! be without the host, and when the host ends nothing is lost.
//!
//! Icons that were there before the host began are learned by asking as a
//! taskbar that starts does (the `TaskbarCreated` call every program with
//! an icon listens to): they are told again, to the host and through it to
//! the real taskbar, which knows them already.
//!
//! A press on an icon is told to its program as the taskbar tells it; the
//! menu is the program's own, opened where the pointer is.
//!
//! Windows only; on Linux the shell speaks the tray's protocol itself.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::icons::Picture;

/// An icon as its program gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    /// What tells it from every other: its program's window and the
    /// number the program gave it.
    pub key: String,
    /// What is said when the pointer rests on it.
    pub tip: String,
    /// What it is known by from one day to the next: its program's file
    /// name and the number the program gave it (`onedrive.exe:1`). The
    /// key is another each time the program starts; this is not.
    pub name: String,
    pub picture: Option<std::sync::Arc<Picture>>,
    /// Another number when the picture is another one.
    pub stamp: u64,
}

/// What was done to an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// The main button: what the program does on a click.
    Main,
    /// The other button: the program's menu.
    Menu,
}

/// What a program told the taskbar, read from the message as it is
/// packed: the same on every machine, handles as four bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Told {
    /// 0 add, 1 change, 2 take away, 4 which way presses are told.
    what: u32,
    window: u32,
    id: u32,
    flags: u32,
    callback: u32,
    icon: u32,
    tip: String,
    state: u32,
    state_mask: u32,
    version: u32,
}

/// The message's mark (`SHELLTRAYDATA`).
const MARK: u32 = 0x3475_3423;

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|four| u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
}

/// Read what was told. `None` for what is no telling about an icon.
fn told(bytes: &[u8]) -> Option<Told> {
    if word(bytes, 0)? != MARK {
        return None;
    }
    // The icon's part begins after the mark and the word for what is told.
    let icon = bytes.get(8..)?;
    let tip: Vec<u16> = icon
        .get(24..)
        .unwrap_or_default()
        .as_chunks::<2>()
        .0
        .iter()
        .take(128)
        .map(|two| u16::from_le_bytes([two[0], two[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    Some(Told {
        what: word(bytes, 4)?,
        window: word(icon, 4)?,
        id: word(icon, 8)?,
        flags: word(icon, 12)?,
        callback: word(icon, 16)?,
        icon: word(icon, 20)?,
        tip: String::from_utf16_lossy(&tip),
        state: word(icon, 280).unwrap_or(0),
        state_mask: word(icon, 284).unwrap_or(0),
        version: word(icon, 800).unwrap_or(0),
    })
}

/// One icon as the host keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    window: u32,
    id: u32,
    callback: u32,
    icon: u32,
    tip: String,
    hidden: bool,
    /// How presses are told (4: the newer way).
    version: u32,
    stamp: u64,
}

/// Take in what was told. `true` when the list is another.
fn take(list: &mut Vec<Kept>, told: &Told, stamp: u64) -> bool {
    let at = list
        .iter()
        .position(|kept| (kept.window, kept.id) == (told.window, told.id));
    match (told.what, at) {
        // Taken away.
        (2, Some(at)) => {
            list.remove(at);
            log::debug!(
                "tray: icon {} of window {} is removed",
                told.id,
                told.window
            );
            true
        }
        // The way presses are told.
        (4, Some(at)) => {
            list[at].version = told.version;
            false
        }
        // Added, or changed (a change of one not known is an addition:
        // the host may have begun after it was added).
        (0 | 1, at) => {
            let mut kept = at.map(|at| list[at].clone()).unwrap_or(Kept {
                window: told.window,
                id: told.id,
                callback: 0,
                icon: 0,
                tip: String::new(),
                hidden: false,
                version: 0,
                stamp,
            });
            if told.flags & 1 != 0 {
                kept.callback = told.callback;
            }
            if told.flags & 2 != 0 && kept.icon != told.icon {
                kept.icon = told.icon;
                kept.stamp = stamp;
            }
            if told.flags & 4 != 0 {
                kept.tip = told.tip.clone();
            }
            if told.flags & 8 != 0 && told.state_mask & 1 != 0 {
                kept.hidden = told.state & 1 != 0;
            }
            match at {
                Some(at) if list[at] == kept => false,
                Some(at) => {
                    list[at] = kept;
                    true
                }
                None => {
                    log::debug!("tray: icon {} of window {} is added", told.id, told.window);
                    list.push(kept);
                    true
                }
            }
        }
        _ => false,
    }
}

/// What a picture is, from the same icon drawn on black and on white:
/// where both are the same the icon is solid, where they differ by all
/// there is nothing of it. (Icons come in several kinds; this reads them
/// all the same way.) Both are blue, green, red and a spare byte.
fn picture_from(black: &[u8], white: &[u8], side: u32) -> Option<Picture> {
    let count = (side * side) as usize * 4;
    if black.len() < count || white.len() < count {
        return None;
    }
    let mut pixels = Vec::with_capacity(count);
    for (on_black, on_white) in black[..count]
        .as_chunks::<4>()
        .0
        .iter()
        .zip(white.as_chunks::<4>().0)
    {
        let apart = (0..3)
            .map(|colour| on_white[colour].saturating_sub(on_black[colour]))
            .max()
            .unwrap_or(0);
        let solid = 255 - apart;
        let full = |colour: u8| -> u8 {
            if solid == 0 {
                0
            } else {
                ((u32::from(colour) * 255) / u32::from(solid)).min(255) as u8
            }
        };
        pixels.extend_from_slice(&[
            full(on_black[2]),
            full(on_black[1]),
            full(on_black[0]),
            solid,
        ]);
    }
    Some(Picture {
        width: side,
        height: side,
        pixels,
    })
}

#[cfg(windows)]
pub use self::windows::Host;

#[cfg(target_os = "macos")]
pub use self::mac::Host;

/// A picture taken off the menu bar made clear where it shows the bar
/// itself: the colour of its corners is the ground, a pixel as much clear
/// as it is near that colour (so an edge drawn soft stays soft), and the
/// colour of a pixel half clear is what it was before the ground showed
/// through.
pub fn without_ground(mut picture: crate::icons::Picture) -> crate::icons::Picture {
    let (width, height) = (picture.width as usize, picture.height as usize);
    if width == 0 || height == 0 || picture.pixels.len() < width * height * 4 {
        return picture;
    }
    let corner = |x: usize, y: usize| {
        let at = (y * width + x) * 4;
        [0, 1, 2].map(|part| f32::from(picture.pixels[at + part]))
    };
    let corners = [
        corner(0, 0),
        corner(width - 1, 0),
        corner(0, height - 1),
        corner(width - 1, height - 1),
    ];
    let ground = [0, 1, 2].map(|part| corners.iter().map(|colour| colour[part]).sum::<f32>() / 4.0);
    let distance_of = |pixel: &[u8]| {
        (0..3)
            .map(|part| (f32::from(pixel[part]) - ground[part]).abs())
            .fold(0.0, f32::max)
    };
    // The item's own colour is what is furthest from the ground (an item
    // of the menu bar is drawn in one colour, mostly): a pixel is as solid
    // as it is far along the way to it.
    let furthest = picture
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| distance_of(pixel))
        .fold(0.0, f32::max)
        .max(1.0);
    for pixel in picture.pixels.as_chunks_mut::<4>().0 {
        let colour = [0, 1, 2].map(|part| f32::from(pixel[part]));
        let distance = distance_of(pixel);
        let alpha = (distance / furthest).clamp(0.0, 1.0);
        for part in 0..3 {
            // What was laid over the ground with this much of it showing.
            let own = if alpha > 0.0 {
                (colour[part] - ground[part] * (1.0 - alpha)) / alpha
            } else {
                0.0
            };
            pixel[part] = own.round().clamp(0.0, 255.0) as u8;
        }
        pixel[3] = (alpha * 255.0).round() as u8;
    }
    picture
}

/// The menu bar's items of the other programs, as the shell's tray. macOS
/// has no tray to stand in for: each program's items are its own, in the
/// menu bar. They are read through the Accessibility API (each program's
/// `AXExtrasMenuBar`), pressed through it as a click on them would be, and
/// pictured from what the screen shows where they are (which needs the
/// Screen Recording permission; without it an item has no picture). The
/// system's own items (the Control Centre's, Spotlight's, the input
/// menu) are left to the menu bar. The items are looked at every few
/// seconds: nothing tells of them.
#[cfg(target_os = "macos")]
mod mac {
    use super::{Icon, Press};
    use crate::macos_cf::{self as cf, Cf};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{NSArray, NSString};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// How often the items are looked at.
    const LOOK: Duration = Duration::from_secs(3);

    /// A program that runs: its process, its bundle id and its name.
    struct Program {
        pid: i32,
        id: String,
        name: String,
    }

    fn programs() -> Vec<Program> {
        let text = |object: Option<Retained<NSString>>| {
            object.map(|text| text.to_string()).unwrap_or_default()
        };
        // SAFETY: the workspace's running applications, and their plain
        // properties.
        unsafe {
            let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            let running: Retained<NSArray<AnyObject>> = msg_send![&*workspace, runningApplications];
            running
                .iter()
                .map(|app| Program {
                    pid: msg_send![&*app, processIdentifier],
                    id: text(msg_send![&*app, bundleIdentifier]),
                    name: text(msg_send![&*app, localizedName]),
                })
                .collect()
        }
    }

    /// The system's own items, which the menu bar keeps.
    fn the_systems(id: &str) -> bool {
        id.starts_with("com.apple.")
    }

    /// A program's items of the menu bar (retained elements).
    fn items_of(pid: i32) -> Vec<Cf> {
        // SAFETY: the program's element, its extras menu bar and that
        // bar's children.
        unsafe {
            let Some(app) = Cf::owned(cf::AXUIElementCreateApplication(pid)) else {
                return Vec::new();
            };
            let Some(bar) = cf::attribute(app.as_ptr(), "AXExtrasMenuBar") else {
                return Vec::new();
            };
            cf::attribute(bar.as_ptr(), "AXChildren")
                .map(|children| cf::items(children.as_ptr()))
                .unwrap_or_default()
        }
    }

    /// Where an item is, in points from the top left of the main display.
    fn frame_of(item: &Cf) -> Option<(f64, f64, f64, f64)> {
        // SAFETY: AXValues of a point and a size, read into their kinds.
        unsafe {
            let mut at = [0f64; 2];
            let mut size = [0f64; 2];
            let position = cf::attribute(item.as_ptr(), "AXPosition")?;
            let extent = cf::attribute(item.as_ptr(), "AXSize")?;
            (cf::AXValueGetValue(position.as_ptr(), 1, at.as_mut_ptr().cast())
                && cf::AXValueGetValue(extent.as_ptr(), 2, size.as_mut_ptr().cast()))
            .then_some((at[0], at[1], size[0], size[1]))
        }
    }

    fn said(item: &Cf, name: &str) -> Option<String> {
        // SAFETY: a text attribute of a valid element.
        unsafe { cf::attribute(item.as_ptr(), name).and_then(|text| cf::string_of(text.as_ptr())) }
            .filter(|text| !text.trim().is_empty())
    }

    /// Every other program's item of the menu bar, as the tray's icons.
    fn icons() -> Vec<Icon> {
        let mut icons = Vec::new();
        for program in programs()
            .into_iter()
            .filter(|program| !the_systems(&program.id))
        {
            for (index, item) in items_of(program.pid).iter().enumerate() {
                let tip = ["AXTitle", "AXDescription", "AXHelp"]
                    .iter()
                    .find_map(|name| said(item, name))
                    .unwrap_or_else(|| program.name.clone());
                let picture = frame_of(item)
                    .and_then(|(x, y, width, height)| {
                        crate::pictures::of_screen_area(x, y, width, height)
                    })
                    .map(super::without_ground);
                let stamp = picture.as_ref().map_or(0, |picture| {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    picture.pixels.hash(&mut hasher);
                    hasher.finish()
                });
                icons.push(Icon {
                    key: format!("{}:{index}", program.pid),
                    tip,
                    name: format!(
                        "{}:{index}",
                        if program.id.is_empty() {
                            &program.name
                        } else {
                            &program.id
                        }
                    ),
                    picture: picture.map(Arc::new),
                    stamp,
                });
            }
        }
        icons
    }

    /// The menu bar's items, told to the shell when they change.
    pub struct Host {
        stop: Arc<AtomicBool>,
    }

    impl Host {
        pub fn start(changed: impl Fn(Vec<Icon>) + Send + 'static) -> Option<Self> {
            // SAFETY: a plain question.
            if !unsafe { cf::AXIsProcessTrusted() } {
                log::info!("the menu bar's items are read through the Accessibility permission, which the shell has not");
            }
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("menu bar items".to_string())
                .spawn(move || {
                    let mut told: Option<Vec<Icon>> = None;
                    while !stopped.load(Ordering::Relaxed) {
                        let now = icons();
                        if told.as_ref() != Some(&now) {
                            log::debug!("{} item(s) of other programs in the menu bar", now.len());
                            changed(now.clone());
                            told = Some(now);
                        }
                        std::thread::sleep(LOOK);
                    }
                })
                .ok()?;
            Some(Self { stop })
        }

        /// Press an item: its program does what a click on it does (most
        /// open their menu). `Menu` asks for the menu where the item has
        /// one of its own.
        pub fn press(&self, key: &str, press: Press) {
            let Some((pid, index)) = key.split_once(':').and_then(|(pid, index)| {
                Some((pid.parse::<i32>().ok()?, index.parse::<usize>().ok()?))
            }) else {
                return;
            };
            let Some(item) = items_of(pid).into_iter().nth(index) else {
                log::debug!("menu bar item {key} is gone");
                return;
            };
            let action = |name: &str| {
                let name = Cf::string(name);
                // SAFETY: an action of a valid element.
                unsafe { cf::AXUIElementPerformAction(item.as_ptr(), name.as_ptr()) == 0 }
            };
            let done = match press {
                Press::Menu => action("AXShowMenu") || action("AXPress"),
                Press::Main => action("AXPress"),
            };
            log::debug!(
                "menu bar item {key} pressed ({press:?}): {}",
                if done { "done" } else { "refused" }
            );
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::{picture_from, take, told, Icon, Kept, Press};
    use crate::icons::Picture;
    use std::sync::{Arc, Mutex};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    };
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
        DrawIconEx, FindWindowExW, GetCursorPos, GetMessageW, GetWindowThreadProcessId, IsWindow,
        PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SendMessageW,
        SendNotifyMessageW, SetForegroundWindow, SetTimer, SetWindowPos, DI_NORMAL, HICON,
        HWND_BROADCAST, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WM_CLOSE,
        WM_COPYDATA, WM_DESTROY, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    /// The side of an icon's picture, in pixels.
    const SIDE: i32 = 32;

    struct Listening {
        kept: Vec<Kept>,
        /// The pictures of the icons, by the icon's handle.
        pictures: Vec<(u32, Arc<Picture>)>,
        /// The file names of the programs, by their icon's window.
        programs: Vec<(u32, String)>,
        stamp: u64,
        changed: Box<dyn Fn(Vec<Icon>) + Send>,
    }

    /// One host for a program.
    static LISTENING: Mutex<Option<Listening>> = Mutex::new(None);

    /// The host, for as long as it is kept.
    pub struct Host {
        window: isize,
    }

    impl Host {
        /// Begin to hear what programs tell the taskbar of their icons.
        /// `changed` is called (on the host's own thread) with the icons
        /// each time they are others. `None` when Windows refuses the
        /// window, or a host is kept already.
        pub fn start(changed: impl Fn(Vec<Icon>) + Send + 'static) -> Option<Self> {
            {
                let mut listening = LISTENING.lock().unwrap_or_else(|error| error.into_inner());
                if listening.is_some() {
                    return None;
                }
                *listening = Some(Listening {
                    kept: Vec::new(),
                    pictures: Vec::new(),
                    programs: Vec::new(),
                    stamp: 0,
                    changed: Box::new(changed),
                });
            }
            let (ready, made) = std::sync::mpsc::channel::<isize>();
            let started = std::thread::Builder::new()
                .name("gevrek-tray-host".to_string())
                .spawn(move || serve(ready));
            let window = started.ok().and_then(|_| made.recv().ok()).unwrap_or(0);
            if window == 0 {
                *LISTENING.lock().unwrap_or_else(|error| error.into_inner()) = None;
                return None;
            }
            Some(Self { window })
        }

        /// Tell an icon's program that the icon was pressed.
        pub fn press(&self, key: &str, press: Press) {
            let kept = LISTENING
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .and_then(|listening| {
                    listening
                        .kept
                        .iter()
                        .find(|kept| key_of(kept) == key)
                        .cloned()
                });
            let Some(kept) = kept else {
                return;
            };
            let window = HWND(kept.window as usize as *mut _);
            let mut at = POINT::default();
            // SAFETY: plain calls; a window that is gone hears nothing.
            unsafe {
                let _ = GetCursorPos(&mut at);
                // The program opens a menu or a window of its own: it may.
                let mut process = 0u32;
                GetWindowThreadProcessId(window, Some(&mut process));
                let _ = AllowSetForegroundWindow(process);
                // (A menu the program opens closes on a press elsewhere
                // only when its window is the one in front.)
                if press == Press::Menu {
                    let _ = SetForegroundWindow(window);
                }
                let tell = |message: u32| {
                    // The newer way: where the pointer is, and the
                    // message with the icon's number; the older: the
                    // number, and the message.
                    let (first, second) = if kept.version >= 4 {
                        (
                            (at.x as u16 as usize) | ((at.y as u16 as usize) << 16),
                            (message as usize & 0xFFFF) | ((kept.id as usize & 0xFFFF) << 16),
                        )
                    } else {
                        (kept.id as usize, message as usize)
                    };
                    let _ = PostMessageW(
                        window,
                        kept.callback,
                        WPARAM(first),
                        LPARAM(second as isize),
                    );
                };
                match press {
                    // Button down and up; the newer way has a word for
                    // "chosen" besides.
                    Press::Main => {
                        tell(0x0201);
                        tell(0x0202);
                        if kept.version >= 4 {
                            tell(0x0400);
                        }
                    }
                    // The other button; the newer way asks for the menu
                    // by its own word.
                    Press::Menu => {
                        tell(0x0204);
                        tell(0x0205);
                        if kept.version >= 4 {
                            tell(0x007B);
                        }
                    }
                }
            }
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            // SAFETY: the window is the host's; its thread ends with it.
            unsafe {
                let _ = PostMessageW(HWND(self.window as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            *LISTENING.lock().unwrap_or_else(|error| error.into_inner()) = None;
        }
    }

    fn key_of(kept: &Kept) -> String {
        format!("{:x}:{}", kept.window, kept.id)
    }

    /// The file name of the program a window is of, in small letters
    /// (`onedrive.exe`); empty where Windows does not say.
    fn program_of(window: u32) -> String {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: the handle is closed below; the buffer is ours.
        unsafe {
            let mut process = 0u32;
            GetWindowThreadProcessId(HWND(window as usize as *mut _), Some(&mut process));
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) else {
                return String::new();
            };
            let mut path = [0u16; 512];
            let mut length = path.len() as u32;
            let named = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(path.as_mut_ptr()),
                &mut length,
            )
            .is_ok();
            let _ = CloseHandle(handle);
            if !named {
                return String::new();
            }
            let path = String::from_utf16_lossy(&path[..length as usize]);
            path.rsplit(['\\', '/'])
                .next()
                .unwrap_or_default()
                .to_lowercase()
        }
    }

    /// The icon's picture, drawn into memory.
    fn picture_of(icon: u32) -> Option<Picture> {
        let draw = |background: u8| -> Option<Vec<u8>> {
            // SAFETY: every handle made here is given back; the pixels
            // are read while the bitmap that owns them lives.
            unsafe {
                let screen = GetDC(None);
                let memory = CreateCompatibleDC(screen);
                ReleaseDC(None, screen);
                if memory.is_invalid() {
                    return None;
                }
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: SIDE,
                        biHeight: -SIDE,
                        biPlanes: 1,
                        biBitCount: 32,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
                let mut found = None;
                if let Ok(bitmap) =
                    CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, None, 0)
                {
                    if !bits.is_null() {
                        let pixels = std::slice::from_raw_parts_mut(
                            bits as *mut u8,
                            (SIDE * SIDE * 4) as usize,
                        );
                        pixels.fill(background);
                        let before = SelectObject(memory, bitmap);
                        let drawn = DrawIconEx(
                            memory,
                            0,
                            0,
                            HICON(icon as usize as *mut _),
                            SIDE,
                            SIDE,
                            0,
                            None,
                            DI_NORMAL,
                        )
                        .is_ok();
                        let _ = GdiFlush();
                        if drawn {
                            found = Some(pixels.to_vec());
                        }
                        SelectObject(memory, before);
                    }
                    let _ = DeleteObject(bitmap);
                }
                let _ = DeleteDC(memory);
                found
            }
        };
        picture_from(&draw(0)?, &draw(255)?, SIDE as u32)
    }

    /// Tell who listens what the icons are now.
    fn tell(listening: &mut Listening) {
        let mut icons = Vec::new();
        for kept in listening.kept.clone() {
            if kept.hidden {
                continue;
            }
            let picture = match listening
                .pictures
                .iter()
                .find(|(icon, _)| *icon == kept.icon)
            {
                Some((_, picture)) => Some(Arc::clone(picture)),
                None if kept.icon != 0 => picture_of(kept.icon).map(|picture| {
                    let picture = Arc::new(picture);
                    listening.pictures.push((kept.icon, Arc::clone(&picture)));
                    picture
                }),
                None => None,
            };
            let program = match listening
                .programs
                .iter()
                .find(|(window, _)| *window == kept.window)
            {
                Some((_, program)) => program.clone(),
                None => {
                    let program = program_of(kept.window);
                    listening.programs.push((kept.window, program.clone()));
                    program
                }
            };
            icons.push(Icon {
                key: key_of(&kept),
                name: format!("{program}:{}", kept.id),
                tip: kept.tip.clone(),
                picture,
                stamp: kept.stamp,
            });
        }
        // Pictures of icons nobody has any more are let go.
        let used: Vec<u32> = listening.kept.iter().map(|kept| kept.icon).collect();
        listening.pictures.retain(|(icon, _)| used.contains(icon));
        let windows: Vec<u32> = listening.kept.iter().map(|kept| kept.window).collect();
        listening
            .programs
            .retain(|(window, _)| windows.contains(window));
        (listening.changed)(icons);
    }

    /// The taskbar behind the host's window: the next of the class.
    fn real_taskbar(own: HWND) -> Option<HWND> {
        // SAFETY: a plain search among the top-level windows.
        unsafe { FindWindowExW(None, own, w!("Shell_TrayWnd"), PCWSTR::null()) }
            .ok()
            .filter(|window| !window.is_invalid())
    }

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_COPYDATA => {
                let data = &*(lparam.0 as *const COPYDATASTRUCT);
                // What is told about an icon (the first kind of message;
                // the others are for the taskbar alone).
                if data.dwData == 1 && !data.lpData.is_null() {
                    let bytes =
                        std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize);
                    if let Some(told) = told(bytes) {
                        if let Ok(mut listening) = LISTENING.try_lock() {
                            if let Some(listening) = listening.as_mut() {
                                listening.stamp += 1;
                                let stamp = listening.stamp;
                                if take(&mut listening.kept, &told, stamp) {
                                    tell(listening);
                                }
                            }
                        }
                    }
                }
                // Passed on as it came; the taskbar's answer is ours.
                match real_taskbar(window) {
                    Some(real) => SendMessageW(real, message, wparam, lparam),
                    None => LRESULT(1),
                }
            }
            WM_TIMER => {
                // In front of the real taskbar again (it comes forward
                // when it is used), and icons of windows that are gone
                // (a program that was ended tells nobody) are taken off.
                let _ = SetWindowPos(
                    window,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
                if let Ok(mut listening) = LISTENING.try_lock() {
                    if let Some(listening) = listening.as_mut() {
                        let before = listening.kept.len();
                        listening.kept.retain(|kept| {
                            IsWindow(HWND(kept.window as usize as *mut _)).as_bool()
                        });
                        if listening.kept.len() != before {
                            log::debug!(
                                "tray: {} icons are removed: their windows are gone",
                                before - listening.kept.len()
                            );
                            tell(listening);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(window);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            // Whatever else a program sends the taskbar is the real one's.
            other if other >= 0x0400 => match real_taskbar(window) {
                Some(real) => SendMessageW(real, message, wparam, lparam),
                None => DefWindowProcW(window, message, wparam, lparam),
            },
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    /// The host's thread: its window, and the messages for it.
    fn serve(ready: std::sync::mpsc::Sender<isize>) {
        // SAFETY: the class and the window are this thread's own.
        unsafe {
            let module = GetModuleHandleW(None).unwrap_or_default();
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module.into(),
                lpszClassName: w!("Shell_TrayWnd"),
                ..Default::default()
            };
            // (Registered already when a host was kept before.)
            let _ = RegisterClassW(&class);
            let window = match CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("Shell_TrayWnd"),
                w!("gevrek tray host"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                module,
                None,
            ) {
                Ok(window) => window,
                Err(error) => {
                    log::warn!("the tray's icons are not heard of: {error}");
                    let _ = ready.send(0);
                    return;
                }
            };
            let _ = SetWindowPos(
                window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
            SetTimer(window, 1, 5000, None);
            let _ = ready.send(window.0 as isize);
            // As a taskbar that starts: every program tells its icon again.
            let created = RegisterWindowMessageW(w!("TaskbarCreated"));
            if created != 0 {
                let _ = SendNotifyMessageW(HWND_BROADCAST, created, WPARAM(0), LPARAM(0));
            }
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                DispatchMessageW(&message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_menu_bars_ground_is_taken_out_of_an_items_picture() {
        let ground = [90u8, 110, 150, 255];
        let white = [255u8, 255, 255, 255];
        // Ground at both ends (the corners), the item's white between.
        let picture = crate::icons::Picture {
            width: 3,
            height: 1,
            pixels: [ground, white, ground].concat(),
        };
        let clear = super::without_ground(picture);
        assert_eq!(clear.pixels[3], 0);
        assert_eq!(&clear.pixels[4..8], &[255, 255, 255, 255]);
        assert_eq!(clear.pixels[11], 0);
        // A soft edge, half the white over the ground: half clear, and
        // white again.
        let half = [172u8, 182, 202, 255];
        let picture = crate::icons::Picture {
            width: 4,
            height: 1,
            pixels: [ground, white, half, ground].concat(),
        };
        let clear = super::without_ground(picture);
        assert!(
            (115..145).contains(&clear.pixels[11]),
            "{}",
            clear.pixels[11]
        );
        assert!(clear.pixels[8] > 245, "{}", clear.pixels[8]);
    }

    use super::*;

    /// A message as a program packs it.
    fn message(what: u32, window: u32, id: u32, flags: u32, icon: u32, tip: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; 8 + 956];
        let mut put =
            |at: usize, value: u32| bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(0, MARK);
        put(4, what);
        put(8, 956);
        put(8 + 4, window);
        put(8 + 8, id);
        put(8 + 12, flags);
        put(8 + 16, 0x8001);
        put(8 + 20, icon);
        put(8 + 800, 4);
        for (index, unit) in tip.encode_utf16().enumerate() {
            bytes[8 + 24 + index * 2..8 + 26 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn what_a_program_tells_the_taskbar_is_read() {
        let heard = told(&message(0, 0x1234, 7, 1 | 2 | 4, 0x99, "Sound: 40%")).expect("a telling");
        assert_eq!((heard.what, heard.window, heard.id), (0, 0x1234, 7));
        assert_eq!((heard.callback, heard.icon), (0x8001, 0x99));
        assert_eq!(heard.tip, "Sound: 40%");
        assert_eq!(heard.version, 4);
        // Something else, and something cut short.
        assert!(told(&[0u8; 64]).is_none());
        assert!(told(&message(0, 1, 1, 0, 0, "")[..20]).is_none());
        // An older, shorter message still has what counts.
        let short = &message(0, 0x55, 3, 4, 0, "old")[..8 + 152];
        let heard = told(short).expect("a telling");
        assert_eq!(
            (heard.window, heard.tip.as_str(), heard.version),
            (0x55, "old", 0)
        );
    }

    #[test]
    fn icons_are_added_changed_hidden_and_taken_away() {
        let mut list = Vec::new();
        let add = told(&message(0, 0x10, 1, 1 | 2 | 4, 0xA, "one")).unwrap();
        assert!(take(&mut list, &add, 1));
        // The same again: no news.
        assert!(!take(&mut list, &add, 2));
        // Another tip: news; the picture is the same one.
        let retip = told(&message(1, 0x10, 1, 4, 0, "one!")).unwrap();
        assert!(take(&mut list, &retip, 3));
        assert_eq!(
            (list[0].tip.as_str(), list[0].icon, list[0].stamp),
            ("one!", 0xA, 1)
        );
        // Another picture: another stamp.
        let repaint = told(&message(1, 0x10, 1, 2, 0xB, "")).unwrap();
        assert!(take(&mut list, &repaint, 4));
        assert_eq!(
            (list[0].icon, list[0].stamp, list[0].tip.as_str()),
            (0xB, 4, "one!")
        );
        // A change of one that was never added adds it.
        let other = told(&message(1, 0x20, 5, 1 | 4, 0, "two")).unwrap();
        assert!(take(&mut list, &other, 5));
        assert_eq!(list.len(), 2);
        // Hidden by its program.
        let mut hide = told(&message(1, 0x20, 5, 8, 0, "")).unwrap();
        (hide.state, hide.state_mask) = (1, 1);
        assert!(take(&mut list, &hide, 6));
        assert!(list[1].hidden);
        // How presses are told is no news for who shows the icons.
        let version = told(&message(4, 0x10, 1, 0, 0, "")).unwrap();
        assert!(!take(&mut list, &version, 7));
        assert_eq!(list[0].version, 4);
        // Taken away; one that is not there cannot be.
        let gone = told(&message(2, 0x10, 1, 0, 0, "")).unwrap();
        assert!(take(&mut list, &gone, 8));
        assert!(!take(&mut list, &gone, 9));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn a_picture_is_read_from_the_icon_on_black_and_on_white() {
        // Three pixels of four: nothing, solid red, half blue.
        let black = [0, 0, 0, 0, 0, 0, 255, 0, 127, 0, 0, 0, 0, 0, 0, 0];
        let white = [
            255, 255, 255, 0, 0, 0, 255, 0, 255, 128, 128, 0, 255, 255, 255, 0,
        ];
        let picture = picture_from(&black, &white, 2).expect("a picture");
        assert_eq!(&picture.pixels[..4], &[0, 0, 0, 0]);
        assert_eq!(&picture.pixels[4..8], &[255, 0, 0, 255]);
        // Half solid, and its colour is all blue.
        assert_eq!(picture.pixels[11], 127);
        assert_eq!(&picture.pixels[8..11], &[0, 0, 255]);
        // Too few bytes for the size.
        assert!(picture_from(&black[..8], &white, 2).is_none());
    }
}
