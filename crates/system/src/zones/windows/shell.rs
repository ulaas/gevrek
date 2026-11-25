//! The shell's namespace: what is on the desktop and in a folder, as the
//! shell itself lists it. The desktop of the shell is more than a folder:
//! it is the person's Desktop folder and the one of all users laid over
//! each other, and things that are no files at all (This PC, the Recycle
//! Bin). Only the shell knows all of that, the names it shows for them
//! and which of them the person has switched off; so it is asked, and no
//! folder is read by hand.
//!
//! An item is known by its parsing name: the text the shell takes back
//! and finds the same item by (`C:\Users\ada\Desktop\a.txt`,
//! `::{20D04FE0-3AEA-1069-A2D8-08002B30309D}` for This PC). It stays the
//! same over restarts, which is what an `ItemId` must do.

use super::facts::{
    class_of, is_path, kind_of, listed, same_path, seconds, shown_on_desktop, type_of, Nature,
    FOLDER_TYPE,
};
use crate::com::Com;
use gevrek_zones::backend::{BackendError, Location};
use gevrek_zones::ids::ItemId;
use gevrek_zones::item::{extension_of, Item, ItemFacts, ItemKind};
use std::collections::HashMap;
use windows::core::{w, Interface, GUID, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, RRF_RT_REG_DWORD,
};
use windows::Win32::System::SystemServices::{
    SFGAO_FILESYSTEM, SFGAO_FOLDER, SFGAO_LINK, SFGAO_STREAM,
};
use windows::Win32::UI::Shell::Common::{
    ITEMIDLIST, PERCEIVED, PERCEIVED_TYPE_APPLICATION, PERCEIVED_TYPE_AUDIO,
    PERCEIVED_TYPE_COMPRESSED, PERCEIVED_TYPE_DOCUMENT, PERCEIVED_TYPE_IMAGE,
    PERCEIVED_TYPE_SYSTEM, PERCEIVED_TYPE_TEXT, PERCEIVED_TYPE_VIDEO,
};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::Shell::{
    AssocGetPerceivedType, AssocQueryStringW, BHID_SFObject, FOLDERID_Desktop,
    FOLDERID_PublicDesktop, IEnumIDList, ILFree, IShellFolder, IShellItem, IShellItem2,
    SHCreateItemFromIDList, SHCreateItemFromParsingName, SHCreateItemWithParent,
    SHGetDesktopFolder, SHGetKnownFolderPath, SHGetNameFromIDList, SHGetSetSettings,
    SHParseDisplayName, ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_CONTENTTYPE, KF_FLAG_DEFAULT,
    SHCONTF_FOLDERS, SHCONTF_INCLUDEHIDDEN, SHCONTF_NONFOLDERS, SHELLSTATEA,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY, SSF_SHOWALLOBJECTS,
};

/// Where a shortcut leads, as the shell's property of that name
/// (`System.Link.TargetParsingPath`).
const LINK_TARGET: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25),
    pid: 2,
};

/// A text Windows made room for itself; the room is given back.
pub(super) unsafe fn taken(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    let read = text.to_string().unwrap_or_default();
    CoTaskMemFree(Some(text.0 as *const _));
    read
}

/// The shell's own name for an item (an "item id list"): the way through
/// the namespace to it, from the desktop down. Given back with this.
pub struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    /// Take over a list the shell made room for.
    ///
    /// # Safety
    /// `list` is null or a list the shell allocated, and nobody else
    /// frees it.
    pub unsafe fn from_raw(list: *mut ITEMIDLIST) -> Option<Self> {
        (!list.is_null()).then_some(Self(list))
    }

    /// The list of the item a parsing name names. Nothing when there is
    /// no such item (a file that went).
    pub fn parse(name: &str) -> Option<Self> {
        let mut list = std::ptr::null_mut();
        // SAFETY: the name ends with a zero; the list is ours to receive.
        unsafe {
            SHParseDisplayName(&HSTRING::from(plain(name)), None, &mut list, 0, None).ok()?;
            Self::from_raw(list)
        }
    }

    pub fn as_ptr(&self) -> *const ITEMIDLIST {
        self.0
    }

    /// The parsing name of the item this names. It is made of the list
    /// alone: the item need not be there any more.
    pub fn parsing_name(&self) -> Option<String> {
        // SAFETY: the list is one of the shell's.
        let name =
            unsafe { taken(SHGetNameFromIDList(self.0, SIGDN_DESKTOPABSOLUTEPARSING).ok()?) };
        (!name.is_empty()).then_some(name)
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: the list was allocated by the shell and is ours alone.
        unsafe { ILFree(Some(self.0)) };
    }
}

/// A path as the shell reads it: with Windows' own separator. (A folder
/// may be written with the other in the configuration.)
pub(super) fn plain(name: &str) -> String {
    if is_path(name) {
        name.replace('/', "\\")
    } else {
        name.to_string()
    }
}

/// The types of files by their endings, as the registry has them. Asked
/// once for each ending: a desktop has many files of few kinds.
///
/// With them goes what else the facts of a file are read with: when the
/// person last opened each file (`recent::Opened`), which whoever reads
/// the shell's record of that shares with everyone who reads facts.
#[derive(Debug, Default)]
pub struct Types {
    known: HashMap<String, Option<String>>,
    pub opened: super::recent::Opened,
}

impl Types {
    /// With a record of opened files that is shared.
    pub fn knowing(opened: super::recent::Opened) -> Self {
        Self {
            known: HashMap::new(),
            opened,
        }
    }

    /// The type of files that end in `extension` (without the dot, in
    /// small letters).
    pub fn of(&mut self, extension: &str) -> Option<String> {
        if let Some(known) = self.known.get(extension) {
            return known.clone();
        }
        let dotted = HSTRING::from(format!(".{extension}"));
        let found = type_of(
            extension,
            content_type(&dotted).as_deref(),
            perceived(&dotted),
        );
        self.known.insert(extension.to_string(), found.clone());
        found
    }
}

/// The content type the registry has for an ending (`.txt`).
fn content_type(dotted: &HSTRING) -> Option<String> {
    let mut buffer = [0u16; 256];
    let mut length = buffer.len() as u32;
    // SAFETY: the buffer is as long as `length` says.
    let answered = unsafe {
        AssocQueryStringW(
            ASSOCF_INIT_IGNOREUNKNOWN,
            ASSOCSTR_CONTENTTYPE,
            dotted,
            PCWSTR::null(),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    if answered.is_err() {
        return None;
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    let text = String::from_utf16_lossy(&buffer[..end]);
    (!text.is_empty()).then_some(text)
}

/// What sort of thing Windows takes files of an ending to be.
fn perceived(dotted: &HSTRING) -> Option<&'static str> {
    let mut sort = PERCEIVED::default();
    let mut how = 0u32;
    // SAFETY: both are ours to receive; the text is not asked for.
    unsafe { AssocGetPerceivedType(dotted, &mut sort, &mut how, None) }.ok()?;
    Some(match sort {
        PERCEIVED_TYPE_TEXT => "text",
        PERCEIVED_TYPE_IMAGE => "image",
        PERCEIVED_TYPE_AUDIO => "audio",
        PERCEIVED_TYPE_VIDEO => "video",
        PERCEIVED_TYPE_COMPRESSED => "compressed",
        PERCEIVED_TYPE_DOCUMENT => "document",
        PERCEIVED_TYPE_SYSTEM => "system",
        PERCEIVED_TYPE_APPLICATION => "application",
        _ => return None,
    })
}

fn missing(what: &str) -> BackendError {
    BackendError::NotFound {
        what: what.to_string(),
    }
}

/// The shell's item of a parsing name.
pub(super) unsafe fn shell_item(id: &str) -> Option<IShellItem> {
    SHCreateItemFromParsingName(&HSTRING::from(plain(id)), None).ok()
}

/// The shell's folder of a location.
unsafe fn folder_of(location: &Location) -> Result<IShellFolder, BackendError> {
    match location {
        Location::Desktop => SHGetDesktopFolder().map_err(|_| missing("the desktop")),
        Location::Directory { path } => {
            if !std::path::Path::new(&plain(path)).is_dir() {
                return Err(missing(path));
            }
            let item = shell_item(path).ok_or_else(|| missing(path))?;
            item.BindToHandler::<_, IShellFolder>(None, &BHID_SFObject)
                .map_err(|_| missing(path))
        }
    }
}

/// Whether the person has Explorer show hidden files. What Explorer
/// shows of a folder is what a zone shows of it.
fn hidden_shown() -> bool {
    let mut state = SHELLSTATEA::default();
    // SAFETY: the structure is ours; only read (`bset` is false).
    unsafe { SHGetSetSettings(Some(&mut state), SSF_SHOWALLOBJECTS, false) };
    // The first of its flags.
    state._bitfield1 & 1 != 0
}

/// What is in a folder of the shell, each as a list relative to it.
unsafe fn children(folder: &IShellFolder) -> Vec<Pidl> {
    let mut wanted = SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0;
    if hidden_shown() {
        wanted |= SHCONTF_INCLUDEHIDDEN.0;
    }
    let mut list: Option<IEnumIDList> = None;
    // (A folder with nothing to list answers "no" without an error and
    // gives no list.)
    if folder
        .EnumObjects(HWND::default(), wanted as u32, &mut list)
        .is_err()
    {
        return Vec::new();
    }
    let Some(list) = list else {
        return Vec::new();
    };
    let mut found = Vec::new();
    loop {
        let mut next = [std::ptr::null_mut(); 1];
        let mut fetched = 0u32;
        if list.Next(&mut next, Some(&mut fetched)).is_err() || fetched == 0 {
            break;
        }
        if let Some(child) = Pidl::from_raw(next[0]) {
            found.push(child);
        }
    }
    found
}

/// Everything at a location, as the shell lists it.
///
/// Of the desktop that is what lies on it as icons, not everything the
/// shell's namespace has directly under it: the namespace begins at the
/// desktop, so the person's folders (Documents, Downloads) and things
/// such as Network or Control Panel are there too, as the roots of the
/// tree Explorer shows at its left. See [`on_the_desktop`].
pub fn list(location: &Location, types: &mut Types) -> Result<Vec<Item>, BackendError> {
    let _com = Com::new();
    // SAFETY: the shell's objects are used on this thread alone, which
    // may use them while `_com` lives.
    unsafe {
        let folder = folder_of(location)?;
        let desktop = matches!(location, Location::Desktop).then(desktop_folders);
        let mut items = Vec::new();
        for child in children(&folder) {
            let Ok(shell_item) =
                SHCreateItemWithParent::<_, IShellItem>(None, &folder, child.as_ptr())
            else {
                continue;
            };
            let Some(item) = item_of(&shell_item, types) else {
                continue;
            };
            if desktop
                .as_ref()
                .is_none_or(|folders| on_the_desktop(item.id.as_str(), folders))
            {
                items.push(item);
            }
        }
        Ok(items)
    }
}

/// The two folders the desktop is made of: the person's, and the one of
/// all users.
pub fn desktop_folders() -> Vec<String> {
    [&FOLDERID_Desktop, &FOLDERID_PublicDesktop]
        .into_iter()
        .filter_map(|folder| {
            // SAFETY: the text is the shell's to give and ours to free.
            let path =
                unsafe { SHGetKnownFolderPath(folder, KF_FLAG_DEFAULT, HANDLE::default()) }.ok()?;
            let path = unsafe { taken(path) };
            (!path.is_empty()).then_some(path)
        })
        .collect()
}

/// Whether something the shell has directly under the desktop lies on
/// the desktop as an icon: a file or folder that is in one of the
/// desktop's two folders, or a thing that is no file and that the
/// person's settings show there.
pub fn on_the_desktop(id: &str, folders: &[String]) -> bool {
    if is_path(id) {
        return gevrek_zones::paths::parent(id)
            .is_some_and(|parent| folders.iter().any(|folder| same_path(parent, folder)));
    }
    let Some(class) = class_of(id) else {
        return false;
    };
    let registered = registered_on_desktop(HKEY_CURRENT_USER, class)
        || registered_on_desktop(HKEY_LOCAL_MACHINE, class);
    shown_on_desktop(
        id,
        hidden_on_desktop(HKEY_CURRENT_USER, class),
        hidden_on_desktop(HKEY_LOCAL_MACHINE, class),
        registered,
    )
}

/// Whether a class is one a program put onto the desktop: Explorer shows
/// what is registered under its `Desktop\NameSpace` key (the person's or
/// the machine's) as an icon there, unless the settings above hide it.
/// This is how a program that is no part of Windows gets a thing that is
/// no file onto the desktop.
fn registered_on_desktop(root: HKEY, class: &str) -> bool {
    let key = HSTRING::from(format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Desktop\\NameSpace\\{class}"
    ));
    let mut opened = HKEY::default();
    // SAFETY: the key is opened for reading only and closed again.
    unsafe {
        if RegOpenKeyExW(root, &key, 0, KEY_READ, &mut opened).is_err() {
            return false;
        }
        let _ = RegCloseKey(opened);
    }
    true
}

/// What the registry says of a class under the person's or the machine's
/// settings of the desktop's icons: 1 for hidden, 0 for shown, nothing
/// where it says nothing.
fn hidden_on_desktop(root: HKEY, class: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: the value is read into a number of our own, whose size is
    // said; a value that is no number is not read at all.
    let read = unsafe {
        RegGetValueW(
            root,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\HideDesktopIcons\\NewStartPanel"),
            &HSTRING::from(class),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    read.is_ok().then_some(value)
}

/// The item a parsing name names, with its facts read now.
pub fn item_by_id(id: &ItemId, types: &mut Types) -> Option<Item> {
    let _com = Com::new();
    // SAFETY: as in `list`.
    unsafe { item_of(&shell_item(id.as_str())?, types) }
}

/// As `item_by_id`, for a change the file system told of: nothing for a
/// file a listing would not show (a hidden one, unless the person has
/// hidden files shown). The shell leaves those out when it lists; the
/// file system tells of every file.
pub fn item_if_listed(id: &ItemId, types: &mut Types) -> Option<Item> {
    use std::os::windows::fs::MetadataExt;
    let attributes = std::fs::symlink_metadata(plain(id.as_str()))
        .ok()?
        .file_attributes();
    if !listed(attributes, hidden_shown()) {
        return None;
    }
    item_by_id(id, types)
}

/// The item a list of the shell's names (what a change is told with).
pub fn item_by_pidl(pidl: &Pidl, types: &mut Types) -> Option<Item> {
    let _com = Com::new();
    // SAFETY: as in `list`; the list is one of the shell's.
    unsafe {
        let shell_item: IShellItem = SHCreateItemFromIDList(pidl.as_ptr()).ok()?;
        item_of(&shell_item, types)
    }
}

/// An item as the core knows it, of the shell's item.
unsafe fn item_of(shell_item: &IShellItem, types: &mut Types) -> Option<Item> {
    let id = taken(
        shell_item
            .GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)
            .ok()?,
    );
    if id.is_empty() {
        return None;
    }
    let said = shell_item
        .GetAttributes(SFGAO_FILESYSTEM | SFGAO_FOLDER | SFGAO_STREAM | SFGAO_LINK)
        .unwrap_or_default();
    let has = |flag: u32| said.0 & flag != 0;
    let kind = kind_of(Nature {
        file_system: has(SFGAO_FILESYSTEM.0) && is_path(&id),
        folder: has(SFGAO_FOLDER.0),
        stream: has(SFGAO_STREAM.0),
        link: has(SFGAO_LINK.0),
    });
    let path = (kind != ItemKind::Virtual).then(|| id.clone());
    let mut name = shell_item
        .GetDisplayName(SIGDN_NORMALDISPLAY)
        .map(|name| taken(name))
        .unwrap_or_default();
    if name.is_empty() {
        name = gevrek_zones::paths::file_name(&id).to_string();
    }
    let mut facts = path
        .as_deref()
        .map(|path| facts_of_path(path, kind, types))
        .unwrap_or_default();
    if kind == ItemKind::Shortcut {
        facts.shortcut_target = shell_item
            .cast::<IShellItem2>()
            .ok()
            .and_then(|item| item.GetString(&LINK_TARGET).ok())
            .map(|target| taken(target))
            .filter(|target| !target.is_empty());
    }
    Some(Item {
        id: ItemId::new(id),
        kind,
        name,
        path,
        facts,
    })
}

/// What the file system says of a file or folder, and the type its
/// ending has. The ending is the real one, of the file's name: the name
/// the shell shows may leave it out.
///
/// Reading these fetches nothing of a file a cloud keeps: they are the
/// file's name, size and times, which are on this machine either way.
///
/// `accessed` is when the person last opened the file, as the shell
/// wrote it down, and absent where it did not: the file system's own
/// time of last access is not to be trusted on Windows (see `recent`).
/// So "untouched for so many days" counts from the latest of made,
/// written and opened.
fn facts_of_path(path: &str, kind: ItemKind, types: &mut Types) -> ItemFacts {
    let mut facts = ItemFacts::default();
    if kind == ItemKind::Folder {
        facts.mime = Some(FOLDER_TYPE.to_string());
    } else {
        facts.extension = extension_of(gevrek_zones::paths::file_name(path));
        facts.mime = facts
            .extension
            .as_deref()
            .and_then(|extension| types.of(extension));
    }
    if let Ok(said) = std::fs::metadata(path) {
        if said.is_file() {
            facts.size = Some(said.len());
        }
        facts.created = seconds(said.created());
        facts.modified = seconds(said.modified());
    }
    facts.accessed = types.opened.at(path);
    facts
}
