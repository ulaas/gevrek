//! Opening items, and the shell's own menu for them.
//!
//! Both are the shell's to do: it knows what a double click on a
//! shortcut, a picture or This PC does, and its menu carries what other
//! programs added to it (an archiver's entries, "Send to", "Open with").
//!
//! Neither is done on the thread that asks. Opening can wait for a
//! program that answers slowly, and a menu is a loop of its own for as
//! long as it is open; the one who asks is the shell's drawing thread,
//! whose panel must not stand still meanwhile. Each gets a short-lived
//! thread, and what goes wrong there the shell shows itself, as Explorer
//! would (the zones lie at the bottom of the desktop: a message of the
//! shell is not hidden behind them).

use super::facts::is_path;
use super::shell::{plain, Pidl};
use gevrek_zones::backend::BackendError;
use gevrek_zones::ids::ItemId;
use std::cell::RefCell;
use windows::core::{w, Interface, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder, SHBindToParent,
    SHCreateShellItemArrayFromIDLists, ShellExecuteExW, CMF_EXTENDEDVERBS, CMF_NORMAL,
    CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO,
    CMINVOKECOMMANDINFOEX, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    EnumThreadWindows, GetMenuItemCount, GetMenuStringW, GetMessageW, IsWindow, IsWindowVisible,
    KillTimer, PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer,
    TrackPopupMenuEx, TranslateMessage, HMENU, MF_BYPOSITION, MSG, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR, WM_NULL, WM_TIMER,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};

/// The numbers the menu's entries get: the first and the last.
const FIRST: u32 = 1;
const LAST: u32 = 0x7FFF;
/// What it means that the menu asks for unicode in what it is told
/// (`CMIC_MASK_UNICODE`).
const UNICODE: u32 = 0x4000;

fn missing(item: &ItemId) -> BackendError {
    BackendError::NotFound {
        what: item.as_str().to_string(),
    }
}

/// Whether what an id names can be there at all: a path is looked for, a
/// place that is no file is the shell's to know.
fn there(item: &ItemId) -> bool {
    !item.as_str().is_empty()
        && (!is_path(item.as_str()) || std::path::Path::new(&plain(item.as_str())).exists())
}

/// Run `work` on a thread of its own that may use the shell's objects,
/// the clipboard among them (a menu's "Copy" needs it).
fn on_a_thread(name: &str, work: impl FnOnce() + Send + 'static) -> Result<(), BackendError> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            // SAFETY: paired with the call that ends it, on this thread.
            let ready = unsafe { OleInitialize(None) }.is_ok();
            work();
            if ready {
                // SAFETY: as above.
                unsafe { OleUninitialize() };
            }
        })
        .map(|_| ())
        .map_err(|error| BackendError::Refused {
            what: format!("no thread: {error}"),
        })
}

/// Have the shell do `verb` with an item (`None`: what a double click
/// does). The shell shows what goes wrong.
fn execute(item: &ItemId, verb: Option<&'static str>) -> Result<(), BackendError> {
    if !there(item) {
        return Err(missing(item));
    }
    let id = item.as_str().to_string();
    on_a_thread("gevrek-zones-open", move || {
        let Some(pidl) = Pidl::parse(&id) else {
            log::warn!("zones: {id} is not there to be opened");
            return;
        };
        let verb = verb.map(HSTRING::from);
        let mut asked = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            // By the shell's own name for the item, so that what is no
            // file opens as well; done when the call returns, for this
            // thread ends with it.
            fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC,
            lpVerb: verb
                .as_ref()
                .map_or(PCWSTR::null(), |verb| PCWSTR(verb.as_ptr())),
            lpIDList: pidl.as_ptr() as *mut _,
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: the structure is ours and says how large it is; the
        // list and the verb live until the call returns.
        match unsafe { ShellExecuteExW(&mut asked) } {
            Ok(()) => log::debug!("zones: {id} was opened by the shell"),
            Err(error) => log::warn!("zones: {id} was not opened: {error}"),
        }
    })
}

/// Open an item as a double click would.
pub fn open(item: &ItemId) -> Result<(), BackendError> {
    execute(item, None)
}

/// Open a file with `program`; without one, let the person choose (the
/// shell's own "Open with").
pub fn open_with(item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
    let Some(program) = program else {
        return execute(item, Some("openas"));
    };
    if !is_path(item.as_str()) {
        // (What is no file cannot be handed to a program.)
        return Err(BackendError::Unsupported);
    }
    if !there(item) {
        return Err(missing(item));
    }
    let (program, file) = (program.to_string(), plain(item.as_str()));
    on_a_thread("gevrek-zones-open", move || {
        let (wide_program, parameters) = (
            HSTRING::from(&program),
            HSTRING::from(format!("\"{file}\"")),
        );
        let mut asked = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC,
            lpFile: PCWSTR(wide_program.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: as in `execute`.
        match unsafe { ShellExecuteExW(&mut asked) } {
            Ok(()) => log::debug!("zones: {file} was opened with {program}"),
            Err(error) => log::warn!("zones: {program} did not open {file}: {error}"),
        }
    })
}

/// The shell's menu for some items. They are asked for together, as one
/// selection; items the shell cannot take together (they lie in folders
/// that have nothing to do with each other) get the menu of the first,
/// which is the one under the pointer.
unsafe fn menu_of(pidls: &[Pidl], owner: HWND) -> Option<IContextMenu> {
    let raw: Vec<*const ITEMIDLIST> = pidls.iter().map(Pidl::as_ptr).collect();
    if let Ok(together) = SHCreateShellItemArrayFromIDLists(&raw) {
        if let Ok(menu) = together.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) {
            return Some(menu);
        }
    }
    let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
    // (`child` points into the list it was found in; nothing to free.)
    let parent: IShellFolder = SHBindToParent(*raw.first()?, Some(&mut child)).ok()?;
    parent
        .GetUIObjectOf::<_, IContextMenu>(owner, &[child as *const ITEMIDLIST], None)
        .ok()
}

/// The lists of the items that are there.
fn lists(items: &[ItemId]) -> Vec<Pidl> {
    items
        .iter()
        .filter_map(|item| Pidl::parse(item.as_str()))
        .collect()
}

/// The entries of the shell's menu for some items, as texts (an empty
/// text for a line between entries). Nothing is shown: this is for
/// looking at what the menu would hold.
pub fn menu_entries(items: &[ItemId]) -> Result<Vec<String>, BackendError> {
    let _com = crate::com::Com::new();
    let pidls = lists(items);
    let first = items.first().ok_or(BackendError::Unsupported)?;
    if pidls.is_empty() {
        return Err(missing(first));
    }
    // SAFETY: the shell's objects and the menu are used on this thread
    // alone; the menu is destroyed before the call returns.
    unsafe {
        let shell_menu = menu_of(&pidls, HWND::default()).ok_or(BackendError::Unsupported)?;
        let menu = CreatePopupMenu().map_err(refused)?;
        let filled = shell_menu.QueryContextMenu(menu, 0, FIRST, LAST, CMF_NORMAL);
        let mut entries = Vec::new();
        if filled.is_ok() {
            for position in 0..GetMenuItemCount(menu).max(0) as u32 {
                let mut text = [0u16; 256];
                let length = GetMenuStringW(menu, position, Some(&mut text), MF_BYPOSITION);
                entries.push(String::from_utf16_lossy(&text[..length.max(0) as usize]));
            }
        }
        let _ = DestroyMenu(menu);
        filled.map_err(refused)?;
        Ok(entries)
    }
}

fn refused(error: windows::core::Error) -> BackendError {
    BackendError::Refused {
        what: error.message(),
    }
}

thread_local! {
    /// The menu that is open on this thread: what its window passes the
    /// menu's own messages on to.
    static OPEN: RefCell<Option<(Option<IContextMenu3>, Option<IContextMenu2>)>> =
        const { RefCell::new(None) };
}

/// The window a menu belongs to. Entries that draw themselves ("Send
/// to", "Open with" and what other programs add) ask their window to
/// measure and draw them; the shell's menu object answers that, and the
/// window only passes it on.
unsafe extern "system" fn menu_window(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR => {
            let handled = OPEN.with(|open| {
                let open = open.borrow();
                let (third, second) = open.as_ref()?;
                if let Some(third) = third {
                    let mut answer = LRESULT(0);
                    return third
                        .HandleMenuMsg2(message, wparam, lparam, Some(&mut answer))
                        .is_ok()
                        .then_some(answer);
                }
                let second = second.as_ref()?;
                second
                    .HandleMenuMsg(message, wparam, lparam)
                    .is_ok()
                    .then_some(LRESULT(if message == WM_INITMENUPOPUP { 0 } else { 1 }))
            });
            handled.unwrap_or_else(|| DefWindowProcW(window, message, wparam, lparam))
        }
        WM_TIMER => {
            // Asked while something an entry opened (the properties of a
            // file) is still on this thread: when it is closed, the
            // thread may end.
            if !thread_has_other_windows(window) {
                let _ = KillTimer(window, 1);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

/// Whether this thread has a window that can be seen, besides `own`.
unsafe fn thread_has_other_windows(own: HWND) -> bool {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
        let found = &mut *(found.0 as *mut (HWND, bool));
        if window != found.0 && IsWindowVisible(window).as_bool() {
            found.1 = true;
            return false.into();
        }
        true.into()
    }
    let mut found = (own, false);
    let _ = EnumThreadWindows(
        windows::Win32::System::Threading::GetCurrentThreadId(),
        Some(each),
        LPARAM(&mut found as *mut (HWND, bool) as isize),
    );
    found.1
}

/// The menu's own window: nothing to see, at the point the menu opens.
unsafe fn make_menu_window(owner: HWND, at: POINT) -> Option<HWND> {
    static CLASS: std::sync::Once = std::sync::Once::new();
    let class = w!("Gevrek_Zones_Menu");
    let module: HINSTANCE = GetModuleHandleW(None).ok()?.into();
    CLASS.call_once(|| {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(menu_window),
            hInstance: module,
            lpszClassName: class,
            ..Default::default()
        });
    });
    CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class,
        PCWSTR::null(),
        WS_POPUP,
        at.x,
        at.y,
        0,
        0,
        owner,
        HMENU::default(),
        module,
        None,
    )
    .ok()
}

/// Show the shell's own menu for `items` at a point of the screen (in
/// pixels, as Windows counts them), and do what the person picks.
///
/// `owner` is the window the items are shown in (the zones' surface).
/// The menu gets a window of its own that belongs to it, so that the menu
/// and whatever its entries open lie above that surface and may take the
/// keyboard although another thread asked. The call returns at once; the
/// menu lives on a thread of its own until it is closed.
pub fn show_menu(owner: isize, items: &[ItemId], at: (i32, i32)) -> Result<(), BackendError> {
    let first = items.first().ok_or(BackendError::Unsupported)?;
    if !items.iter().any(there) {
        return Err(missing(first));
    }
    let owner = HWND(owner as *mut _);
    // SAFETY: only asks whether the handle names a window.
    if !owner.is_invalid() && !unsafe { IsWindow(owner) }.as_bool() {
        return Err(BackendError::Refused {
            what: "the menu's owner is no window".to_string(),
        });
    }
    let (items, owner) = (items.to_vec(), owner.0 as isize);
    on_a_thread("gevrek-zones-menu", move || {
        // SAFETY: everything below is made, used and destroyed on this
        // thread.
        unsafe { run_menu(HWND(owner as *mut _), &items, POINT { x: at.0, y: at.1 }) };
    })
}

unsafe fn run_menu(owner: HWND, items: &[ItemId], at: POINT) {
    let pidls = lists(items);
    if pidls.is_empty() {
        return;
    }
    let Some(window) = make_menu_window(owner, at) else {
        log::warn!("zones: no window for the menu");
        return;
    };
    let shown = (|| {
        let shell_menu = menu_of(&pidls, window)?;
        let menu = CreatePopupMenu().ok()?;
        let held = |key| GetKeyState(i32::from(key)) < 0;
        // (With Shift held the shell adds the entries it keeps back
        // otherwise, as in Explorer.)
        let how = if held(VK_SHIFT.0) {
            CMF_NORMAL | CMF_EXTENDEDVERBS
        } else {
            CMF_NORMAL
        };
        if shell_menu
            .QueryContextMenu(menu, 0, FIRST, LAST, how)
            .is_err()
        {
            let _ = DestroyMenu(menu);
            return None;
        }
        OPEN.with(|open| {
            *open.borrow_mut() = Some((shell_menu.cast().ok(), shell_menu.cast().ok()));
        });
        // A menu closes when the person presses elsewhere only if its
        // window is the one in front.
        let _ = SetForegroundWindow(window);
        let picked = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            at.x,
            at.y,
            window,
            None,
        );
        let _ = PostMessageW(window, WM_NULL, WPARAM(0), LPARAM(0));
        OPEN.with(|open| *open.borrow_mut() = None);
        let _ = DestroyMenu(menu);
        let picked = picked.0 as u32;
        if picked < FIRST {
            log::debug!("zones: the shell's menu was closed without a choice");
            return None;
        }
        // The entry by its number, said both ways (some menus read the
        // one, some the other).
        let number = (picked - FIRST) as usize;
        let mut how = UNICODE | CMIC_MASK_PTINVOKE;
        if held(VK_SHIFT.0) {
            how |= CMIC_MASK_SHIFT_DOWN;
        }
        if held(VK_CONTROL.0) {
            how |= CMIC_MASK_CONTROL_DOWN;
        }
        let asked = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: how,
            hwnd: window,
            lpVerb: PCSTR(number as *const u8),
            lpVerbW: PCWSTR(number as *const u16),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: at,
            ..Default::default()
        };
        if let Err(error) = shell_menu
            .InvokeCommand(&asked as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO)
        {
            log::warn!("zones: the menu's entry was not carried out: {error}");
        } else {
            log::debug!("zones: entry {number} of the shell's menu was carried out");
        }
        Some(shell_menu)
    })();
    // What an entry opened on this thread (a file's properties) lives as
    // long as the thread passes its messages on. Looked at twice a second
    // only while there is such a window; the thread ends when it closed.
    if shown.is_some() && thread_has_other_windows(window) {
        SetTimer(window, 1, 500, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    drop(shown);
    let _ = DestroyWindow(window);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_not_there_is_not_opened() {
        let gone = ItemId::new("C:\\gevrek-no-such-folder\\nothing.txt");
        assert_eq!(open(&gone), Err(missing(&gone)));
        assert_eq!(open_with(&gone, None), Err(missing(&gone)));
        assert_eq!(open_with(&gone, Some("notepad.exe")), Err(missing(&gone)));
        assert_eq!(
            show_menu(0, std::slice::from_ref(&gone), (0, 0)),
            Err(missing(&gone))
        );
        assert_eq!(show_menu(0, &[], (0, 0)), Err(BackendError::Unsupported));
        // What is no file is not handed to a program.
        let this_pc = ItemId::new("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}");
        assert_eq!(
            open_with(&this_pc, Some("notepad.exe")),
            Err(BackendError::Unsupported)
        );
        assert!(there(&this_pc));
        assert!(!there(&ItemId::new("")));
    }
}
