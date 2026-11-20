//! What is done to files: copying, moving, renaming, a new folder, into
//! the Recycle Bin and out of it.
//!
//! All of it is the shell's own file operation (`IFileOperation`), the
//! one Explorer uses: it puts a file into the Recycle Bin instead of
//! deleting it, gives a file another name where one of its name is
//! already (never writing over it), and says afterwards what it made. So
//! what takes an operation back is known exactly, and nothing more is
//! needed for the Recycle Bin than the shell itself.
//!
//! Nothing here ever deletes a file outright. Where the shell could only
//! do that (a drive without a Recycle Bin, a file too large for it), it
//! asks the person itself before it does; that question is the one thing
//! of the shell's that is not silenced.

use super::facts::same_path;
use super::shell::{plain, shell_item, taken};
use crate::com::Com;
use gevrek_zones::backend::{BackendError, FileOperation};
use gevrek_zones::paths;
use std::cell::RefCell;
use std::collections::HashMap;
use crate::zones::operations::Watched;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use windows::core::{implement, Interface, Result as ComResult, GUID, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::{FILETIME, HANDLE, HWND};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_RecycleBinFolder, FileOperation as ShellFileOperation,
    IEnumShellItems, IFileOperation, IFileOperationProgressSink,
    IFileOperationProgressSink_Impl, IShellItem, IShellItem2, SHGetKnownFolderItem,
    FILEOPERATION_FLAGS, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE,
    FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI,
    FOF_RENAMEONCOLLISION, FOF_SILENT, FOF_WANTNUKEWARNING, KF_FLAG_DEFAULT,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};

/// Where something in the Recycle Bin was deleted from (the folder), and
/// when: the shell's properties of those names.
const DELETED_FROM: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871),
    pid: 2,
};
const DELETED_AT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871),
    pid: 3,
};

/// What the shell said of the one operation it was given.
#[derive(Debug, Default)]
struct Outcome {
    /// The parsing name of what it made (the copy, the moved or renamed
    /// file, the new folder, the file as it lies in the Recycle Bin).
    made: Option<String>,
    failed: Option<HRESULT>,
}

/// The system's number for "the person (or the program) called it off",
/// as a result: what the shell is answered to make it stop.
const CALLED_OFF: HRESULT = HRESULT(0x8007_04C7_u32 as i32);

/// Listens to the shell while it works, to hear what became of the item;
/// and, for an operation that ends later, to say how far it is and to
/// stop it when it was called off.
#[implement(IFileOperationProgressSink)]
struct Listener(Rc<RefCell<Outcome>>, Option<Arc<Watched>>);

impl Listener_Impl {
    /// Whether the shell is to go on: not when the operation was called
    /// off. (The shell asks before each item and while it copies; an
    /// error for an answer ends the operation, and what it had begun to
    /// write is taken away by the shell itself.)
    fn go_on(&self) -> ComResult<()> {
        match &self.1 {
            Some(watched) if watched.called_off() => Err(CALLED_OFF.into()),
            _ => Ok(()),
        }
    }

    fn heard(&self, result: HRESULT, made: Option<&IShellItem>) -> ComResult<()> {
        let mut outcome = self.0.borrow_mut();
        if result.is_err() {
            outcome.failed = Some(result);
        } else if let Some(made) = made {
            // SAFETY: the item is the shell's and lives for this call.
            outcome.made = unsafe { made.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }
                .ok()
                .map(|name| unsafe { taken(name) })
                .filter(|name| !name.is_empty());
        }
        Ok(())
    }
}

impl IFileOperationProgressSink_Impl for Listener_Impl {
    fn StartOperations(&self) -> ComResult<()> {
        Ok(())
    }
    fn FinishOperations(&self, _: HRESULT) -> ComResult<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Option<&IShellItem>, _: &PCWSTR) -> ComResult<()> {
        self.go_on()
    }
    fn PostRenameItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        made: Option<&IShellItem>,
    ) -> ComResult<()> {
        self.heard(result, made)
    }
    fn PreMoveItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        _: Option<&IShellItem>,
        _: &PCWSTR,
    ) -> ComResult<()> {
        self.go_on()
    }
    fn PostMoveItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        _: Option<&IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        made: Option<&IShellItem>,
    ) -> ComResult<()> {
        self.heard(result, made)
    }
    fn PreCopyItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        _: Option<&IShellItem>,
        _: &PCWSTR,
    ) -> ComResult<()> {
        self.go_on()
    }
    fn PostCopyItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        _: Option<&IShellItem>,
        _: &PCWSTR,
        result: HRESULT,
        made: Option<&IShellItem>,
    ) -> ComResult<()> {
        self.heard(result, made)
    }
    fn PreDeleteItem(&self, _: u32, _: Option<&IShellItem>) -> ComResult<()> {
        Ok(())
    }
    fn PostDeleteItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        result: HRESULT,
        made: Option<&IShellItem>,
    ) -> ComResult<()> {
        self.heard(result, made)
    }
    fn PreNewItem(&self, _: u32, _: Option<&IShellItem>, _: &PCWSTR) -> ComResult<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _: u32,
        _: Option<&IShellItem>,
        _: &PCWSTR,
        _: &PCWSTR,
        _: u32,
        result: HRESULT,
        made: Option<&IShellItem>,
    ) -> ComResult<()> {
        self.heard(result, made)
    }
    fn UpdateProgress(&self, total: u32, so_far: u32) -> ComResult<()> {
        // (In the shell's own count of work, which is no count of bytes.)
        if let Some(watched) = &self.1 {
            watched.progress(u64::from(so_far), u64::from(total));
        }
        self.go_on()
    }
    fn ResetTimer(&self) -> ComResult<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> ComResult<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> ComResult<()> {
        Ok(())
    }
}

/// Why the shell did not do it, in the core's words.
pub fn error_of(result: HRESULT, what: &str) -> BackendError {
    // The system's own numbers, as results: "file not found", "path not
    // found"; "file exists", "already exists".
    match result.0 as u32 {
        0x8007_0002 | 0x8007_0003 => BackendError::NotFound { what: what.to_string() },
        0x8007_0050 | 0x8007_00B7 => BackendError::Exists { what: what.to_string() },
        0x8007_04C7 => BackendError::Cancelled,
        _ => BackendError::Refused { what: format!("{what}: {}", result.message()) },
    }
}

fn missing(what: &str) -> BackendError {
    BackendError::NotFound { what: what.to_string() }
}

/// The file operations, and what they need to remember. A copy of this
/// remembers the same: the one that carries operations out on a thread
/// of its own and the one that does it at once put back each other's
/// files.
#[derive(Clone, Default)]
pub struct FileOperations {
    /// Nothing of the shell's is shown while it works, not even how far
    /// a long copy is. For tests.
    pub quiet: bool,
    /// The window what the shell shows belongs to; 0 for none.
    pub owner: isize,
    /// What was put into the Recycle Bin in this run, by the path it had
    /// (in small letters): where it lies there. One of several files
    /// deleted from the same place is then put back exactly.
    recycled: Arc<Mutex<HashMap<String, String>>>,
    /// For an operation that ends later: where to say how far it is,
    /// and where it is said that it was called off.
    pub watched: Option<Arc<Watched>>,
}

impl FileOperations {
    /// Carry out one operation. The answer is what takes it back, as the
    /// shell did it (a file may have got another name).
    pub fn perform(&mut self, operation: &FileOperation) -> Result<FileOperation, BackendError> {
        let _com = Com::new();
        match operation {
            FileOperation::Copy { from, to_folder } => {
                let (from, to_folder) = (existing(from)?, existing_folder(to_folder)?);
                let made = self.run(&from, collide(), |shell, listener| unsafe {
                    shell.CopyItem(&item(&from)?, &item(&to_folder)?, PCWSTR::null(), listener)
                })?;
                Ok(FileOperation::Trash { path: made })
            }
            FileOperation::Move { from, to_folder } => {
                let (from, to_folder) = (existing(from)?, existing_folder(to_folder)?);
                let back = paths::parent(&from).ok_or(BackendError::Unsupported)?.to_string();
                if same_path(&back, &to_folder) {
                    // It is there already; the shell would call that a
                    // collision and give it another name.
                    return Ok(FileOperation::Move { from, to_folder });
                }
                let made = self.run(&from, collide(), |shell, listener| unsafe {
                    shell.MoveItem(&item(&from)?, &item(&to_folder)?, PCWSTR::null(), listener)
                })?;
                Ok(FileOperation::Move { from: made, to_folder: back })
            }
            FileOperation::Rename { path, new_name } => {
                let path = existing(path)?;
                let folder = paths::parent(&path).ok_or(BackendError::Unsupported)?;
                if new_name.is_empty() || new_name.contains(['\\', '/']) {
                    return Err(BackendError::Refused { what: format!("{new_name:?} is no name") });
                }
                let old_name = paths::file_name(&path).to_string();
                let target = paths::join(folder, new_name);
                // (Only the case of its letters changed: the same file.)
                if !same_path(&target, &path) && std::path::Path::new(&target).exists() {
                    return Err(BackendError::Exists { what: new_name.clone() });
                }
                let name = HSTRING::from(new_name.as_str());
                let made = self.run(&path, collide(), |shell, listener| unsafe {
                    shell.RenameItem(&item(&path)?, &name, listener)
                })?;
                Ok(FileOperation::Rename { path: made, new_name: old_name })
            }
            FileOperation::NewFolder { parent, name } => {
                let parent = existing_folder(parent)?;
                let wide = HSTRING::from(name.as_str());
                let made = self.run(&parent, collide(), |shell, listener| unsafe {
                    shell.NewItem(
                        &item(&parent)?,
                        FILE_ATTRIBUTE_DIRECTORY.0,
                        &wide,
                        PCWSTR::null(),
                        listener,
                    )
                })?;
                Ok(FileOperation::Trash { path: made })
            }
            FileOperation::Trash { path } => {
                let path = existing(path)?;
                // Into the Recycle Bin; and where that cannot be, the
                // shell asks before it deletes.
                let how = FOFX_RECYCLEONDELETE | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING;
                let lies_at = self.run(&path, how, |shell, listener| unsafe {
                    shell.DeleteItem(&item(&path)?, listener)
                });
                match lies_at {
                    Ok(lies_at) => {
                        self.recycled().insert(path.to_lowercase(), lies_at);
                    }
                    // (Done, and the shell named nothing it made: the
                    // file is gone for good, with the person's yes.)
                    Err(BackendError::Unsupported) => {}
                    Err(error) => return Err(error),
                }
                Ok(FileOperation::Restore { path })
            }
            FileOperation::Restore { path } => {
                let path = plain(path);
                let folder = paths::parent(&path).ok_or(BackendError::Unsupported)?.to_string();
                let remembered = self.recycled().remove(&path.to_lowercase());
                let lies_at = match remembered {
                    Some(lies_at) if std::path::Path::new(&lies_at).exists() => lies_at,
                    _ => in_recycle_bin(&path).ok_or_else(|| missing(&path))?,
                };
                // (As the shell does when it puts something back whose
                // folder went meanwhile.)
                std::fs::create_dir_all(&folder).map_err(|error| BackendError::Refused {
                    what: format!("{folder}: {error}"),
                })?;
                let name = HSTRING::from(paths::file_name(&path));
                let made = self.run(&path, collide(), |shell, listener| unsafe {
                    shell.MoveItem(&item(&lies_at)?, &item(&folder)?, &name, listener)
                })?;
                Ok(FileOperation::Trash { path: made })
            }
        }
    }

    fn recycled(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.recycled.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Give the shell one operation and have it carried out. The answer
    /// is the parsing name of what the shell made; `Unsupported` stands
    /// for "done, and it named nothing".
    fn run(
        &self,
        what: &str,
        how: FILEOPERATION_FLAGS,
        queue: impl FnOnce(&IFileOperation, &IFileOperationProgressSink) -> ComResult<()>,
    ) -> Result<String, BackendError> {
        let outcome = Rc::new(RefCell::new(Outcome::default()));
        let listener: IFileOperationProgressSink = Listener(Rc::clone(&outcome), self.watched.clone()).into();
        // No questions and no messages of the shell's: what goes wrong is
        // answered to who asked. It ends at the first thing that fails.
        let mut flags = how | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_NOCONFIRMMKDIR | FOFX_EARLYFAILURE;
        if self.quiet {
            flags |= FOF_SILENT;
        }
        // SAFETY: the shell's objects are used on this thread alone,
        // which the caller has set up for them.
        let done = unsafe {
            (|| {
                let shell: IFileOperation = CoCreateInstance(&ShellFileOperation, None, CLSCTX_ALL)?;
                shell.SetOperationFlags(flags)?;
                if self.owner != 0 {
                    shell.SetOwnerWindow(HWND(self.owner as *mut _))?;
                }
                queue(&shell, &listener)?;
                shell.PerformOperations()?;
                Ok::<bool, windows::core::Error>(shell.GetAnyOperationsAborted()?.as_bool())
            })()
        };
        let outcome = outcome.take();
        if let Some(failed) = outcome.failed {
            return Err(error_of(failed, what));
        }
        match done {
            Err(error) => Err(error_of(error.code(), what)),
            Ok(true) => Err(BackendError::Refused { what: format!("{what}: not carried out") }),
            Ok(false) => outcome.made.ok_or(BackendError::Unsupported),
        }
    }
}

/// Another name is given where one of that name is already: nothing is
/// ever written over.
fn collide() -> FILEOPERATION_FLAGS {
    FOF_RENAMEONCOLLISION
}

/// The shell's item of a path.
unsafe fn item(path: &str) -> ComResult<IShellItem> {
    shell_item(path).ok_or_else(|| windows::core::Error::from(HRESULT(0x8007_0002u32 as i32)))
}

/// A path as Windows writes it, of something that is there.
fn existing(path: &str) -> Result<String, BackendError> {
    let path = plain(path);
    if std::path::Path::new(&path).exists() {
        Ok(path)
    } else {
        Err(missing(&path))
    }
}

fn existing_folder(path: &str) -> Result<String, BackendError> {
    let path = plain(path);
    if std::path::Path::new(&path).is_dir() {
        Ok(path)
    } else {
        Err(missing(&path))
    }
}

/// One thing in the Recycle Bin.
struct Recycled {
    /// The folder it was deleted from, and the name it had there.
    from: String,
    name: String,
    /// Where it lies now (`C:\$Recycle.Bin\<user>\$R....txt`).
    lies_at: String,
    /// When it was deleted, to tell later from earlier.
    at: (Option<std::time::SystemTime>, u64),
}

/// Everything in the Recycle Bin. The bin is the shell's folder of that
/// name; each thing in it says where it was deleted from and when.
fn recycle_bin() -> Vec<Recycled> {
    let mut found = Vec::new();
    // SAFETY: the shell's objects are used on this thread alone.
    unsafe {
        let Ok(bin) = SHGetKnownFolderItem::<_, IShellItem>(
            &FOLDERID_RecycleBinFolder,
            KF_FLAG_DEFAULT,
            HANDLE::default(),
        ) else {
            return found;
        };
        let Ok(all) = bin.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else {
            return found;
        };
        loop {
            let mut next = [None];
            let mut fetched = 0u32;
            if all.Next(&mut next, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(lying) = next[0].take().and_then(|lying| lying.cast::<IShellItem2>().ok()) else {
                continue;
            };
            let Ok(from) = lying.GetString(&DELETED_FROM).map(|from| taken(from)) else {
                continue;
            };
            let Ok(lies_at) = lying
                .GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)
                .map(|lies_at| taken(lies_at))
            else {
                continue;
            };
            // The shell's own time of the deleting is whole seconds. The
            // note the bin keeps beside each thing (`$I...` beside
            // `$R...`) was written at that moment and says it finer.
            let note = note_of(&lies_at);
            let noted = note
                .as_deref()
                .and_then(|note| std::fs::metadata(note).ok())
                .and_then(|note| note.modified().ok());
            // The name it had is read from that note, which holds the
            // whole path. What the bin shows it by is that path too, but
            // without the name's ending where Explorer is set to hide
            // the endings of known types (as it is on a new Windows):
            // `kept.txt` was then looked for as `kept` and not found, and
            // nothing that was deleted by an earlier run could be put
            // back. Only where the note cannot be read is what the bin
            // shows taken.
            let had = note
                .as_deref()
                .and_then(|note| std::fs::read(note).ok())
                .and_then(|note| path_in_note(&note))
                .unwrap_or_else(|| {
                    lying
                        .GetDisplayName(SIGDN_NORMALDISPLAY)
                        .map(|shown| taken(shown))
                        .unwrap_or_default()
                });
            found.push(Recycled {
                from,
                name: paths::file_name(&had).to_string(),
                at: (noted, lying.GetFileTime(&DELETED_AT).map(ticks).unwrap_or(0)),
                lies_at,
            });
        }
    }
    found
}

/// The note the bin keeps beside a thing that lies in it.
fn note_of(lies_at: &str) -> Option<std::path::PathBuf> {
    let lying = std::path::Path::new(lies_at);
    let rest = lying.file_name()?.to_str()?.strip_prefix("$R")?;
    Some(lying.with_file_name(format!("$I{rest}")))
}

/// The path a thing had before it was deleted, as the bin's note of it
/// says (the `$I...` file). The note begins with three numbers of eight
/// bytes (its kind, the thing's size, when it was deleted); then, in the
/// kind Windows 10 and later write (2), the length of the path and the
/// path, and in the older kind (1) the path alone, in room for 260
/// letters. Nothing for a note that is not so.
fn path_in_note(note: &[u8]) -> Option<String> {
    let kind = u64::from_le_bytes(note.get(..8)?.try_into().ok()?);
    let letters = match kind {
        1 => note.get(24..)?,
        2 => {
            let length = u32::from_le_bytes(note.get(24..28)?.try_into().ok()?) as usize;
            note.get(28..)?.get(..length.checked_mul(2)?)?
        }
        _ => return None,
    };
    let units: Vec<u16> = letters
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    let path = String::from_utf16(&units).ok()?;
    (!path.is_empty()).then_some(path)
}

/// Where in the Recycle Bin the file lies that was last deleted from
/// `path`.
fn in_recycle_bin(path: &str) -> Option<String> {
    let folder = paths::parent(path)?;
    let name = paths::file_name(path);
    recycle_bin()
        .into_iter()
        .filter(|lying| same_path(&lying.from, folder) && lying.name.to_lowercase() == name.to_lowercase())
        .max_by_key(|lying| lying.at)
        .map(|lying| lying.lies_at)
}

/// A time of Windows as one number, to tell later from earlier.
fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systems_numbers_become_the_cores_errors() {
        let error = |number: u32| error_of(HRESULT(number as i32), "a.txt");
        assert_eq!(error(0x8007_0002), BackendError::NotFound { what: "a.txt".to_string() });
        assert_eq!(error(0x8007_0003), BackendError::NotFound { what: "a.txt".to_string() });
        assert_eq!(error(0x8007_0050), BackendError::Exists { what: "a.txt".to_string() });
        assert_eq!(error(0x8007_00B7), BackendError::Exists { what: "a.txt".to_string() });
        assert!(matches!(error(0x8007_0005), BackendError::Refused { what } if what.starts_with("a.txt: ")));
    }

    /// A folder of this test's own under the system's folder for
    /// temporary files. Nothing outside it is touched.
    fn folder(name: &str) -> String {
        let folder = std::env::temp_dir().join(format!("gevrek-zones-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        // (As the shell writes it: the long form, not `RUNNER~1`.)
        std::fs::canonicalize(&folder)
            .expect("the folder")
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .to_string()
    }

    /// A test's folder, tidied when the test ends, however it ends:
    /// whatever the test put into the Recycle Bin from under its folder
    /// is put back by the shell (so nothing of a test stays in the
    /// person's bin), and then the folder goes.
    struct Tidy(String);

    impl Drop for Tidy {
        fn drop(&mut self) {
            let _com = Com::new();
            let under = format!("{}\\", self.0.to_lowercase());
            let mut shell = FileOperations { quiet: true, ..Default::default() };
            for lying in recycle_bin() {
                if format!("{}\\", lying.from.to_lowercase()).starts_with(&under) {
                    let _ = shell.perform(&FileOperation::Restore {
                        path: paths::join(&lying.from, &lying.name),
                    });
                }
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(folder: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .expect("listed")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_file_operation_is_carried_out_by_the_shell_and_taken_back() {
        let root = folder("fileops");
        let _tidy = Tidy(root.clone());
        let (desk, archive) = (paths::join(&root, "desk"), paths::join(&root, "archive"));
        std::fs::create_dir_all(&desk).expect("made");
        std::fs::create_dir_all(&archive).expect("made");
        let file = paths::join(&desk, "a.txt");
        std::fs::write(&file, b"what the file holds").expect("written");
        let mut shell = FileOperations { quiet: true, ..Default::default() };
        let before = (names(&desk), names(&archive));
        for (operation, after) in [
            (
                FileOperation::Copy { from: file.clone(), to_folder: archive.clone() },
                (vec!["a.txt"], vec!["a.txt"]),
            ),
            (
                FileOperation::Move { from: file.clone(), to_folder: archive.clone() },
                (vec![], vec!["a.txt"]),
            ),
            (
                FileOperation::Rename { path: file.clone(), new_name: "b.md".to_string() },
                (vec!["b.md"], vec![]),
            ),
            (
                FileOperation::NewFolder { parent: desk.clone(), name: "New".to_string() },
                (vec!["New", "a.txt"], vec![]),
            ),
            (FileOperation::Trash { path: file.clone() }, (vec![], vec![])),
        ] {
            let undo = shell.perform(&operation).unwrap_or_else(|error| panic!("{operation:?}: {error}"));
            let now = (names(&desk), names(&archive));
            assert_eq!(
                (now.0.iter().map(String::as_str).collect::<Vec<_>>(), now.1.iter().map(String::as_str).collect::<Vec<_>>()),
                after,
                "{operation:?}"
            );
            // The way back is what the core would have assumed, here
            // where nothing was in the way.
            assert_eq!(Some(&undo), operation.reversed().as_ref(), "{operation:?}");
            shell.perform(&undo).unwrap_or_else(|error| panic!("{undo:?}: {error}"));
            assert_eq!((names(&desk), names(&archive)), before, "{undo:?} did not take {operation:?} back");
            // What lies in the bin is a file there, beside its note.
            assert!(
                shell.recycled().values().all(|lies_at| note_of(lies_at).is_some_and(|note| note.exists())),
                "{:?}",
                shell.recycled
            );
            assert_eq!(std::fs::read(&file).expect("read"), b"what the file holds");
        }

    }

    #[test]
    fn a_file_is_never_written_over_and_the_way_back_knows_its_new_name() {
        let root = folder("collide");
        let _tidy = Tidy(root.clone());
        let (desk, archive) = (paths::join(&root, "desk"), paths::join(&root, "archive"));
        std::fs::create_dir_all(&desk).expect("made");
        std::fs::create_dir_all(&archive).expect("made");
        let (file, other) = (paths::join(&desk, "a.txt"), paths::join(&archive, "a.txt"));
        std::fs::write(&file, b"mine").expect("written");
        std::fs::write(&other, b"the other").expect("written");
        let mut shell = FileOperations { quiet: true, ..Default::default() };

        let operation = FileOperation::Move { from: file.clone(), to_folder: archive.clone() };
        let undo = shell.perform(&operation).expect("moved");
        assert_eq!(std::fs::read(&other).expect("read"), b"the other");
        let FileOperation::Move { from: moved, to_folder } = &undo else {
            panic!("a move is taken back by a move: {undo:?}");
        };
        assert!(same_path(to_folder, &desk));
        assert!(!same_path(moved, &other), "it got a name of its own: {moved}");
        assert_eq!(std::fs::read(moved).expect("read"), b"mine");
        assert_ne!(Some(&undo), operation.reversed().as_ref());
        assert_eq!(names(&archive).len(), 2);

        // A name that is taken is refused, not given to both.
        let taken = paths::file_name(moved).to_string();
        assert_eq!(
            shell.perform(&FileOperation::Rename { path: other.clone(), new_name: taken.clone() }),
            Err(BackendError::Exists { what: taken })
        );
        // Only the letters' case: that is the same file, and allowed.
        let renamed = shell
            .perform(&FileOperation::Rename { path: other.clone(), new_name: "A.TXT".to_string() })
            .expect("renamed");
        assert_eq!(names(&archive).iter().filter(|name| name.as_str() == "A.TXT").count(), 1);
        shell.perform(&renamed).expect("renamed back");

        // What is not there, and a name that is none.
        let gone = paths::join(&desk, "gone.txt");
        for operation in [
            FileOperation::Trash { path: gone.clone() },
            FileOperation::Restore { path: gone.clone() },
            FileOperation::Copy { from: gone.clone(), to_folder: archive.clone() },
            FileOperation::Move { from: other.clone(), to_folder: paths::join(&root, "nowhere") },
        ] {
            assert!(
                matches!(shell.perform(&operation), Err(BackendError::NotFound { .. })),
                "{operation:?}"
            );
        }
        assert!(matches!(
            shell.perform(&FileOperation::Rename { path: other.clone(), new_name: "a\\b".to_string() }),
            Err(BackendError::Refused { .. })
        ));
    }

    /// The bin's note of a deleted thing holds the whole path it had,
    /// the name's ending with it, in both kinds Windows writes.
    #[test]
    fn the_bins_note_says_the_path_a_thing_had() {
        let path = "C:\\Users\\ada\\Desktop\\kept for later.txt";
        let letters = |with_end: bool| -> Vec<u8> {
            path.encode_utf16()
                .chain(with_end.then_some(0))
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        let head = |kind: u64| -> Vec<u8> {
            [kind, 5, 133_000_000_000_000_000].iter().flat_map(|number| number.to_le_bytes()).collect()
        };
        // The kind of Windows 10 and later: the length, then the path.
        let mut new = head(2);
        new.extend((path.encode_utf16().count() as u32 + 1).to_le_bytes());
        new.extend(letters(true));
        assert_eq!(path_in_note(&new).as_deref(), Some(path));
        // The older kind: the path in room for 260 letters.
        let mut old = head(1);
        old.extend(letters(true));
        old.resize(24 + 520, 0);
        assert_eq!(path_in_note(&old).as_deref(), Some(path));
        // What is no such note.
        assert_eq!(path_in_note(&head(3)), None);
        assert_eq!(path_in_note(&new[..30]), None);
        assert_eq!(path_in_note(b"short"), None);
        assert_eq!(path_in_note(&head(2)), None);
    }

    #[test]
    fn what_was_deleted_is_found_in_the_recycle_bin_by_another_run() {
        let root = folder("recycle");
        let _tidy = Tidy(root.clone());
        let file = paths::join(&root, "kept for later.txt");
        std::fs::write(&file, b"first").expect("written");
        let mut shell = FileOperations { quiet: true, ..Default::default() };
        shell.perform(&FileOperation::Trash { path: file.clone() }).expect("deleted");
        // Another of the same name, deleted later: that is the one a
        // run that remembers nothing puts back.
        std::fs::write(&file, b"second").expect("written");
        let undo = shell.perform(&FileOperation::Trash { path: file.clone() }).expect("deleted");
        assert!(names(&root).is_empty());
        // The bin's own note of it says the path it had, whole: that is
        // what it is found by, however Explorer is set to show names.
        let lying = in_recycle_bin(&file).expect("in the bin");
        let note = std::fs::read(note_of(&lying).expect("a note beside it")).expect("the note is read");
        assert!(same_path(&path_in_note(&note).expect("a path in the note"), &file));
        let mut another_run = FileOperations { quiet: true, ..Default::default() };
        let again = another_run.perform(&undo).expect("put back");
        assert_eq!(again, FileOperation::Trash { path: file.clone() });
        assert_eq!(std::fs::read(&file).expect("read"), b"second");
        // The earlier one is still there, and comes back beside it under
        // a name of its own.
        let beside = another_run.perform(&undo).expect("put back");
        let FileOperation::Trash { path: beside } = beside else {
            panic!("what was put back is taken back by deleting it");
        };
        assert!(!same_path(&beside, &file));
        assert_eq!(std::fs::read(&beside).expect("read"), b"first");
        assert!(matches!(another_run.perform(&undo), Err(BackendError::NotFound { .. })));
    }
}
