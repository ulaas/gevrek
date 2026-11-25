//! Explorer's own desktop icons: written down, put away, put back.
//!
//! Explorer draws the desktop's icons in a view of its own. When the
//! zones draw every item themselves, that view is put away, and this is
//! the one place where gevrek changes something that is Explorer's. So it
//! is done under a journal, and the rule is: **never lose a person's
//! layout**.
//!
//! - Before anything of Explorer's is changed, how it is now is written
//!   to `state/zones/explorer.json`: whether the icons are shown, how
//!   they are arranged and how large, and where each one is.
//! - The journal says it is *open* from that moment until everything was
//!   put back. A run that ends without putting back (a crash, the power)
//!   leaves it open, and the next start puts back first.
//! - An open journal is never written over: what Explorer shows while
//!   one is open is gevrek's doing, not the person's layout.
//! - That the icons are being put away is written into the journal
//!   *before* Explorer is asked to.
//!
//! Explorer is reached from outside, through the interfaces it gives
//! every program (`IShellWindows` finds the desktop's window,
//! `IFolderView2` is its view): no code runs in Explorer, no window of
//! its is touched. The icons are put away with the view's own flag for it
//! (`FWF_NOICONS`), which is what "Show desktop icons" in the desktop's
//! own menu switches; the person can always switch it back there, with
//! or without gevrek.
//!
//! All of it is asked through the small trait [`Explorer`], so that the
//! journal's logic runs in tests against an Explorer in memory
//! ([`FakeExplorer`]) and never touches the real one there.

use gevrek_zones::backend::BackendError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The journal's file under the state folder.
pub const FILE: &str = "zones/explorer.json";
/// The format of the journal.
const VERSION: u32 = 1;

/// How Explorer shows the desktop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// The icons are shown ("Show desktop icons" is ticked).
    pub icons_shown: bool,
    /// Explorer keeps the icons in rows by itself; they have no places
    /// of their own then.
    pub auto_arrange: bool,
    pub snap_to_grid: bool,
    /// Explorer's number for the kind of view (icons, a list), and the
    /// side of an icon in pixels.
    pub view_mode: i32,
    pub icon_size: i32,
}

/// One icon of Explorer's and where it is, in the pixels of the view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Icon {
    /// The item's parsing name: the id the zones know it by as well.
    pub id: String,
    /// The name as it is shown, for a person who reads the journal.
    pub name: String,
    pub x: i32,
    pub y: i32,
}

/// The desktop as Explorer shows it at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopView {
    pub view: View,
    pub icons: Vec<Icon>,
}

/// What is asked of Explorer.
pub trait Explorer {
    /// How the desktop is shown now, and where its icons are.
    fn read(&mut self) -> Result<DesktopView, BackendError>;
    /// Show the icons, or put them away.
    fn show_icons(&mut self, shown: bool) -> Result<(), BackendError>;
    fn arrange(&mut self, auto_arrange: bool, snap_to_grid: bool) -> Result<(), BackendError>;
    fn size_icons(&mut self, view_mode: i32, icon_size: i32) -> Result<(), BackendError>;
    /// Put these icons at these places.
    fn place(&mut self, icons: &[Icon]) -> Result<(), BackendError>;
}

/// What is written down.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Journal {
    pub version: u32,
    /// When it was written, in seconds since 1970.
    pub written_at: u64,
    /// Not everything was put back yet.
    pub open: bool,
    /// gevrek asked Explorer to put the icons away (or was about to).
    pub hidden: bool,
    /// How the desktop was before gevrek changed anything.
    pub desktop: DesktopView,
    /// The file was there and could not be read: nothing is known of how
    /// the desktop was. (Never written; said by `Keeper::load`.)
    #[serde(skip)]
    pub unreadable: bool,
}

impl Journal {
    pub fn encode(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Read a journal. One of a format that is not known, or that is no
    /// journal at all, is not guessed at.
    pub fn decode(text: &str) -> Option<Journal> {
        let journal: Journal = serde_json::from_str(text).ok()?;
        (journal.version == VERSION).then_some(journal)
    }

    /// What is assumed when a journal is there and cannot be read: that
    /// something was changed, and that the icons were shown before. The
    /// one thing gevrek ever does to Explorer is put its icons away, and
    /// showing icons a person had put away themselves is the smaller harm
    /// than leaving a desktop empty.
    fn unreadable() -> Journal {
        Journal {
            version: VERSION,
            open: true,
            hidden: true,
            unreadable: true,
            desktop: DesktopView {
                view: View {
                    icons_shown: true,
                    ..View::default()
                },
                icons: Vec::new(),
            },
            ..Journal::default()
        }
    }
}

/// One thing to do to Explorer to put the desktop back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Size {
        view_mode: i32,
        icon_size: i32,
    },
    Arrange {
        auto_arrange: bool,
        snap_to_grid: bool,
    },
    Place(Vec<Icon>),
    ShowIcons,
}

/// What has to be done to make the desktop as the journal has it, given
/// how it is now. Only what differs is touched, in the order in which one
/// step does not undo another: the size first (Explorer lays the icons
/// out anew when it changes), then how they are arranged, then each icon
/// to its place, and the icons are shown last, when all is where it
/// belongs.
///
/// Icons are never put away by this: if the person switched them on
/// while gevrek ran, that is what they want.
pub fn plan(journal: &Journal, now: &DesktopView) -> Vec<Step> {
    let (was, is) = (&journal.desktop.view, &now.view);
    let mut steps = Vec::new();
    if journal.unreadable {
        // Nothing is known but that the icons may be away.
        if !is.icons_shown {
            steps.push(Step::ShowIcons);
        }
        return steps;
    }
    if was.icon_size > 0 && (was.view_mode, was.icon_size) != (is.view_mode, is.icon_size) {
        steps.push(Step::Size {
            view_mode: was.view_mode,
            icon_size: was.icon_size,
        });
    }
    if (was.auto_arrange, was.snap_to_grid) != (is.auto_arrange, is.snap_to_grid) {
        steps.push(Step::Arrange {
            auto_arrange: was.auto_arrange,
            snap_to_grid: was.snap_to_grid,
        });
    }
    // With auto-arrange Explorer decides the places itself and takes none.
    if !was.auto_arrange {
        let resized = steps
            .iter()
            .any(|step| matches!(step, Step::Size { .. } | Step::Arrange { .. }));
        let moved: Vec<Icon> = journal
            .desktop
            .icons
            .iter()
            .filter(|icon| {
                now.icons
                    .iter()
                    .find(|other| other.id.eq_ignore_ascii_case(&icon.id))
                    // (Something that is not on the desktop any more has
                    // no place to go back to. After a change of size or
                    // arrangement every place is given again: Explorer
                    // moved the icons when it was made.)
                    .is_some_and(|other| resized || (other.x, other.y) != (icon.x, icon.y))
            })
            .cloned()
            .collect();
        if !moved.is_empty() {
            steps.push(Step::Place(moved));
        }
    }
    if was.icons_shown && !is.icons_shown {
        steps.push(Step::ShowIcons);
    }
    steps
}

fn refused(what: impl std::fmt::Display) -> BackendError {
    BackendError::Refused {
        what: what.to_string(),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// The journal on the disk.
#[derive(Debug, Clone)]
pub struct Keeper {
    path: PathBuf,
}

impl Keeper {
    /// The journal under a state folder (`gevrek_model::paths::state_dir`).
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join(FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The journal as it is on the disk; none when nothing was ever
    /// written down.
    pub fn load(&self) -> Option<Journal> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                log::warn!("zones: {} cannot be read ({error})", self.path.display());
                return Some(Journal::unreadable());
            }
        };
        Some(Journal::decode(&text).unwrap_or_else(|| {
            log::warn!(
                "zones: {} is no journal that is understood",
                self.path.display()
            );
            Journal::unreadable()
        }))
    }

    fn save(&self, journal: &Journal) -> Result<(), BackendError> {
        gevrek_model::files::write_atomic(&self.path, journal.encode().as_bytes())
            .map_err(|error| refused(format!("{}: {error}", self.path.display())))
    }

    /// Whether something is written down that was not put back.
    pub fn to_restore(&self) -> bool {
        self.load().is_some_and(|journal| journal.open)
    }

    /// Write down how the desktop is now. Nothing of Explorer's is
    /// changed. While a journal is open it stays as it is: what Explorer
    /// shows then is not the person's layout any more.
    pub fn journal(&self, explorer: &mut dyn Explorer) -> Result<(), BackendError> {
        let before = self.load();
        if before.as_ref().is_some_and(|journal| journal.open) {
            return Ok(());
        }
        let desktop = explorer.read()?;
        // The one from the time before is kept beside the new one: a
        // layout that was written down is not thrown away by the next.
        if before.is_some() {
            if let Err(error) = std::fs::copy(&self.path, self.path.with_extension("previous.json"))
            {
                log::debug!(
                    "zones: the journal before is not kept beside {}: {error}",
                    self.path.display()
                );
            }
        }
        log::debug!(
            "zones: how Explorer's desktop is, is written down in {}",
            self.path.display()
        );
        self.save(&Journal {
            version: VERSION,
            written_at: now(),
            open: true,
            hidden: false,
            desktop,
            unreadable: false,
        })
    }

    /// Put Explorer's icons away. Refused unless the desktop is written
    /// down (`journal`), and written into the journal before it is done.
    pub fn hide(&self, explorer: &mut dyn Explorer) -> Result<(), BackendError> {
        let Some(mut journal) = self.load().filter(|journal| journal.open) else {
            return Err(refused(
                "Explorer's icons are not written down; they are not put away",
            ));
        };
        if !journal.desktop.view.icons_shown {
            // The person has them switched off already: nothing to do,
            // and nothing to put back later.
            log::debug!("zones: Explorer's icons are switched off by the person already; nothing is put away");
            return Ok(());
        }
        log::debug!("zones: Explorer's icons are put away");
        journal.hidden = true;
        self.save(&journal)?;
        explorer.show_icons(false)
    }

    /// Put everything back as it is written down, and close the journal.
    /// If a step fails the journal stays open, and it is tried again
    /// when asked the next time (at the latest at the next start).
    pub fn restore(&self, explorer: &mut dyn Explorer) -> Result<(), BackendError> {
        let Some(mut journal) = self.load().filter(|journal| journal.open) else {
            return Ok(());
        };
        let now_shown = explorer.read()?;
        let steps = plan(&journal, &now_shown);
        log::debug!(
            "zones: Explorer's desktop is put back as written down, in {} steps",
            steps.len()
        );
        for step in steps {
            match step {
                Step::Size {
                    view_mode,
                    icon_size,
                } => explorer.size_icons(view_mode, icon_size)?,
                Step::Arrange {
                    auto_arrange,
                    snap_to_grid,
                } => explorer.arrange(auto_arrange, snap_to_grid)?,
                Step::Place(icons) => explorer.place(&icons)?,
                Step::ShowIcons => explorer.show_icons(true)?,
            }
        }
        // Kept as a record of the layout, closed.
        journal.unreadable = false;
        journal.open = false;
        journal.hidden = false;
        self.save(&journal)
    }
}

/// An Explorer in memory: what it shows is a value, and what was asked
/// of it is written down. For the tests of everything above, and for
/// whoever tests against the backend.
#[derive(Debug, Default)]
pub struct FakeExplorer {
    pub desktop: DesktopView,
    /// What was asked of it, in order.
    pub asked: Vec<String>,
    /// The next thing asked that changes something fails.
    pub fail_next: bool,
    /// The journal's file, to look at in the moment something is changed:
    /// what was on the disk each time, as text.
    pub journal_file: Option<PathBuf>,
    pub journal_at_each_change: Vec<String>,
}

impl FakeExplorer {
    pub fn showing(desktop: DesktopView) -> Self {
        Self {
            desktop,
            ..Self::default()
        }
    }

    fn change(&mut self, what: String) -> Result<(), BackendError> {
        if let Some(file) = &self.journal_file {
            self.journal_at_each_change
                .push(std::fs::read_to_string(file).unwrap_or_default());
        }
        if std::mem::take(&mut self.fail_next) {
            return Err(refused("Explorer is not there"));
        }
        self.asked.push(what);
        Ok(())
    }
}

impl Explorer for FakeExplorer {
    fn read(&mut self) -> Result<DesktopView, BackendError> {
        Ok(self.desktop.clone())
    }

    fn show_icons(&mut self, shown: bool) -> Result<(), BackendError> {
        self.change(format!("show_icons({shown})"))?;
        self.desktop.view.icons_shown = shown;
        Ok(())
    }

    fn arrange(&mut self, auto_arrange: bool, snap_to_grid: bool) -> Result<(), BackendError> {
        self.change(format!("arrange({auto_arrange}, {snap_to_grid})"))?;
        self.desktop.view.auto_arrange = auto_arrange;
        self.desktop.view.snap_to_grid = snap_to_grid;
        Ok(())
    }

    fn size_icons(&mut self, view_mode: i32, icon_size: i32) -> Result<(), BackendError> {
        self.change(format!("size_icons({view_mode}, {icon_size})"))?;
        self.desktop.view.view_mode = view_mode;
        self.desktop.view.icon_size = icon_size;
        Ok(())
    }

    fn place(&mut self, icons: &[Icon]) -> Result<(), BackendError> {
        self.change(format!("place({})", icons.len()))?;
        for icon in icons {
            if let Some(known) = self
                .desktop
                .icons
                .iter_mut()
                .find(|known| known.id.eq_ignore_ascii_case(&icon.id))
            {
                (known.x, known.y) = (icon.x, icon.y);
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use real::ShellDesktop;

#[cfg(windows)]
mod real {
    use super::{refused, DesktopView, Explorer, Icon, View};
    use crate::com::Com;
    use crate::zones::windows::shell::{taken, Pidl};
    use gevrek_zones::backend::BackendError;
    use windows::core::{Interface, VARIANT};
    use windows::Win32::Foundation::POINT;
    use windows::Win32::System::Com::{CoCreateInstance, IServiceProvider, CLSCTX_ALL};
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{
        IEnumIDList, IFolderView2, IShellBrowser, IShellWindows, SHGetNameFromIDList,
        SID_STopLevelBrowser, ShellWindows, FOLDERVIEWMODE, FWF_AUTOARRANGE, FWF_NOICONS,
        FWF_SNAPTOGRID, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY, SVGIO_ALLVIEW,
        SVSI_POSITIONITEM, SWC_DESKTOP, SWFO_NEEDDISPATCH,
    };

    /// Explorer's desktop, reached from outside.
    #[derive(Debug)]
    pub struct ShellDesktop {
        /// Only reading is allowed: everything that would change
        /// something of Explorer's is refused. For looking (an example, a
        /// test on a desktop that is somebody's).
        read_only: bool,
    }

    impl ShellDesktop {
        /// Explorer's desktop, to read and to change.
        pub fn new() -> Self {
            Self { read_only: false }
        }

        /// Explorer's desktop, to read only.
        pub fn read_only() -> Self {
            Self { read_only: true }
        }

        fn changing(&self) -> Result<(), BackendError> {
            if self.read_only {
                Err(refused("Explorer's desktop is only read here"))
            } else {
                Ok(())
            }
        }
    }

    impl Default for ShellDesktop {
        fn default() -> Self {
            Self::new()
        }
    }

    /// The view Explorer shows the desktop in. Explorer registers the
    /// desktop among its windows; from the window the way goes to its
    /// browser, from there to the view that is shown in it. All of it are
    /// objects in Explorer's process that this one only talks to.
    unsafe fn view() -> Result<IFolderView2, BackendError> {
        let found = (|| {
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            let mut window = 0i32;
            // (The desktop is asked for by its number among the special
            // folders, 0, and no root.)
            let desktop = windows.FindWindowSW(
                &VARIANT::from(0i32),
                &VARIANT::default(),
                SWC_DESKTOP,
                &mut window,
                SWFO_NEEDDISPATCH,
            )?;
            let services: IServiceProvider = desktop.cast()?;
            let browser: IShellBrowser = services.QueryService(&SID_STopLevelBrowser)?;
            browser.QueryActiveShellView()?.cast::<IFolderView2>()
        })();
        found.map_err(|error: windows::core::Error| {
            refused(format!(
                "Explorer's desktop was not reached: {}",
                error.message()
            ))
        })
    }

    /// Every item of the view, as Explorer's own list for it.
    unsafe fn items(view: &IFolderView2) -> Result<Vec<Pidl>, BackendError> {
        let all: IEnumIDList = view
            .Items(SVGIO_ALLVIEW)
            .map_err(|error| refused(error.message()))?;
        let mut found = Vec::new();
        loop {
            let mut next = [std::ptr::null_mut(); 1];
            let mut fetched = 0u32;
            if all.Next(&mut next, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            found.extend(Pidl::from_raw(next[0]));
        }
        Ok(found)
    }

    /// The parsing name of an item of the desktop's view. (The desktop
    /// is where the shell's names begin: an item's list within it is its
    /// whole name.)
    unsafe fn name(item: &Pidl, shown: bool) -> String {
        let which = if shown {
            SIGDN_NORMALDISPLAY
        } else {
            SIGDN_DESKTOPABSOLUTEPARSING
        };
        SHGetNameFromIDList(item.as_ptr(), which)
            .map(|name| taken(name))
            .unwrap_or_default()
    }

    /// How often, and how far apart, the desktop is read before its
    /// refusal is believed. Just after its icons were shown again (an
    /// earlier run's put back at the start), Explorer's view answers
    /// "Element not found" for a moment, while it lists them anew; the
    /// zones were then not shown for the whole run.
    const READ_TRIES: u32 = 10;
    const READ_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

    impl ShellDesktop {
        fn read_once(&mut self) -> Result<DesktopView, BackendError> {
            let _com = Com::new();
            // SAFETY: Explorer's objects are used on this thread alone.
            unsafe {
                let view = view()?;
                let flags = view
                    .GetCurrentFolderFlags()
                    .map_err(|error| refused(error.message()))?;
                let has = |flag: i32| flags & flag as u32 != 0;
                let (mut view_mode, mut icon_size) = (FOLDERVIEWMODE::default(), 0i32);
                let _ = view.GetViewModeAndIconSize(&mut view_mode, &mut icon_size);
                let mut icons = Vec::new();
                for item in items(&view)? {
                    let id = name(&item, false);
                    if id.is_empty() {
                        continue;
                    }
                    let Ok(POINT { x, y }) = view.GetItemPosition(item.as_ptr()) else {
                        continue;
                    };
                    icons.push(Icon {
                        id,
                        name: name(&item, true),
                        x,
                        y,
                    });
                }
                Ok(DesktopView {
                    view: View {
                        icons_shown: !has(FWF_NOICONS.0),
                        auto_arrange: has(FWF_AUTOARRANGE.0),
                        snap_to_grid: has(FWF_SNAPTOGRID.0),
                        view_mode: view_mode.0,
                        icon_size,
                    },
                    icons,
                })
            }
        }
    }

    impl Explorer for ShellDesktop {
        fn read(&mut self) -> Result<DesktopView, BackendError> {
            let mut tried = 1;
            loop {
                match self.read_once() {
                    Ok(desktop) => {
                        if tried > 1 {
                            log::debug!("zones: Explorer's desktop was read at the {tried}. try");
                        }
                        return Ok(desktop);
                    }
                    Err(error) if tried >= READ_TRIES => return Err(error),
                    Err(error) => {
                        log::debug!(
                            "zones: Explorer's desktop was not read ({error}); it is read again"
                        );
                        tried += 1;
                        std::thread::sleep(READ_PAUSE);
                    }
                }
            }
        }

        fn show_icons(&mut self, shown: bool) -> Result<(), BackendError> {
            self.changing()?;
            let _com = Com::new();
            let flag = FWF_NOICONS.0 as u32;
            // SAFETY: as in `read`.
            unsafe {
                view()?
                    .SetCurrentFolderFlags(flag, if shown { 0 } else { flag })
                    .map_err(|error| refused(error.message()))
            }
        }

        fn arrange(&mut self, auto_arrange: bool, snap_to_grid: bool) -> Result<(), BackendError> {
            self.changing()?;
            let _com = Com::new();
            let (auto, snap) = (FWF_AUTOARRANGE.0 as u32, FWF_SNAPTOGRID.0 as u32);
            let wanted = if auto_arrange { auto } else { 0 } | if snap_to_grid { snap } else { 0 };
            // SAFETY: as in `read`.
            unsafe {
                view()?
                    .SetCurrentFolderFlags(auto | snap, wanted)
                    .map_err(|error| refused(error.message()))
            }
        }

        fn size_icons(&mut self, view_mode: i32, icon_size: i32) -> Result<(), BackendError> {
            self.changing()?;
            let _com = Com::new();
            // SAFETY: as in `read`.
            unsafe {
                view()?
                    .SetViewModeAndIconSize(FOLDERVIEWMODE(view_mode), icon_size)
                    .map_err(|error| refused(error.message()))
            }
        }

        fn place(&mut self, icons: &[Icon]) -> Result<(), BackendError> {
            self.changing()?;
            let _com = Com::new();
            // SAFETY: as in `read`.
            unsafe {
                let view = view()?;
                // An icon is moved by the view's own list for its item,
                // found by the item's name.
                for item in items(&view)? {
                    let id = name(&item, false);
                    let Some(icon) = icons.iter().find(|icon| icon.id.eq_ignore_ascii_case(&id))
                    else {
                        continue;
                    };
                    let (list, at): (*const ITEMIDLIST, POINT) = (
                        item.as_ptr(),
                        POINT {
                            x: icon.x,
                            y: icon.y,
                        },
                    );
                    view.SelectAndPositionItems(1, &list, Some(&at), SVSI_POSITIONITEM.0 as u32)
                        .map_err(|error| refused(error.message()))?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(name: &str, x: i32, y: i32) -> Icon {
        Icon {
            id: format!("C:\\Users\\ada\\Desktop\\{name}"),
            name: name.to_string(),
            x,
            y,
        }
    }

    fn desktop() -> DesktopView {
        DesktopView {
            view: View {
                icons_shown: true,
                auto_arrange: false,
                snap_to_grid: true,
                view_mode: 1,
                icon_size: 48,
            },
            icons: vec![
                icon("a.txt", 20, 2),
                icon("b.txt", 20, 102),
                icon("c.txt", 116, 2),
            ],
        }
    }

    fn keeper(name: &str) -> (Keeper, PathBuf) {
        let folder = std::env::temp_dir().join(format!(
            "gevrek-zones-journal-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&folder);
        (Keeper::new(&folder), folder)
    }

    #[test]
    fn a_journal_survives_its_text_and_an_unknown_one_is_not_guessed_at() {
        let journal = Journal {
            version: VERSION,
            written_at: 1_700_000_000,
            open: true,
            hidden: true,
            desktop: desktop(),
            unreadable: false,
        };
        let text = journal.encode();
        assert_eq!(Journal::decode(&text), Some(journal.clone()));
        // The file is one a person can read.
        assert!(
            text.contains("\"icons_shown\": true") && text.contains("\"name\": \"a.txt\""),
            "{text}"
        );
        // A format from later, and what is no journal.
        assert_eq!(
            Journal::decode(&text.replace("\"version\": 1", "\"version\": 2")),
            None
        );
        assert_eq!(Journal::decode("{ half"), None);
        assert_eq!(Journal::decode("{}"), None);
        // Keys that are not known are passed over; missing ones have
        // their defaults.
        let lean =
            r#"{"version":1,"open":true,"later":{"x":1},"desktop":{"view":{"icons_shown":true}}}"#;
        let read = Journal::decode(lean).expect("read");
        assert!(read.open && read.desktop.view.icons_shown && read.desktop.icons.is_empty());
    }

    #[test]
    fn putting_back_touches_only_what_differs() {
        let journal = Journal {
            version: VERSION,
            open: true,
            desktop: desktop(),
            ..Journal::default()
        };
        // As it was: nothing to do.
        assert_eq!(plan(&journal, &desktop()), []);
        // Put away, and nothing else: only shown again.
        let mut now = desktop();
        now.view.icons_shown = false;
        assert_eq!(plan(&journal, &now), [Step::ShowIcons]);
        // One icon elsewhere, one gone from the desktop, one new.
        now.icons[1].y = 500;
        now.icons.remove(2);
        now.icons.push(icon("new.txt", 300, 300));
        assert_eq!(
            plan(&journal, &now),
            [Step::Place(vec![icon("b.txt", 20, 102)]), Step::ShowIcons]
        );
        // Another size and arrangement: both first, then every icon that
        // is still there to its place.
        now.view.icon_size = 96;
        now.view.auto_arrange = true;
        assert_eq!(
            plan(&journal, &now),
            [
                Step::Size {
                    view_mode: 1,
                    icon_size: 48
                },
                Step::Arrange {
                    auto_arrange: false,
                    snap_to_grid: true
                },
                Step::Place(vec![icon("a.txt", 20, 2), icon("b.txt", 20, 102)]),
                Step::ShowIcons,
            ]
        );
    }

    #[test]
    fn icons_the_person_arranges_by_explorer_or_switched_off_are_left_so() {
        // Auto-arrange: Explorer has the places, none are given.
        let mut arranged = desktop();
        arranged.view.auto_arrange = true;
        let journal = Journal {
            version: VERSION,
            open: true,
            desktop: arranged.clone(),
            ..Journal::default()
        };
        let mut now = arranged.clone();
        now.icons[0].x = 999;
        now.view.icons_shown = false;
        assert_eq!(plan(&journal, &now), [Step::ShowIcons]);
        // Switched off by the person before gevrek came: stays off. And
        // switched on by them while gevrek ran: stays on.
        let mut off = desktop();
        off.view.icons_shown = false;
        let journal = Journal {
            version: VERSION,
            open: true,
            desktop: off.clone(),
            ..Journal::default()
        };
        assert_eq!(plan(&journal, &off), []);
        assert_eq!(plan(&journal, &desktop()), []);
    }

    #[test]
    fn the_desktop_is_written_down_before_it_is_changed_and_put_back_after() {
        let (keeper, folder) = keeper("order");
        let mut explorer = FakeExplorer::showing(desktop());
        explorer.journal_file = Some(keeper.path().to_path_buf());
        // Nothing written down: nothing is put away, nothing to put back.
        assert!(!keeper.to_restore());
        assert!(keeper.hide(&mut explorer).is_err());
        assert_eq!(explorer.asked, Vec::<String>::new());
        keeper.restore(&mut explorer).expect("nothing to do");

        keeper.journal(&mut explorer).expect("written down");
        assert!(keeper.to_restore());
        assert_eq!(
            explorer.asked,
            Vec::<String>::new(),
            "writing down changes nothing"
        );
        let written = keeper.load().expect("a journal");
        assert_eq!(
            (written.open, written.hidden, &written.desktop),
            (true, false, &desktop())
        );

        keeper.hide(&mut explorer).expect("put away");
        assert_eq!(explorer.asked, ["show_icons(false)"]);
        // In the moment Explorer was asked, the disk already said so.
        let at_the_change =
            Journal::decode(&explorer.journal_at_each_change[0]).expect("a journal on the disk");
        assert!(at_the_change.open && at_the_change.hidden);
        assert_eq!(at_the_change.desktop, desktop());

        // Written down again while open: the layout from before stays.
        explorer.desktop.icons[0].x = 777;
        keeper.journal(&mut explorer).expect("kept");
        assert_eq!(keeper.load().expect("a journal").desktop, desktop());

        keeper.restore(&mut explorer).expect("put back");
        assert_eq!(
            explorer.asked,
            ["show_icons(false)", "place(1)", "show_icons(true)"]
        );
        assert_eq!(explorer.desktop, desktop());
        assert!(!keeper.to_restore());
        // Closed, it stays as a record; put back again, nothing is asked.
        assert_eq!(
            keeper.load().map(|journal| (journal.open, journal.hidden)),
            Some((false, false))
        );
        keeper.restore(&mut explorer).expect("nothing to do");
        assert_eq!(explorer.asked.len(), 3);

        // The next writing down keeps the one before beside it.
        explorer.desktop.icons[0].x = 40;
        keeper.journal(&mut explorer).expect("written down");
        assert_eq!(keeper.load().expect("a journal").desktop.icons[0].x, 40);
        let previous =
            std::fs::read_to_string(keeper.path().with_extension("previous.json")).expect("kept");
        assert_eq!(
            Journal::decode(&previous).expect("a journal").desktop,
            desktop()
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_run_that_ended_without_putting_back_is_put_back_by_the_next() {
        let (keeper, folder) = keeper("crash");
        let mut explorer = FakeExplorer::showing(desktop());
        keeper.journal(&mut explorer).expect("written down");
        keeper.hide(&mut explorer).expect("put away");
        // The run ends here. Explorer still has the icons put away, and
        // meanwhile lost a place.
        explorer.desktop.icons[2] = icon("c.txt", 0, 0);
        let next_run = Keeper::new(&folder);
        assert!(next_run.to_restore());
        // Explorer is not there yet: the journal stays open.
        explorer.fail_next = true;
        assert!(next_run.restore(&mut explorer).is_err());
        assert!(next_run.to_restore());
        next_run.restore(&mut explorer).expect("put back");
        assert_eq!(explorer.desktop, desktop());
        assert!(!next_run.to_restore());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_journal_that_cannot_be_read_still_brings_the_icons_back() {
        let (keeper, folder) = keeper("corrupt");
        std::fs::create_dir_all(keeper.path().parent().expect("a folder")).expect("made");
        std::fs::write(keeper.path(), b"{ \"version\": 1, \"open\": tr").expect("written");
        assert!(keeper.to_restore());
        let mut hidden = desktop();
        hidden.view.icons_shown = false;
        let mut explorer = FakeExplorer::showing(hidden);
        keeper.restore(&mut explorer).expect("put back");
        assert_eq!(explorer.asked, ["show_icons(true)"]);
        assert!(!keeper.to_restore());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn icons_the_person_switched_off_are_not_touched_at_all() {
        let (keeper, folder) = keeper("off");
        let mut off = desktop();
        off.view.icons_shown = false;
        let mut explorer = FakeExplorer::showing(off.clone());
        keeper.journal(&mut explorer).expect("written down");
        keeper.hide(&mut explorer).expect("nothing to put away");
        keeper.restore(&mut explorer).expect("nothing to put back");
        assert_eq!(explorer.asked, Vec::<String>::new());
        assert_eq!(explorer.desktop, off);
        let _ = std::fs::remove_dir_all(folder);
    }

    /// Reads the real Explorer's desktop and changes nothing: the one
    /// used here refuses everything but reading.
    #[cfg(windows)]
    #[test]
    fn explorers_desktop_is_read_from_outside_and_never_changed_here() {
        let mut explorer = ShellDesktop::read_only();
        for refused in [
            explorer.show_icons(false),
            explorer.show_icons(true),
            explorer.arrange(true, true),
            explorer.size_icons(1, 48),
            explorer.place(&[icon("a.txt", 0, 0)]),
        ] {
            assert!(matches!(refused, Err(BackendError::Refused { .. })));
        }
        // (A machine that runs tests with no Explorer has no desktop to
        // read; that is an answer too, and not a failure of this.)
        match explorer.read() {
            Ok(shown) => {
                for icon in &shown.icons {
                    assert!(!icon.id.is_empty());
                }
                // Read twice, it is the same: reading changed nothing.
                assert_eq!(explorer.read().expect("read again"), shown);
            }
            Err(error) => assert!(matches!(error, BackendError::Refused { .. })),
        }
    }

    /// Puts the real Explorer's icons away and brings them back. Changes
    /// the desktop of whoever runs it: run by hand only, on a desktop
    /// that is yours, with `cargo test -p gevrek-system -- --ignored
    /// really_hides`.
    #[cfg(windows)]
    #[test]
    #[ignore = "hides the real desktop icons for three seconds; run by hand"]
    fn really_hides_explorers_icons_and_brings_them_back() {
        let (keeper, folder) = keeper("real");
        let mut explorer = ShellDesktop::new();
        let before = explorer.read().expect("Explorer's desktop");
        keeper.journal(&mut explorer).expect("written down");
        keeper.hide(&mut explorer).expect("put away");
        assert!(!explorer.read().expect("read").view.icons_shown || !before.view.icons_shown);
        std::thread::sleep(std::time::Duration::from_secs(3));
        keeper.restore(&mut explorer).expect("put back");
        assert_eq!(explorer.read().expect("read"), before);
        let _ = std::fs::remove_dir_all(folder);
    }
}
