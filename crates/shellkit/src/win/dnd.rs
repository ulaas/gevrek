//! Drag and drop on Windows: OLE.
//!
//! A surface that accepts drops has a drop target registered for its
//! window. Windows calls the target while another program's drag is over
//! the window, in the middle of this thread's handling of messages, and
//! wants the answer on the spot (which pointer the source shows, what was
//! done with the drop). The host cannot be touched there, so the target
//! works from two lists of this thread: what each window would take
//! (written by the host when a surface says so), and what the targets were
//! told (read by the host at its next pass and handed to the surfaces as
//! [`DragEvent`](crate::dnd::DragEvent)s).
//!
//! A drag out of a surface is `DoDragDrop`, which does not return until
//! the drag is over: it runs a loop of its own that takes the mouse and
//! the keys. What the shell must go on doing meanwhile is done from the
//! calls that loop makes to the drag's source ([`run`]).
//!
//! What is dragged is an `IDataObject`. Whoever has one (the zones'
//! backend, for things of the shell that are no files) hands it over; for
//! plain paths one is made here ([`object_for`]) with the shell's own
//! empty data object, which takes whatever is put into it. That matters:
//! the picture that follows the pointer is kept in the data object by the
//! shell's helper, in kinds of its own, and an object written by hand
//! would have to store those too.

use crate::dnd::{
    self, hdrop, Continue, DragAction, DragActions, DragImage, DragOutcome, DragPayload,
    DropAnswer, Held, NativeData, Offered,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows::core::{implement, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, BOOL, COLORREF, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
    HANDLE, HGLOBAL, HWND, POINT, POINTL, SIZE, S_OK,
};
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, DeleteObject, ScreenToClient, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
    HDC,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IDataObject, CLSCTX_INPROC_SERVER, DATADIR_GET,
    DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::{GetClipboardFormatNameW, RegisterClipboardFormatW};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, OleInitialize,
    OleUninitialize, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, CF_UNICODETEXT,
    DROPEFFECT,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::{
    CLSID_DragDropHelper, DragQueryFileW, IDragSourceHelper, IDropTargetHelper, SHCreateDataObject,
    CFSTR_INETURLW, CFSTR_LOGICALPERFORMEDDROPEFFECT, CFSTR_PERFORMEDDROPEFFECT, CFSTR_SHELLIDLIST,
    HDROP, SHDRAGIMAGE,
};

/// The thread may drag and be dropped on while this lives.
///
/// OLE wants a thread whose objects are called on that thread alone (a
/// single-threaded apartment). A thread nobody set up becomes one; a
/// thread that is one already (the shell's objects were used on it) stays
/// as it is and counts one more user. A thread that was set up the other
/// way cannot be changed, and has no drag and drop: said once, in the log.
pub(super) struct Ole(());

impl Ole {
    pub(super) fn new() -> Option<Self> {
        // SAFETY: no pointers; undone in `drop` on the same thread.
        match unsafe { OleInitialize(None) } {
            Ok(()) => Some(Self(())),
            Err(error) => {
                log::warn!(
                    "no drag and drop for the shell's surfaces: OLE refused this thread ({error})"
                );
                None
            }
        }
    }
}

impl Drop for Ole {
    fn drop(&mut self) {
        // SAFETY: pairs with the `OleInitialize` that made this.
        unsafe { OleUninitialize() };
    }
}

/// What a window would take of a drag.
#[derive(Debug, Clone, Copy, Default)]
struct Standing {
    /// Before the surface has said anything of the drag that is over it.
    standing: DropAnswer,
    /// What it said of this one.
    now: Option<DropAnswer>,
    /// A drag is over it.
    over: bool,
}

/// What a drop target was told, for the host.
pub(super) enum Told {
    /// (Places are in the pixels of the window's inside.)
    Entered {
        window: isize,
        x: i32,
        y: i32,
        offered: Offered,
    },
    Moved {
        window: isize,
        x: i32,
        y: i32,
    },
    Left {
        window: isize,
    },
    Dropped {
        window: isize,
        x: i32,
        y: i32,
        payload: DragPayload,
        action: DragAction,
        object: IDataObject,
    },
}

thread_local! {
    static STANDING: RefCell<HashMap<isize, Standing>> = RefCell::new(HashMap::new());
    static TOLD: RefCell<Vec<Told>> = const { RefCell::new(Vec::new()) };
}

fn standing<T>(window: isize, change: impl FnOnce(&mut Standing) -> T) -> Option<T> {
    STANDING.with(|all| {
        let mut all = all.try_borrow_mut().ok()?;
        Some(change(all.entry(window).or_default()))
    })
}

fn tell(told: Told) {
    TOLD.with(|all| {
        if let Ok(mut all) = all.try_borrow_mut() {
            all.push(told);
        }
    });
}

/// What the targets were told since this was last asked.
pub(super) fn told() -> Vec<Told> {
    TOLD.with(|all| {
        all.try_borrow_mut()
            .map(|mut all| std::mem::take(&mut *all))
            .unwrap_or_default()
    })
}

/// What a window takes of any drag, until its surface says more.
pub(super) fn set_standing(window: HWND, answer: DropAnswer) {
    standing(window.0 as isize, |known| known.standing = answer);
}

/// What a window takes of the drag that is over it now. (Said when none
/// is, it is about one that left already, and is not kept for the next.)
pub(super) fn set_answer(window: HWND, answer: DropAnswer) {
    standing(window.0 as isize, |known| {
        if known.over {
            known.now = Some(answer);
        }
    });
}

/// Make a window a place to drop on. `false`: Windows refused.
pub(super) fn register(window: HWND) -> bool {
    let target: IDropTarget = Target::new(window.0 as isize, true).into();
    // SAFETY: the window is this thread's; the target is kept by Windows
    // until `revoke`.
    match unsafe { RegisterDragDrop(window, &target) } {
        Ok(()) => true,
        Err(error) => {
            log::warn!("a surface takes no drops: {error}");
            false
        }
    }
}

/// A window is no place to drop on any more (it must be told before the
/// window is destroyed: Windows keeps the target for it until then).
pub(super) fn revoke(window: HWND) {
    // SAFETY: a window of this thread that was registered.
    unsafe {
        let _ = RevokeDragDrop(window);
    }
    STANDING.with(|all| {
        if let Ok(mut all) = all.try_borrow_mut() {
            all.remove(&(window.0 as isize));
        }
    });
}

/// How a kind of data is asked for: the whole of it, in memory.
fn format(kind: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: kind,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// The number of a kind that programs agree on by name.
fn named(name: PCWSTR) -> u16 {
    // SAFETY: a constant name with its zero.
    (unsafe { RegisterClipboardFormatW(name) }) as u16
}

/// Run `read` on a kind's data while it is held; nothing when the object
/// has none of that kind, or has it in another shape than memory.
fn with_data<T>(
    object: &IDataObject,
    kind: u16,
    read: impl FnOnce(HGLOBAL, &[u8]) -> T,
) -> Option<T> {
    // SAFETY: the medium is ours until released; its memory is read only
    // while locked, and no further than Windows says it reaches.
    unsafe {
        let mut medium = object.GetData(&format(kind)).ok()?;
        let mut out = None;
        if medium.tymed == TYMED_HGLOBAL.0 as u32 && !medium.u.hGlobal.is_invalid() {
            let memory = medium.u.hGlobal;
            let size = GlobalSize(memory);
            let at = GlobalLock(memory);
            if !at.is_null() {
                out = Some(read(
                    memory,
                    std::slice::from_raw_parts(at.cast::<u8>(), size),
                ));
                let _ = GlobalUnlock(memory);
            }
        }
        ReleaseStgMedium(&mut medium);
        out
    }
}

/// Put a kind's data into an object, which owns the memory from then on.
fn put(object: &IDataObject, kind: u16, bytes: &[u8]) -> windows::core::Result<()> {
    // SAFETY: the memory is as large as what is copied into it; it is the
    // object's once `SetData` took it, and freed here when it did not.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let at = GlobalLock(memory);
        if at.is_null() {
            let _ = GlobalFree(memory);
            return Err(windows::core::Error::from_win32());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), at.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        };
        let done = object.SetData(&format(kind), &medium, true);
        if done.is_err() {
            let _ = GlobalFree(memory);
        }
        done
    }
}

/// Text in Windows' wide letters, up to its zero.
fn wide_text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn wide_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A number an object keeps about what was done with it (the effects).
fn number(object: &IDataObject, name: PCWSTR) -> Option<u32> {
    with_data(object, named(name), |_, bytes| {
        bytes
            .get(0..4)
            .map(|four| u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
    })
    .flatten()
}

/// The paths of a list of dropped files, read by the system (which knows
/// the letters of the machine's older programs).
fn paths_by_system(memory: HGLOBAL) -> Vec<String> {
    let list = HDROP(memory.0);
    let mut paths = Vec::new();
    // SAFETY: the list is held by the caller; each buffer is as long as
    // Windows said the path is, with room for its zero.
    unsafe {
        let count = DragQueryFileW(list, u32::MAX, None);
        for index in 0..count {
            let length = DragQueryFileW(list, index, None) as usize;
            let mut path = vec![0u16; length + 1];
            let written = DragQueryFileW(list, index, Some(&mut path)) as usize;
            paths.push(String::from_utf16_lossy(&path[..written.min(length)]));
        }
    }
    paths
}

/// The names of the kinds of data an object has.
pub fn kinds_of(object: &IDataObject) -> Vec<String> {
    let mut kinds = Vec::new();
    // SAFETY: one description at a time into a place of that size; what
    // Windows set aside for a description is given back.
    unsafe {
        let Ok(all) = object.EnumFormatEtc(DATADIR_GET.0 as u32) else {
            return kinds;
        };
        loop {
            let mut one = [FORMATETC::default()];
            let mut fetched = 0u32;
            if all.Next(&mut one, Some(&mut fetched)) != S_OK || fetched == 0 {
                break;
            }
            if !one[0].ptd.is_null() {
                CoTaskMemFree(Some(one[0].ptd.cast()));
            }
            let name = kind_name(one[0].cfFormat);
            if !kinds.contains(&name) {
                kinds.push(name);
            }
            // (A source that never says "no more" is not listened to
            // for ever.)
            if kinds.len() >= 256 {
                break;
            }
        }
    }
    kinds
}

/// What a kind is called: Windows' own by their names in its headers, the
/// others by the names programs registered them with.
fn kind_name(kind: u16) -> String {
    match kind {
        1 => "CF_TEXT".to_string(),
        2 => "CF_BITMAP".to_string(),
        7 => "CF_OEMTEXT".to_string(),
        8 => "CF_DIB".to_string(),
        13 => "CF_UNICODETEXT".to_string(),
        15 => "CF_HDROP".to_string(),
        16 => "CF_LOCALE".to_string(),
        17 => "CF_DIBV5".to_string(),
        other => {
            let mut name = [0u16; 256];
            // SAFETY: a buffer of the length given.
            let length = unsafe { GetClipboardFormatNameW(u32::from(other), &mut name) };
            if length > 0 {
                String::from_utf16_lossy(&name[..length as usize])
            } else {
                format!("#{other}")
            }
        }
    }
}

/// What an object carries, as a payload: files as paths; what the shell
/// names without a path as it came, for the system's own calls; a link;
/// text when there are no files. `allowed` is what the source lets be done
/// (it says so beside the object, not in it).
pub fn payload_of(object: &IDataObject, allowed: DragActions) -> DragPayload {
    let mut payload = DragPayload {
        allowed,
        ..Default::default()
    };
    if let Some(paths) = with_data(object, CF_HDROP.0, |memory, bytes| {
        hdrop::decode(bytes).unwrap_or_else(|| paths_by_system(memory))
    }) {
        payload.paths = paths;
    }
    if let Some(bytes) = with_data(object, named(CFSTR_SHELLIDLIST), |_, bytes| bytes.to_vec()) {
        payload.native.push(NativeData {
            kind: "Shell IDList Array".to_string(),
            bytes,
        });
    }
    if let Some(link) = with_data(object, named(CFSTR_INETURLW), |_, bytes| wide_text(bytes)) {
        if !link.is_empty() {
            payload.uris.push(link);
        }
    }
    if payload.paths.is_empty() {
        payload.text = with_data(object, CF_UNICODETEXT.0, |_, bytes| wide_text(bytes))
            .filter(|text| !text.is_empty());
    }
    payload
}

/// An object to drag for a payload of plain paths (and text): the shell's
/// own empty one with the list of files put into it. What only the shell
/// can name (`native`) is not made here; whoever has such things has an
/// object for them and drags that.
pub fn object_for(payload: &DragPayload) -> windows::core::Result<IDataObject> {
    // SAFETY: no folder, no items and no inner object are all allowed.
    let object: IDataObject = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }?;
    if !payload.paths.is_empty() {
        put(&object, CF_HDROP.0, &hdrop::encode(&payload.paths))?;
    }
    if let Some(link) = payload.uris.first() {
        put(&object, named(CFSTR_INETURLW), &wide_bytes(link))?;
    }
    let text = match &payload.text {
        Some(text) => Some(text.clone()),
        // Dropped into a text: the paths, a line each.
        None if !payload.paths.is_empty() => Some(payload.paths.join("\r\n")),
        None => payload.uris.first().cloned(),
    };
    if let Some(text) = text {
        put(&object, CF_UNICODETEXT.0, &wide_bytes(&text))?;
    }
    Ok(object)
}

/// A window's drop target.
#[implement(IDropTarget)]
struct Target {
    window: isize,
    /// Shows the picture another program drags (Explorer's faded icons)
    /// while it is over the window. Not there: no picture, no harm.
    helper: Option<IDropTargetHelper>,
    /// Where the drag was last, so that only moves are told as moves
    /// (Windows asks again and again while the pointer rests).
    last: Cell<Option<(i32, i32)>>,
}

impl Target {
    fn new(window: isize, with_picture: bool) -> Self {
        let helper = with_picture
            .then(|| {
                // SAFETY: an object of the shell, made in this program.
                unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER) }.ok()
            })
            .flatten();
        Self {
            window,
            helper,
            last: Cell::new(None),
        }
    }

    fn handle(&self) -> HWND {
        HWND(self.window as *mut _)
    }

    /// A place on the desktop, on the window's inside.
    fn inside(&self, at: &POINTL) -> (i32, i32) {
        let mut point = POINT { x: at.x, y: at.y };
        // SAFETY: a point of ours. (Left as it is where there is no
        // window to measure from.)
        unsafe {
            let _ = ScreenToClient(self.handle(), &mut point);
        }
        (point.x, point.y)
    }

    /// What would be done now: what the source allows, the keys ask for,
    /// and the surface last said.
    fn action(&self, keys: MODIFIERKEYS_FLAGS, allowed: u32) -> Option<DragAction> {
        let answer = standing(self.window, |known| known.now.unwrap_or(known.standing))
            .unwrap_or(DropAnswer::REFUSE);
        dnd::negotiate(
            DragActions::from_effects(allowed),
            Held::from_key_state(keys.0),
            answer,
        )
    }
}

fn effect_of(action: Option<DragAction>) -> DROPEFFECT {
    DROPEFFECT(action.map_or(0, DragAction::effect))
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        object: Option<&IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if effect.is_null() {
            return Ok(());
        }
        // SAFETY: Windows gives a place to answer in, with what the
        // source allows in it.
        let allowed = unsafe { (*effect).0 };
        standing(self.window, |known| {
            known.over = true;
            known.now = None;
        });
        let (x, y) = self.inside(at);
        self.last.set(Some((x, y)));
        let offered = match object {
            Some(object) => {
                let payload = payload_of(object, DragActions::from_effects(allowed));
                Offered {
                    kinds: kinds_of(object),
                    files: !payload.paths.is_empty(),
                    allowed: payload.allowed,
                    payload: Some(payload),
                }
            }
            None => Offered::default(),
        };
        tell(Told::Entered {
            window: self.window,
            x,
            y,
            offered,
        });
        let answer = effect_of(self.action(keys, allowed));
        // SAFETY: as above; the helper is told what Windows told us.
        unsafe {
            *effect = answer;
            if let (Some(helper), Some(object)) = (&self.helper, object) {
                let point = POINT { x: at.x, y: at.y };
                let _ = helper.DragEnter(self.handle(), object, &point, answer);
            }
        }
        Ok(())
    }

    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if effect.is_null() {
            return Ok(());
        }
        let (x, y) = self.inside(at);
        if self.last.replace(Some((x, y))) != Some((x, y)) {
            tell(Told::Moved {
                window: self.window,
                x,
                y,
            });
        }
        // SAFETY: as in `DragEnter`.
        unsafe {
            let answer = effect_of(self.action(keys, (*effect).0));
            *effect = answer;
            if let Some(helper) = &self.helper {
                let point = POINT { x: at.x, y: at.y };
                let _ = helper.DragOver(&point, answer);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        standing(self.window, |known| {
            known.over = false;
            known.now = None;
        });
        self.last.set(None);
        tell(Told::Left {
            window: self.window,
        });
        if let Some(helper) = &self.helper {
            // SAFETY: no arguments.
            unsafe {
                let _ = helper.DragLeave();
            }
        }
        Ok(())
    }

    fn Drop(
        &self,
        object: Option<&IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if effect.is_null() {
            return Ok(());
        }
        // SAFETY: as in `DragEnter`.
        let allowed = unsafe { (*effect).0 };
        // (The button that dragged is up by now; the keys still count.)
        let action = self.action(keys, allowed);
        standing(self.window, |known| {
            known.over = false;
            known.now = None;
        });
        self.last.set(None);
        let (x, y) = self.inside(at);
        if let Some(helper) = &self.helper {
            if let Some(object) = object {
                let point = POINT { x: at.x, y: at.y };
                // SAFETY: what Windows told us, passed on.
                unsafe {
                    let _ = helper.Drop(object, &point, effect_of(action));
                }
            }
        }
        let told = match (object, action) {
            (Some(object), Some(action)) => {
                let payload = payload_of(object, DragActions::from_effects(allowed));
                // A move is done by the surface, later and in full: the
                // source is told that nothing is left for it to do (it
                // would delete the files before they were moved), and,
                // for those that ask, that it was a move all the same.
                let said = if action == DragAction::Move {
                    let _ = put(
                        object,
                        named(CFSTR_PERFORMEDDROPEFFECT),
                        &0u32.to_le_bytes(),
                    );
                    let _ = put(
                        object,
                        named(CFSTR_LOGICALPERFORMEDDROPEFFECT),
                        &DragAction::Move.effect().to_le_bytes(),
                    );
                    DROPEFFECT(0)
                } else {
                    effect_of(Some(action))
                };
                // SAFETY: as in `DragEnter`.
                unsafe { *effect = said };
                Told::Dropped {
                    window: self.window,
                    x,
                    y,
                    payload,
                    action,
                    object: object.clone(),
                }
            }
            _ => {
                // SAFETY: as in `DragEnter`.
                unsafe { *effect = DROPEFFECT(0) };
                Told::Left {
                    window: self.window,
                }
            }
        };
        tell(told);
        Ok(())
    }
}

/// What the shell goes on doing while a drag of its own is under way.
type During = *mut (dyn FnMut() + 'static);

/// The source of a drag out of a surface: Windows asks it whether the
/// drag goes on, and tells it what the target would do.
#[implement(IDropSource)]
struct Source {
    /// The buttons that were down when the drag began.
    began: u32,
    /// Gone when the drag is over (Windows may keep the source longer).
    during: Rc<Cell<Option<During>>>,
    /// The shell's own work is under way: it is not begun inside itself.
    busy: Cell<bool>,
    worked: Cell<Instant>,
}

impl Source {
    /// The shell's surfaces are drawn and its messages read, a few
    /// hundred times a second at most.
    fn work(&self) {
        if self.worked.get().elapsed() < Duration::from_millis(4) || self.busy.replace(true) {
            return;
        }
        if let Some(during) = self.during.get() {
            // SAFETY: there until `run` takes it away, which it does
            // before what it points to ends; called on the thread that
            // made it, and never inside itself (`busy`).
            unsafe { (*during)() };
        }
        self.worked.set(Instant::now());
        self.busy.set(false);
    }
}

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        self.work();
        match dnd::continue_drag(escape.as_bool(), keys.0, self.began) {
            Continue::On => S_OK,
            Continue::Drop => DRAGDROP_S_DROP,
            Continue::Cancel => DRAGDROP_S_CANCEL,
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        self.work();
        // The pointers everyone knows: an arrow with a plus, a shortcut's
        // arrow, the sign for "not here".
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// The mouse buttons that are down, as Windows counts them for a drag.
fn buttons_down() -> u32 {
    let down = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| {
        // SAFETY: no pointers.
        (unsafe { GetKeyState(i32::from(key.0)) }) < 0
    };
    u32::from(down(VK_LBUTTON))
        | u32::from(down(VK_RBUTTON)) << 1
        | u32::from(down(VK_MBUTTON)) << 4
}

/// Give an object the picture that follows the pointer while it is
/// dragged. The shell's helper keeps it in the object and draws it; over
/// windows that show such pictures (Explorer, ours) it is seen.
fn picture(object: &IDataObject, image: &DragImage) -> windows::core::Result<()> {
    // SAFETY: the bitmap is as large as its header says, which is what is
    // copied into it; the helper owns it once it took it, and it is
    // deleted here when it did not.
    unsafe {
        let helper: IDragSourceHelper =
            CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER)?;
        let about = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: image.width as i32,
                // (Less than nothing: the first row is the top one.)
                biHeight: -(image.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            HDC::default(),
            &about,
            DIB_RGB_COLORS,
            &mut bits,
            HANDLE::default(),
            0,
        )?;
        if bits.is_null() {
            let _ = DeleteObject(bitmap);
            return Err(windows::core::Error::from_win32());
        }
        let pixels = dnd::premultiplied_bgra(&image.rgba);
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        let dragged = SHDRAGIMAGE {
            sizeDragImage: SIZE {
                cx: image.width as i32,
                cy: image.height as i32,
            },
            ptOffset: POINT {
                x: image.hotspot.0,
                y: image.hotspot.1,
            },
            hbmpDragImage: bitmap,
            // No colour of the picture stands for "nothing here": how
            // solid each pixel is says that.
            crColorKey: COLORREF(0xFFFF_FFFF),
        };
        let done = helper.InitializeFromBitmap(&dragged, object);
        if done.is_err() {
            let _ = DeleteObject(bitmap);
        }
        done
    }
}

/// Drag an object until it is dropped or given up. This returns only
/// then: Windows runs the drag in a loop of its own, which takes every
/// message of the mouse and the keys and hands the others to their
/// windows. `during` is called from inside that loop, whenever Windows
/// asks the source something (at every move of the pointer, and at every
/// change of a key or a button), so the shell's surfaces go on being
/// drawn and its messages read.
pub(super) fn run(
    object: &IDataObject,
    allowed: DragActions,
    image: Option<&DragImage>,
    during: &mut dyn FnMut(),
) -> DragOutcome {
    let began = buttons_down();
    if began == 0 {
        // The button was let go before the drag could begin: begun now,
        // it would at once be dropped on whatever is under the pointer.
        return DragOutcome::Cancelled;
    }
    if let Some(image) = image.filter(|image| image.is_sound()) {
        if let Err(error) = picture(object, image) {
            log::debug!("a drag without its picture: {error}");
        }
    }
    // SAFETY: the same pointer with a longer name for its lifetime; it is
    // taken out of the source below, before `during` ends.
    let during: During =
        unsafe { std::mem::transmute::<*mut (dyn FnMut() + '_), During>(during as *mut _) };
    let shared = Rc::new(Cell::new(Some(during)));
    let source: IDropSource = Source {
        began,
        during: Rc::clone(&shared),
        busy: Cell::new(false),
        worked: Cell::new(Instant::now()),
    }
    .into();
    let mut effect = DROPEFFECT(0);
    // SAFETY: both objects live through the call, which is on the thread
    // OLE was started on.
    let result = unsafe { DoDragDrop(object, &source, DROPEFFECT(allowed.effects()), &mut effect) };
    shared.set(None);
    if result.is_err() {
        log::warn!(
            "a drag could not be made: {}",
            windows::core::Error::from(result)
        );
    }
    dnd::outcome(
        result == DRAGDROP_S_DROP,
        effect.0,
        number(object, CFSTR_PERFORMEDDROPEFFECT),
        number(object, CFSTR_LOGICALPERFORMEDDROPEFFECT),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::SystemServices::{MK_CONTROL, MK_LBUTTON, MK_SHIFT};

    /// OLE for a test's thread.
    fn ole() -> Ole {
        Ole::new().expect("a test's thread is nobody's yet")
    }

    #[test]
    fn paths_put_into_an_object_are_read_back_from_it() {
        let _ole = ole();
        let sent = DragPayload::files([
            r"C:\Users\ada\a.txt".to_string(),
            r"D:\Çay Bahçesi\日本語 𝄞.txt".to_string(),
            format!(r"C:\{}\long.txt", "a-folder-with-a-long-name\\".repeat(20)),
        ]);
        let object = object_for(&sent).expect("the shell's data object");
        let kinds = kinds_of(&object);
        assert!(kinds.contains(&"CF_HDROP".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"CF_UNICODETEXT".to_string()), "{kinds:?}");
        let back = payload_of(&object, DragActions::COPY_OR_MOVE);
        assert_eq!(back.paths, sent.paths);
        assert_eq!(back.allowed, DragActions::COPY_OR_MOVE);
        // (Files are there: the text that stands in for them is not
        // taken as text that was dragged.)
        assert_eq!(back.text, None);
        // The system reads the same list the same way.
        let by_system = with_data(&object, CF_HDROP.0, |memory, _| paths_by_system(memory));
        assert_eq!(by_system, Some(sent.paths));
    }

    #[test]
    fn text_and_a_link_are_carried_too() {
        let _ole = ole();
        let sent = DragPayload {
            text: Some("çay, şeker".to_string()),
            uris: vec!["https://example.org/a?b=c".to_string()],
            ..Default::default()
        };
        let object = object_for(&sent).expect("the shell's data object");
        let back = payload_of(&object, DragActions::COPY);
        assert_eq!(back.text, sent.text);
        assert_eq!(back.uris, sent.uris);
        assert!(back.paths.is_empty());
        assert!(kinds_of(&object).contains(&"UniformResourceLocatorW".to_string()));
    }

    #[test]
    fn an_object_keeps_what_was_done_with_it() {
        let _ole = ole();
        let object = object_for(&DragPayload::files([r"C:\a"])).expect("the shell's data object");
        assert_eq!(number(&object, CFSTR_PERFORMEDDROPEFFECT), None);
        put(
            &object,
            named(CFSTR_PERFORMEDDROPEFFECT),
            &2u32.to_le_bytes(),
        )
        .unwrap();
        assert_eq!(number(&object, CFSTR_PERFORMEDDROPEFFECT), Some(2));
    }

    #[test]
    fn the_shells_helper_keeps_a_drags_picture_in_the_object() {
        let _ole = ole();
        let object = object_for(&DragPayload::files([r"C:.txt"])).unwrap();
        let image = DragImage {
            width: 48,
            height: 32,
            rgba: [64, 224, 208, 200].repeat(48 * 32),
            hotspot: (24, 16),
            scale: 1.5,
        };
        picture(&object, &image).expect("the helper takes a picture");
        // (Kept in kinds of the helper's own, which only an object that
        // takes whatever it is given can hold.)
        let kinds = kinds_of(&object);
        assert!(kinds.contains(&"DragImageBits".to_string()), "{kinds:?}");
        // The files are still what is dragged.
        assert_eq!(payload_of(&object, DragActions::ALL).paths, [r"C:.txt"]);
    }

    #[test]
    fn a_target_answers_as_its_surface_said_and_tells_what_it_was_told() {
        let _ole = ole();
        // A window that is none: the target is called as Windows would
        // call it, and places stay as they are given.
        let window = 0x7E57;
        let target: IDropTarget = Target::new(window, false).into();
        let object = object_for(&DragPayload::files([r"C:\a.txt"])).unwrap();
        let at = POINTL { x: 30, y: 40 };
        let ask = |call: &dyn Fn(*mut DROPEFFECT)| {
            let mut effect = DROPEFFECT(7);
            call(&mut effect);
            effect.0
        };
        told();
        // Nothing was said for the window: nothing can be dropped.
        // SAFETY (all calls below): the pointers are to places that live
        // through the call, as Windows' own would.
        assert_eq!(
            ask(&|effect| unsafe { target.DragEnter(&object, MK_LBUTTON, at, effect).unwrap() }),
            0
        );
        unsafe { target.DragLeave().unwrap() };
        told();

        set_standing(
            HWND(window as *mut _),
            DropAnswer::accept(DragActions::COPY_OR_MOVE),
        );
        assert_eq!(
            ask(&|effect| unsafe { target.DragEnter(&object, MK_LBUTTON, at, effect).unwrap() }),
            1
        );
        // Shift moves; Ctrl and Shift ask for a link, which is not taken.
        assert_eq!(
            ask(&|effect| unsafe { target.DragOver(MK_LBUTTON | MK_SHIFT, at, effect).unwrap() }),
            2
        );
        assert_eq!(
            ask(&|effect| unsafe {
                target
                    .DragOver(MK_LBUTTON | MK_SHIFT | MK_CONTROL, at, effect)
                    .unwrap()
            }),
            0
        );
        // The surface says more of this drag.
        set_answer(
            HWND(window as *mut _),
            DropAnswer::preferring(DragActions::COPY_OR_MOVE, DragAction::Move),
        );
        let moved = POINTL { x: 31, y: 40 };
        assert_eq!(
            ask(&|effect| unsafe { target.DragOver(MK_LBUTTON, moved, effect).unwrap() }),
            2
        );
        // Dropped as a move: the source is told to delete nothing, and
        // can read that it was a move.
        assert_eq!(
            ask(&|effect| unsafe {
                target
                    .Drop(&object, MODIFIERKEYS_FLAGS(0), moved, effect)
                    .unwrap()
            }),
            0
        );
        assert_eq!(number(&object, CFSTR_PERFORMEDDROPEFFECT), Some(0));
        assert_eq!(number(&object, CFSTR_LOGICALPERFORMEDDROPEFFECT), Some(2));
        assert_eq!(
            dnd::outcome(true, 0, Some(0), Some(2)),
            DragOutcome::Taken {
                action: DragAction::Move,
                remove: false
            }
        );

        let heard = told();
        assert_eq!(heard.len(), 3, "entered, moved once, dropped");
        match &heard[0] {
            Told::Entered {
                window: on,
                x,
                y,
                offered,
            } => {
                assert_eq!((*on, *x, *y), (window, 30, 40));
                assert!(offered.files);
                assert_eq!(offered.allowed, DragActions::ALL);
                assert_eq!(offered.payload.as_ref().unwrap().paths, [r"C:\a.txt"]);
            }
            _ => panic!("the first thing told is that it came"),
        }
        assert!(matches!(heard[1], Told::Moved { x: 31, y: 40, .. }));
        match &heard[2] {
            Told::Dropped {
                payload, action, x, ..
            } => {
                assert_eq!(payload.paths, [r"C:\a.txt"]);
                assert_eq!(*action, DragAction::Move);
                assert_eq!(*x, 31);
            }
            _ => panic!("the last thing told is the drop"),
        }
        // What was said of that drag is not held against the next.
        assert_eq!(
            ask(&|effect| unsafe { target.DragEnter(&object, MK_LBUTTON, at, effect).unwrap() }),
            1
        );
        // A drop where the keys ask for what is not taken is no drop.
        assert_eq!(
            ask(&|effect| unsafe {
                target
                    .Drop(&object, MK_SHIFT | MK_CONTROL, at, effect)
                    .unwrap()
            }),
            0
        );
        assert!(matches!(told().last(), Some(Told::Left { .. })));
        revoke(HWND(window as *mut _));
    }
}
