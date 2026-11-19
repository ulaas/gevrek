//! What a drag carries: the data half of drag and drop.
//!
//! Windows carries what is dragged in a data object that holds the same
//! thing in several kinds at once; each program takes the kind it
//! understands. For files those are the list of dropped files (`CF_HDROP`:
//! paths) and the shell's own names for the items ("Shell IDList Array":
//! also for what has no path, such as This PC). The shell makes such an
//! object for any items of its namespace, with every kind Explorer and
//! other programs expect; nothing of it is written by hand here.
//!
//! One kind is added that is gevrek's own: the ids of the items, for a
//! drag that began in a zone. A zone a drag ends on then knows the items
//! themselves (and can move them between zones without touching a file),
//! where for a drag from another program it only knows paths.
//!
//! The other half (the window that takes drops, and the loop that runs
//! while something is dragged out) is `gevrek_shellkit`'s. It calls:
//!
//! - [`data_object`] with the items a drag out of a zone carries, and
//!   hands the answer to `DoDragDrop`;
//! - [`payload_of`] with the object a drag over the zones' surface
//!   brought (`IDropTarget::DragEnter` has it) and what the source
//!   allows; the answer goes to the backend (`drag_is_over`), where the
//!   core asks for it;
//! - [`effects_of`] and [`actions_of`] between the core's three actions
//!   and Windows' count of them (`DROPEFFECT`);
//! - [`items_of_id_list`], where it has only the bytes of the shell's
//!   kind and wants to know which items they name.

use super::shell::Pidl;
use gevrek_zones::backend::{BackendError, DragActions, DropPayload};
use gevrek_zones::ids::ItemId;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, ILCombine, SHCreateShellItemArrayFromIDLists,
};

/// The name of gevrek's own kind of data.
const ITEMS: PCWSTR = w!("gevrek.zones.items");
/// The name of the shell's kind: its own names for the items.
const ID_LIST: PCWSTR = w!("Shell IDList Array");

/// Windows' count of what may be done with what is dragged: copy 1, move
/// 2, link 4.
pub fn effects_of(actions: DragActions) -> u32 {
    u32::from(actions.copy) | u32::from(actions.moving) << 1 | u32::from(actions.link) << 2
}

/// The core's three actions of Windows' count; what else is in it (that
/// the target scrolls) is no action.
pub fn actions_of(effects: u32) -> DragActions {
    DragActions {
        copy: effects & 1 != 0,
        moving: effects & 2 != 0,
        link: effects & 4 != 0,
    }
}

/// gevrek's own kind, written: which program began the drag, and the ids
/// of the items, as JSON.
pub fn encode_items(process: u32, items: &[ItemId]) -> Vec<u8> {
    serde_json::json!({ "process": process, "items": items })
        .to_string()
        .into_bytes()
}

/// gevrek's own kind, read: the items, when it was this program that
/// began the drag. Another gevrek's ids (another session's, say) name
/// items of a desktop that is not this one.
pub fn decode_items(bytes: &[u8], process: u32) -> Option<Vec<ItemId>> {
    // (Memory of Windows may be longer than what was put into it.)
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    let read: serde_json::Value = serde_json::from_slice(&bytes[..end]).ok()?;
    if read.get("process")?.as_u64()? != u64::from(process) {
        return None;
    }
    serde_json::from_value(read.get("items")?.clone()).ok()
}

/// The paths in a list of dropped files (`DROPFILES`): a head that says
/// where the names begin and whether they are wide, then each name ended
/// by a zero, and one more zero after the last.
pub fn paths_of_hdrop(bytes: &[u8]) -> Option<Vec<String>> {
    let number = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let (begin, wide) = (number(0)? as usize, number(16)? != 0);
    let names = bytes.get(begin..)?;
    let mut paths = Vec::new();
    if wide {
        let units: Vec<u16> = names
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        for name in units.split(|unit| *unit == 0) {
            if name.is_empty() {
                break;
            }
            paths.push(String::from_utf16_lossy(name));
        }
    } else {
        for name in names.split(|byte| *byte == 0) {
            if name.is_empty() {
                break;
            }
            paths.push(String::from_utf8_lossy(name).into_owned());
        }
    }
    Some(paths)
}

/// Where the lists begin in the shell's kind (`CIDA`): a count, then
/// that many places and one more. The first is the folder the items lie
/// in, each of the others an item relative to it. Nothing for bytes that
/// are no such thing.
pub fn id_list_offsets(bytes: &[u8]) -> Option<(usize, Vec<usize>)> {
    let number = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize);
    let count = number(0)?;
    // (Not more than could be there: a count read from garbage.)
    if count > bytes.len() / 4 {
        return None;
    }
    let mut places = Vec::with_capacity(count + 1);
    for index in 0..=count {
        let place = number(4 + index * 4)?;
        // Every list has at least the two bytes that end it.
        if place + 2 > bytes.len() {
            return None;
        }
        places.push(place);
    }
    let folder = places.remove(0);
    Some((folder, places))
}

/// The items the shell's kind names, by their ids.
pub fn items_of_id_list(bytes: &[u8]) -> Vec<ItemId> {
    let Some((folder, items)) = id_list_offsets(bytes) else {
        return Vec::new();
    };
    let _com = crate::com::Com::new();
    let at = |place: usize| bytes[place..].as_ptr() as *const ITEMIDLIST;
    items
        .into_iter()
        .filter_map(|item| {
            // SAFETY: both places were checked to lie within the bytes,
            // which the shell wrote as lists that end; the joined list is
            // ours and freed with `Pidl`.
            let whole = unsafe { Pidl::from_raw(ILCombine(Some(at(folder)), Some(at(item)))) }?;
            whole.parsing_name().map(ItemId::new)
        })
        .collect()
}

fn kind(name: PCWSTR) -> u16 {
    // SAFETY: the name ends with a zero.
    unsafe { RegisterClipboardFormatW(name) as u16 }
}

fn in_memory(kind: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: kind,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// The bytes an object holds of one kind.
fn bytes_of(object: &IDataObject, kind: u16) -> Option<Vec<u8>> {
    // SAFETY: the object gives memory of its own, which is read while it
    // is locked and given back as the object asks.
    unsafe {
        let mut medium = object.GetData(&in_memory(kind)).ok()?;
        let mut bytes = None;
        if medium.tymed == TYMED_HGLOBAL.0 as u32 && !medium.u.hGlobal.is_invalid() {
            let memory = medium.u.hGlobal;
            let length = GlobalSize(memory);
            let start = GlobalLock(memory) as *const u8;
            if !start.is_null() {
                bytes = Some(std::slice::from_raw_parts(start, length).to_vec());
                let _ = GlobalUnlock(memory);
            }
        }
        ReleaseStgMedium(&mut medium);
        bytes
    }
}

/// Put bytes of one kind into an object. The object takes the memory
/// over.
fn put(object: &IDataObject, kind: u16, bytes: &[u8]) -> windows::core::Result<()> {
    // SAFETY: memory of the length of the bytes, filled while locked, and
    // handed to the object, which frees it (`frelease`).
    unsafe {
        let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))?;
        let start = GlobalLock(memory) as *mut u8;
        if start.is_null() {
            return Err(windows::core::Error::from_win32());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), start, bytes.len());
        let _ = GlobalUnlock(memory);
        let mut medium: STGMEDIUM = std::mem::zeroed();
        medium.tymed = TYMED_HGLOBAL.0 as u32;
        medium.u.hGlobal = memory;
        object.SetData(&in_memory(kind), &medium, true)
    }
}

/// The object a drag of these items carries: the shell's own for them,
/// with every kind other programs take (paths, the shell's names, what
/// Explorer needs to copy or link), and gevrek's kind beside them.
///
/// The calling thread must be one that may use the shell's objects for as
/// long as the object lives (the thread a drag is begun on is).
pub fn data_object(asked: &[ItemId]) -> Result<IDataObject, BackendError> {
    let items = asked;
    // (What is not there any more is left out of what is carried.)
    let (items, lists): (Vec<ItemId>, Vec<Pidl>) = items
        .iter()
        .filter_map(|item| Some((item.clone(), Pidl::parse(item.as_str())?)))
        .unzip();
    if lists.is_empty() {
        return Err(match asked.first() {
            Some(item) => BackendError::NotFound { what: item.as_str().to_string() },
            None => BackendError::Unsupported,
        });
    }
    let raw: Vec<*const ITEMIDLIST> = lists.iter().map(Pidl::as_ptr).collect();
    let refused = |error: windows::core::Error| BackendError::Refused { what: error.message() };
    // SAFETY: the lists live until the object is made, which copies them.
    let object: IDataObject = unsafe {
        SHCreateShellItemArrayFromIDLists(&raw)
            .and_then(|together| together.BindToHandler(None, &BHID_DataObject))
            .map_err(refused)?
    };
    // (An object that would not take it still carries everything other
    // programs need; only zones would see paths instead of items.)
    if let Err(error) = put(&object, kind(ITEMS), &encode_items(std::process::id(), &items)) {
        log::warn!("zones: what is dragged does not carry its items: {error}");
    }
    Ok(object)
}

/// What an object carries, as the core wants it: the items when the drag
/// began in a zone of this program, paths of files, or text. `allowed` is
/// what the source lets be done (Windows says so beside the object, not
/// in it).
pub fn payload_of(object: &IDataObject, allowed: DragActions) -> DropPayload {
    let items = bytes_of(object, kind(ITEMS))
        .and_then(|bytes| decode_items(&bytes, std::process::id()))
        .unwrap_or_default();
    let paths = bytes_of(object, CF_HDROP.0)
        .and_then(|bytes| paths_of_hdrop(&bytes))
        .unwrap_or_default();
    // Text counts only where there is nothing else: a dragged file often
    // carries its path as text as well.
    let text = (items.is_empty() && paths.is_empty())
        .then(|| bytes_of(object, CF_UNICODETEXT.0))
        .flatten()
        .map(|bytes| {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .take_while(|unit| *unit != 0)
                .collect();
            String::from_utf16_lossy(&units)
        })
        .filter(|text| !text.is_empty());
    DropPayload { items, paths, text, allowed }
}

/// The kinds an object has that the zones read, by what they are: for
/// looking at what a drag carries.
pub fn kinds_read(object: &IDataObject) -> Vec<&'static str> {
    [
        ("paths", CF_HDROP.0),
        ("the shell's names", kind(ID_LIST)),
        ("gevrek's items", kind(ITEMS)),
        ("text", CF_UNICODETEXT.0),
    ]
    .into_iter()
    // SAFETY: only asks whether the object has data of that kind.
    .filter(|(_, kind)| unsafe { object.QueryGetData(&in_memory(*kind)) }.is_ok())
    .map(|(name, _)| name)
    .collect()
}

/// The bytes of the shell's kind in an object, if it has them.
pub fn id_list_of(object: &IDataObject) -> Option<Vec<u8>> {
    bytes_of(object, kind(ID_LIST))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_actions_are_windows_three_effects() {
        for (actions, effects) in [
            (DragActions::ALL, 7),
            (DragActions::default(), 0),
            (DragActions { copy: true, ..Default::default() }, 1),
            (DragActions { moving: true, ..Default::default() }, 2),
            (DragActions { link: true, ..Default::default() }, 4),
            (DragActions { copy: true, moving: true, link: false }, 3),
        ] {
            assert_eq!(effects_of(actions), effects);
            assert_eq!(actions_of(effects), actions);
        }
        // "The target scrolls" is no action.
        assert_eq!(actions_of(0x8000_0001), DragActions { copy: true, ..Default::default() });
    }

    #[test]
    fn the_items_of_a_drag_are_read_back_by_the_program_that_began_it() {
        let items = [ItemId::new("C:\\Users\\ada\\Desktop\\a \"b\".txt"), ItemId::new("::{645FF040-5081-101B-9F08-00AA002F954E}")];
        let mut bytes = encode_items(42, &items);
        assert_eq!(decode_items(&bytes, 42).as_deref(), Some(&items[..]));
        // Memory that is longer than what was written into it.
        bytes.extend([0, 0, 0, 7]);
        assert_eq!(decode_items(&bytes, 42).as_deref(), Some(&items[..]));
        // Another program's, and what is no such thing.
        assert_eq!(decode_items(&bytes, 43), None);
        assert_eq!(decode_items(b"not json", 42), None);
        assert_eq!(decode_items(b"{\"items\":[]}", 42), None);
    }

    fn hdrop(names: &[&str], wide: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(20u32.to_le_bytes());
        bytes.extend([0u8; 12]);
        bytes.extend(u32::from(wide).to_le_bytes());
        for name in names {
            if wide {
                bytes.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
                bytes.extend([0, 0]);
            } else {
                bytes.extend(name.bytes());
                bytes.push(0);
            }
        }
        bytes.extend(if wide { vec![0, 0] } else { vec![0] });
        bytes
    }

    #[test]
    fn a_list_of_dropped_files_is_read_wide_and_narrow() {
        let names = ["C:\\Users\\ada\\Desktop\\a.txt", "D:\\Bilder\\Größe.png"];
        assert_eq!(paths_of_hdrop(&hdrop(&names, true)).expect("read"), names);
        assert_eq!(paths_of_hdrop(&hdrop(&names[..1], false)).expect("read"), names[..1]);
        assert_eq!(paths_of_hdrop(&hdrop(&[], true)).expect("read"), Vec::<String>::new());
        // Cut short, or a head that points past the end.
        assert_eq!(paths_of_hdrop(&[1, 2, 3]), None);
        let mut far = hdrop(&names, true);
        far[0..4].copy_from_slice(&9999u32.to_le_bytes());
        assert_eq!(paths_of_hdrop(&far), None);
    }

    #[test]
    fn the_places_in_the_shells_kind_are_read_and_garbage_is_not() {
        // Two items in a folder: the count, three places, three lists
        // (here each only its two ending bytes).
        let mut bytes = Vec::new();
        for number in [2u32, 16, 18, 20] {
            bytes.extend(number.to_le_bytes());
        }
        bytes.extend([0u8; 6]);
        assert_eq!(id_list_offsets(&bytes), Some((16, vec![18, 20])));
        // A place past the end; a count that cannot be; too short.
        let mut past = bytes.clone();
        past[12..16].copy_from_slice(&21u32.to_le_bytes());
        assert_eq!(id_list_offsets(&past), None);
        let mut many = bytes.clone();
        many[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(id_list_offsets(&many), None);
        assert_eq!(id_list_offsets(&bytes[..6]), None);
        assert!(items_of_id_list(&past).is_empty());
    }

    #[test]
    fn the_shells_object_for_files_carries_paths_names_and_the_items() {
        let folder = std::env::temp_dir().join(format!("gevrek-zones-dnd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("inner")).expect("folders");
        let folder = std::fs::canonicalize(&folder)
            .expect("the folder")
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .to_string();
        let files = [format!("{folder}\\a.txt"), format!("{folder}\\inner\\b c.txt")];
        for file in &files {
            std::fs::write(file, b"x").expect("written");
        }
        let _com = crate::com::Com::new();
        let items: Vec<ItemId> = files.iter().map(|file| ItemId::new(file.clone())).collect();
        let object = data_object(&items).expect("an object");
        let kinds = kinds_read(&object);
        assert!(kinds.contains(&"paths") && kinds.contains(&"the shell's names") && kinds.contains(&"gevrek's items"), "{kinds:?}");
        let payload = payload_of(&object, DragActions::ALL);
        assert_eq!(payload.items, items);
        assert_eq!(payload.paths, files);
        assert_eq!(payload.text, None);
        assert_eq!(payload.allowed, DragActions::ALL);
        // The shell's own names say the same.
        let named = items_of_id_list(&id_list_of(&object).expect("the shell's kind"));
        assert_eq!(named, items);

        // What is not there cannot be dragged; one of two that is, can.
        let gone = ItemId::new(format!("{folder}\\gone.txt"));
        assert!(matches!(data_object(std::slice::from_ref(&gone)), Err(BackendError::NotFound { .. })));
        assert_eq!(data_object(&[]).err(), Some(BackendError::Unsupported));
        let one = data_object(&[gone, items[0].clone()]).expect("an object");
        assert_eq!(payload_of(&one, DragActions::default()).paths, files[..1]);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
