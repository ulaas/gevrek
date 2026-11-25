//! What was copied lately, for the launcher to offer again (after a `:`).
//!
//! On Windows the system keeps what is on the clipboard after its program
//! has gone, and has a history of its own (Win+V) for whoever switched it
//! on. gevrek's shell keeps the texts itself, so that its launcher can list
//! them like everything else it lists: it is told each time the clipboard
//! changes, reads the text, and remembers the last thirty. What a password
//! manager marks as not to be kept is not kept. Kept while the shell runs,
//! not written down.
//!
//! On macOS the pasteboard's count of changes is looked at twice a second
//! (macOS tells no program when the pasteboard changes); what a password
//! manager marks as concealed or transient (the `org.nspasteboard` types
//! macOS's programs agree on) is not kept.
//!
//! On Linux the shell keeps the clipboard through the compositor
//! (`gevrek-shell`'s own `clipboard`), which this does not replace.

use std::sync::{Arc, Mutex};

/// How many texts are remembered, and how long one may be (in bytes).
pub const REMEMBERED: usize = 30;
pub const LONGEST: usize = 100_000;

/// Take a text into the list of what was copied: at the front, once, and
/// only if it is worth it (something other than blanks, not enormous).
pub fn remember(texts: &mut Vec<String>, text: &str) {
    if text.trim().is_empty() || text.len() > LONGEST {
        return;
    }
    texts.retain(|known| known != text);
    texts.insert(0, text.to_string());
    texts.truncate(REMEMBERED);
}

/// The keeper, as the shell holds it.
pub struct Keeper {
    /// The texts that were copied, the newest first.
    pub texts: Arc<Mutex<Vec<String>>>,
    #[cfg(windows)]
    put: windows_keeper::Put,
}

#[cfg(target_os = "macos")]
mod mac_keeper {
    use super::{remember, Keeper};
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;
    use std::sync::{Arc, Mutex};

    /// What password managers put on the pasteboard to say: not to be kept.
    const NOT_KEPT: &[&str] = &["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];

    /// How often the pasteboard is looked at.
    const LOOK: std::time::Duration = std::time::Duration::from_millis(500);

    pub fn start() -> Option<Keeper> {
        let texts = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&texts);
        std::thread::Builder::new()
            .name("gevrek-clipboard".to_string())
            .spawn(move || {
                let board = NSPasteboard::generalPasteboard();
                let mut seen = board.changeCount();
                loop {
                    std::thread::sleep(LOOK);
                    let count = board.changeCount();
                    if count == seen {
                        continue;
                    }
                    seen = count;
                    let concealed = board.types().is_some_and(|types| {
                        types.iter().any(|kind| NOT_KEPT.contains(&kind.to_string().as_str()))
                    });
                    if concealed {
                        continue;
                    }
                    // SAFETY: the system's constant.
                    let Some(text) = board.stringForType(unsafe { NSPasteboardTypeString }) else {
                        continue;
                    };
                    let mut texts = kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    remember(&mut texts, &text.to_string());
                }
            })
            .ok()?;
        Some(Keeper { texts })
    }

    /// Make `text` what is on the pasteboard.
    pub fn put(text: &str) {
        let board = NSPasteboard::generalPasteboard();
        board.clearContents();
        // SAFETY: the system's constant.
        board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    }
}

impl Keeper {
    /// The texts as they are now.
    pub fn copied(&self) -> Vec<String> {
        self.texts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Make `text` what is on the clipboard.
    pub fn put(&self, text: &str) {
        #[cfg(windows)]
        self.put.put(text);
        #[cfg(target_os = "macos")]
        mac_keeper::put(text);
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = text;
    }
}

/// Start keeping what is copied. `None` where this does not (anywhere but
/// Windows), or when the system cannot be listened to.
pub fn start() -> Option<Keeper> {
    #[cfg(windows)]
    {
        windows_keeper::start()
    }
    #[cfg(target_os = "macos")]
    {
        mac_keeper::start()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

#[cfg(windows)]
mod windows_keeper {
    use super::{remember, Keeper};
    use std::sync::{mpsc, Arc, Mutex};
    use windows::core::w;
    use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW,
        RegisterClassExW, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WNDCLASSEXW,
    };

    /// Text, in the system's wide letters (`CF_UNICODETEXT`).
    const TEXT: u32 = 13;
    const WM_CLIPBOARDUPDATE: u32 = 0x031D;
    /// "There is a text to put on the clipboard."
    const PUT: u32 = WM_APP + 1;

    /// How the shell asks for a text to be put on the clipboard.
    pub struct Put {
        texts: mpsc::Sender<String>,
        window: isize,
    }

    impl Put {
        pub fn put(&self, text: &str) {
            if self.texts.send(text.to_string()).is_ok() {
                // SAFETY: a message to the keeper's own window.
                let _ = unsafe {
                    PostMessageW(
                        HWND(self.window as *mut _),
                        PUT,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
        }
    }

    unsafe extern "system" fn proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcW(window, message, wparam, lparam)
    }

    pub fn start() -> Option<Keeper> {
        let texts = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&texts);
        let (put, to_put) = mpsc::channel::<String>();
        let (ready, is_ready) = mpsc::channel::<isize>();
        std::thread::Builder::new()
            .name("gevrek-clipboard".to_string())
            .spawn(move || {
                // SAFETY: a window of this thread's own that is never
                // shown, told when the clipboard changes; its messages are
                // read here for as long as the program runs.
                unsafe {
                    let module = GetModuleHandleW(None).unwrap_or_default();
                    let class = w!("Gevrek_ShellClipboard");
                    RegisterClassExW(&WNDCLASSEXW {
                        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                        lpfnWndProc: Some(proc),
                        hInstance: module.into(),
                        lpszClassName: class,
                        ..Default::default()
                    });
                    let window = CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        class,
                        WINDOW_STYLE(0),
                        0,
                        0,
                        0,
                        0,
                        HWND_MESSAGE,
                        None,
                        module,
                        None,
                    );
                    let Ok(window) = window else {
                        let _ = ready.send(0);
                        return;
                    };
                    if AddClipboardFormatListener(window).is_err() {
                        let _ = ready.send(0);
                        return;
                    }
                    let _ = ready.send(window.0 as isize);
                    // What programs put beside a text that is not to be kept.
                    let secret = RegisterClipboardFormatW(w!(
                        "ExcludeClipboardContentFromMonitorProcessing"
                    ));
                    // What this thread put there itself is known already.
                    let mut own: Option<String> = None;
                    let mut message = MSG::default();
                    while GetMessageW(&mut message, None, 0, 0).as_bool() {
                        match message.message {
                            WM_CLIPBOARDUPDATE => {
                                if secret != 0 && IsClipboardFormatAvailable(secret).is_ok() {
                                    continue;
                                }
                                if let Some(text) = read(window) {
                                    if own.as_deref() == Some(text.as_str()) {
                                        own = None;
                                    }
                                    remember(
                                        &mut kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
                                        &text,
                                    );
                                }
                            }
                            PUT => {
                                while let Ok(text) = to_put.try_recv() {
                                    if write(window, &text) {
                                        own = Some(text);
                                    }
                                }
                            }
                            _ => {
                                DispatchMessageW(&message);
                            }
                        }
                    }
                }
            })
            .ok()?;
        let window = is_ready.recv().ok().filter(|window| *window != 0)?;
        Some(Keeper {
            texts,
            put: Put { texts: put, window },
        })
    }

    /// The text on the clipboard, if it has one.
    unsafe fn read(window: HWND) -> Option<String> {
        IsClipboardFormatAvailable(TEXT).ok()?;
        OpenClipboard(window).ok()?;
        let text = (|| {
            let data = GetClipboardData(TEXT).ok()?;
            let memory = HGLOBAL(data.0);
            let letters = GlobalLock(memory) as *const u16;
            if letters.is_null() {
                return None;
            }
            // No further than the memory goes: a program may put a text
            // there that does not end with a nought. (It was read until
            // one came, wherever that was.)
            let most = GlobalSize(memory) / std::mem::size_of::<u16>();
            let all = std::slice::from_raw_parts(letters, most);
            let length = all.iter().position(|letter| *letter == 0).unwrap_or(most);
            let text = String::from_utf16(&all[..length]).ok();
            let _ = GlobalUnlock(memory);
            text
        })();
        let _ = CloseClipboard();
        text
    }

    /// Put a text on the clipboard. Whether the system took it.
    unsafe fn write(window: HWND, text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        if OpenClipboard(window).is_err() {
            return false;
        }
        let taken = (|| {
            EmptyClipboard().ok()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let to = GlobalLock(memory) as *mut u16;
            if to.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), to, wide.len());
            let _ = GlobalUnlock(memory);
            // (The system owns the memory from here on.)
            SetClipboardData(TEXT, HANDLE(memory.0)).ok()?;
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_copied_is_remembered_once_and_the_newest_first() {
        let mut texts = Vec::new();
        remember(&mut texts, "one");
        remember(&mut texts, "two");
        remember(&mut texts, "one");
        assert_eq!(texts, ["one", "two"]);
        // Blanks and what is enormous are not worth keeping.
        remember(&mut texts, "   \n");
        remember(&mut texts, &"x".repeat(LONGEST + 1));
        assert_eq!(texts.len(), 2);
        for number in 0..40 {
            remember(&mut texts, &format!("text {number}"));
        }
        assert_eq!(texts.len(), REMEMBERED);
        assert_eq!(texts[0], "text 39");
    }
}
