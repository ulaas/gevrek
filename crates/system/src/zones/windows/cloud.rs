//! Files a cloud keeps (OneDrive's "Files On-Demand", and every other
//! program that syncs through Windows' cloud files).
//!
//! Such a folder holds files that are not on this machine: only their
//! names, sizes and times are, and the bytes are fetched the moment a
//! program reads them. A zone that shows the folder must not be that
//! program: showing a thousand names must not download a thousand files.
//!
//! Windows marks such a file in its attributes, which are read with its
//! name and cost nothing:
//!
//! | Attribute | Means |
//! |---|---|
//! | `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` | reading its bytes fetches it |
//! | `FILE_ATTRIBUTE_RECALL_ON_OPEN` | even opening it fetches it |
//! | `FILE_ATTRIBUTE_OFFLINE` | the older word for "not here" |
//! | `FILE_ATTRIBUTE_PINNED` | "always keep on this device" |
//! | `FILE_ATTRIBUTE_UNPINNED` | "free up space": may be sent away again |
//!
//! What is done with that here:
//!
//! - **Listing and facts** ask for names, attributes, sizes and times
//!   only. None of them reads a file, so nothing is fetched. The type is
//!   told from the ending.
//! - **The icon** of a file that is not here is the icon of its kind,
//!   asked for of a made-up name with the same ending: the shell is never
//!   handed the real file, so it cannot read it for a picture of its own
//!   (a program's own icon lies inside its file).
//! - **The small picture** of a file that is not here is only what
//!   Windows already has in its store of small pictures
//!   (`SIIGBF_INCACHEONLY`); where it has none the answer is none, and
//!   the icon is shown.
//! - **Opening** it is the person's own wish, and fetches it as it would
//!   from Explorer.
//!
//! No sign is drawn on the icon here (Explorer's cloud and tick are its
//! own). [`state_of_path`] says how a file stands, for a renderer that
//! wants to draw one.

use crate::com::Com;
use crate::icons::Picture;
use windows::core::{implement, Result as ComResult, HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
use windows::Win32::System::Com::{CreateBindCtx, IBindCtx};
use windows::Win32::UI::Shell::{
    IFileSystemBindData, IFileSystemBindData_Impl, IShellItemImageFactory,
    SHCreateItemFromParsingName, SIIGBF_ICONONLY, SIIGBF_INCACHEONLY, SIIGBF_THUMBNAILONLY,
};

const OFFLINE: u32 = 0x1000;
const RECALL_ON_OPEN: u32 = 0x4_0000;
const PINNED: u32 = 0x8_0000;
const UNPINNED: u32 = 0x10_0000;
const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
const NORMAL: u32 = 0x80;

/// How a file a cloud keeps stands with this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudState {
    /// Its bytes are not here; reading them fetches them.
    OnlineOnly,
    /// It is here and the person asked that it always be.
    AlwaysKept,
    /// It is here now, and may be sent away again when room is needed.
    Here,
}

impl CloudState {
    /// Whether reading the file's bytes would fetch them from elsewhere.
    pub fn fetched_when_read(self) -> bool {
        self == CloudState::OnlineOnly
    }
}

/// What a file's attributes say of it. None for a file no cloud marks:
/// an ordinary file, and also one in a synced folder that is simply here
/// (the attributes do not tell those two apart).
pub fn state_of(attributes: u32) -> Option<CloudState> {
    if attributes & (RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE) != 0 {
        Some(CloudState::OnlineOnly)
    } else if attributes & PINNED != 0 {
        Some(CloudState::AlwaysKept)
    } else if attributes & UNPINNED != 0 {
        Some(CloudState::Here)
    } else {
        None
    }
}

/// How the file at `path` stands, read from its attributes (which
/// fetches nothing). None also where there is no such file.
pub fn state_of_path(path: &str) -> Option<CloudState> {
    use std::os::windows::fs::MetadataExt;
    state_of(std::fs::symlink_metadata(path).ok()?.file_attributes())
}

/// What the shell is told of a file instead of looking for it: with
/// this, it takes a name for a file without going to the disk.
#[implement(IFileSystemBindData)]
struct MadeUp(WIN32_FIND_DATAW);

impl IFileSystemBindData_Impl for MadeUp_Impl {
    fn SetFindData(&self, _: *const WIN32_FIND_DATAW) -> ComResult<()> {
        Ok(())
    }

    fn GetFindData(&self, data: *mut WIN32_FIND_DATAW) -> ComResult<()> {
        // SAFETY: the shell hands room for one such structure.
        unsafe { *data = self.0 };
        Ok(())
    }
}

/// A name that is no file, with the ending of `path`: what stands in for
/// a file whose bytes must not be read.
fn stand_in(path: &str) -> String {
    let name = gevrek_zones::paths::file_name(path);
    match gevrek_zones::item::extension_of(name) {
        Some(extension) => format!("C:\\gevrek-no-such-folder\\file.{extension}"),
        None => "C:\\gevrek-no-such-folder\\file".to_string(),
    }
}

/// The picture of a file that is not on this machine, made without
/// reading it: the icon of its kind, or (`thumbnail`) the small picture
/// Windows already keeps of it, if it does.
pub fn picture(path: &str, size: u32, thumbnail: bool) -> Option<Picture> {
    let _com = Com::new();
    // SAFETY: the shell's objects are used on this thread alone, which
    // may use them while `_com` lives.
    unsafe {
        if thumbnail {
            let item: IShellItemImageFactory =
                SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
            return crate::icons::shell::picture_from(
                &item,
                size,
                SIIGBF_THUMBNAILONLY | SIIGBF_INCACHEONLY,
            );
        }
        let told: IFileSystemBindData = MadeUp(WIN32_FIND_DATAW {
            dwFileAttributes: NORMAL,
            ..Default::default()
        })
        .into();
        let context: IBindCtx = CreateBindCtx(0).ok()?;
        // (The name the shell looks its made-up file data up by.)
        let key = HSTRING::from("File System Bind Data");
        context
            .RegisterObjectParam(PCWSTR(key.as_ptr()), &told)
            .ok()?;
        let item: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(stand_in(path)), &context).ok()?;
        crate::icons::shell::picture_from(&item, size, SIIGBF_ICONONLY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attributes_say_how_a_file_stands_with_the_cloud() {
        const ARCHIVE: u32 = 0x20;
        const SPARSE: u32 = 0x200;
        const REPARSE: u32 = 0x400;
        for (attributes, state) in [
            (ARCHIVE, None),
            (ARCHIVE | REPARSE, None),
            // As OneDrive marks a file that is only in the cloud.
            (
                ARCHIVE | SPARSE | REPARSE | UNPINNED | RECALL_ON_DATA_ACCESS,
                Some(CloudState::OnlineOnly),
            ),
            (ARCHIVE | OFFLINE, Some(CloudState::OnlineOnly)),
            (RECALL_ON_OPEN, Some(CloudState::OnlineOnly)),
            (ARCHIVE | REPARSE | PINNED, Some(CloudState::AlwaysKept)),
            (ARCHIVE | REPARSE | UNPINNED, Some(CloudState::Here)),
            // Asked to stay, and not fetched yet: still not here.
            (PINNED | RECALL_ON_DATA_ACCESS, Some(CloudState::OnlineOnly)),
        ] {
            assert_eq!(state_of(attributes), state, "{attributes:#x}");
        }
        assert!(CloudState::OnlineOnly.fetched_when_read());
        assert!(!CloudState::AlwaysKept.fetched_when_read());
        assert!(!CloudState::Here.fetched_when_read());
    }

    #[test]
    fn a_stand_in_has_the_ending_and_is_no_file() {
        assert_eq!(
            stand_in("C:\\Users\\ada\\OneDrive\\Setup.EXE"),
            "C:\\gevrek-no-such-folder\\file.exe"
        );
        assert_eq!(
            stand_in("\\\\server\\share\\notes"),
            "C:\\gevrek-no-such-folder\\file"
        );
        assert!(!std::path::Path::new(&stand_in("a.txt")).exists());
    }

    /// The shell draws the icon of a kind for a name that is no file:
    /// nothing is read, so nothing can be fetched.
    #[test]
    fn the_icon_of_a_kind_is_drawn_without_a_file() {
        let folder =
            std::env::temp_dir().join(format!("gevrek-zones-cloud-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        // The file is there, and is not what is asked: the picture must
        // come also for a file that could not be read at all.
        let path = folder.join("not read.txt");
        let icon = picture(&path.to_string_lossy(), 48, false).expect("the icon of text files");
        assert_eq!((icon.width, icon.height), (48, 48));
        assert!(icon.pixels.iter().any(|byte| *byte != 0));
        // No small picture is kept of a file that never was.
        assert!(picture(&path.to_string_lossy(), 48, true).is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
