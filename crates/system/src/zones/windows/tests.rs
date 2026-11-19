//! The backend as a whole, against the real shell where that only reads,
//! and against folders of the tests' own (under the system's folder for
//! temporary files) where something is written. Explorer is one in
//! memory throughout: nothing of the real one's is ever changed here.

use super::*;
use explorer::{DesktopView, FakeExplorer, Icon, View};
use gevrek_zones::item::ItemKind;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A folder of this test's own under the system's folder for temporary
/// files, by its long name. Nothing outside it is touched.
fn folder(name: &str) -> String {
    let folder = std::env::temp_dir().join(format!("gevrek-zones-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("a folder");
    std::fs::canonicalize(&folder)
        .expect("the folder")
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string()
}

/// A backend that keeps its journal under `root`, talks to an Explorer in
/// memory, and says when it wakes its host.
fn backend(root: &str, explorer: FakeExplorer) -> (SystemBackend, mpsc::Receiver<()>) {
    let (woken, wakes) = mpsc::channel();
    let woken = Mutex::new(woken);
    let mut backend = SystemBackend::at(&Path::new(root).join("state"), Box::new(explorer), move || {
        let _ = woken.lock().expect("the channel").send(());
    });
    backend.set_quiet(true);
    (backend, wakes)
}

/// Drain until an event comes that `wanted` takes; what was told is kept
/// in `all`. The wait ends when the backend wakes its host.
fn until(
    backend: &mut SystemBackend,
    wakes: &mpsc::Receiver<()>,
    all: &mut Vec<BackendEvent>,
    wanted: impl Fn(&BackendEvent) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        all.extend(backend.drain_events());
        if all.iter().any(&wanted) {
            return;
        }
        let left = end.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "not told in time; told so far: {all:#?}");
        let _ = wakes.recv_timeout(left);
    }
}

#[test]
fn a_folder_is_listed_with_the_facts_of_what_is_in_it() {
    let root = folder("list");
    let shown = format!("{root}\\shown");
    std::fs::create_dir_all(format!("{shown}\\Folder")).expect("made");
    std::fs::write(format!("{shown}\\notes.txt"), b"twelve bytes").expect("written");
    std::fs::write(format!("{shown}\\Picture.PNG"), b"no picture").expect("written");
    std::fs::write(format!("{shown}\\Makefile"), b"").expect("written");
    std::fs::write(
        format!("{shown}\\site.url"),
        b"[InternetShortcut]\r\nURL=https://example.org/\r\n",
    )
    .expect("written");
    let (mut backend, _wakes) = backend(&root, FakeExplorer::default());
    // Written with the other separator, as a configuration may.
    let at = Location::Directory { path: shown.replace('\\', "/") };
    let items = backend.list(&at).expect("listed");
    let by_name = |name: &str| {
        items
            .iter()
            .find(|item| item.id.as_str() == format!("{shown}\\{name}"))
            .unwrap_or_else(|| panic!("{name} is not listed: {items:#?}"))
    };
    assert_eq!(items.len(), 5, "{items:#?}");

    let notes = by_name("notes.txt");
    assert_eq!(notes.kind, ItemKind::File);
    assert_eq!(notes.path.as_deref(), Some(notes.id.as_str()));
    assert_eq!(notes.facts.extension.as_deref(), Some("txt"));
    assert_eq!(notes.facts.mime.as_deref(), Some("text/plain"));
    assert_eq!(notes.facts.size, Some(12));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs();
    for time in [notes.facts.created, notes.facts.modified, notes.facts.accessed] {
        let time = time.expect("a time");
        assert!(time <= now + 5 && time + 600 > now, "{time} is not about {now}");
    }
    // The name is the shell's (with or without the ending, as the person
    // has Explorer show names); the ending is the file's.
    assert!(notes.name == "notes" || notes.name == "notes.txt", "{}", notes.name);

    let picture = by_name("Picture.PNG");
    assert_eq!(picture.facts.extension.as_deref(), Some("png"));
    assert_eq!(picture.facts.mime.as_deref(), Some("image/png"));
    let inner = by_name("Folder");
    assert_eq!(
        (inner.kind, inner.facts.mime.as_deref(), inner.facts.size, inner.facts.extension.as_deref()),
        (ItemKind::Folder, Some("inode/directory"), None, None)
    );
    let plain = by_name("Makefile");
    assert_eq!(
        (plain.kind, plain.facts.extension.as_deref(), plain.facts.size),
        (ItemKind::File, None, Some(0))
    );
    let link = by_name("site.url");
    assert_eq!(link.kind, ItemKind::Shortcut);
    assert_eq!(link.facts.shortcut_target.as_deref(), Some("https://example.org/"));

    // One item's facts, read now, are the listed ones; of what is not
    // there, none.
    assert_eq!(backend.facts(&notes.id), Some(notes.facts.clone()));
    assert_eq!(backend.facts(&ItemId::new(format!("{shown}\\gone.txt"))), None);
    assert!(matches!(
        backend.list(&Location::Directory { path: format!("{root}\\nowhere") }),
        Err(BackendError::NotFound { .. })
    ));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_watched_folder_tells_through_the_backend_and_wakes_its_host() {
    let root = folder("backend-watch");
    let shown = format!("{root}\\shown");
    std::fs::create_dir_all(&shown).expect("made");
    std::fs::write(format!("{shown}\\there.txt"), b"x").expect("written");
    let (mut backend, wakes) = backend(&root, FakeExplorer::default());
    let at = Location::Directory { path: shown.clone() };
    // Watching begins before the listing, as the host does it.
    let watch = backend.watch(&at).expect("watched");
    assert_eq!(backend.list(&at).expect("listed").len(), 1);
    let mut all = Vec::new();

    std::fs::write(format!("{shown}\\new.txt"), b"new").expect("written");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::ItemAdded { location, item }
            if location == &at && item.id.as_str() == format!("{shown}\\new.txt"))
    });
    // What was listed is known: written to, it changed; it did not come.
    std::fs::write(format!("{shown}\\there.txt"), b"longer now").expect("written");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::ItemChanged { item, .. }
            if item.id.as_str() == format!("{shown}\\there.txt") && item.facts.size == Some(10))
    });
    assert!(!all.iter().any(|event| {
        matches!(event, BackendEvent::ItemAdded { item, .. } if item.id.as_str().ends_with("there.txt"))
    }));

    // A file operation of the backend's own is told like any other
    // change, and its way back as well.
    let undo = backend
        .perform(&FileOperation::Rename {
            path: format!("{shown}\\new.txt"),
            new_name: "old.txt".to_string(),
        })
        .expect("renamed");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::ItemRenamed { from, item, .. }
            if from.as_str() == format!("{shown}\\new.txt") && item.id.as_str() == format!("{shown}\\old.txt"))
    });
    backend.perform(&undo).expect("renamed back");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::ItemRenamed { from, .. } if from.as_str() == format!("{shown}\\old.txt"))
    });

    backend.unwatch(watch);
    std::fs::remove_file(format!("{shown}\\new.txt")).expect("removed");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(backend.drain_events(), []);
    assert!(backend.watch(&Location::Directory { path: format!("{root}\\nowhere") }).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_icon_is_drawn_by_the_shell_off_this_thread_at_the_displays_size() {
    let root = folder("icon");
    let file = format!("{root}\\letter.txt");
    std::fs::write(&file, b"dear").expect("written");
    let (mut backend, wakes) = backend(&root, FakeExplorer::default());
    let item = ItemId::new(file);
    let icon = backend.request_icon(&item, IconSize { size: 32, scale: 1.5 });
    let thumbnail = backend.request_thumbnail(&item, IconSize { size: 32, scale: 1.0 });
    let gone = backend.request_icon(&ItemId::new(format!("{root}\\gone.txt")), IconSize { size: 32, scale: 1.0 });
    let mut all = Vec::new();
    let answered = |all: &[BackendEvent], wanted: IconRequest| {
        all.iter().find_map(|event| match event {
            BackendEvent::IconReady { request, icon } if *request == wanted => Some(*icon),
            _ => None,
        })
    };
    for wanted in [icon, thumbnail, gone] {
        until(&mut backend, &wakes, &mut all, |event| {
            matches!(event, BackendEvent::IconReady { request, .. } if *request == wanted)
        });
    }
    let handle = answered(&all, icon).flatten().expect("an icon for a text file");
    let picture = backend.picture(handle).expect("its pixels");
    assert_eq!((picture.width, picture.height), (48, 48));
    assert_eq!(picture.pixels.len(), 48 * 48 * 4);
    assert!(picture.pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0), "the icon is empty");
    // A text file holds no picture, and what is not there has no icon.
    assert_eq!(answered(&all, thumbnail), Some(None));
    assert_eq!(answered(&all, gone), Some(None));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_journal_of_explorers_icons_is_the_backends_and_survives_it() {
    let root = folder("backend-journal");
    let shown = DesktopView {
        view: View { icons_shown: true, snap_to_grid: true, view_mode: 1, icon_size: 48, ..View::default() },
        icons: vec![Icon {
            id: "C:\\Users\\ada\\Desktop\\a.txt".to_string(),
            name: "a.txt".to_string(),
            x: 20,
            y: 2,
        }],
    };
    let (mut first, _wakes) = backend(&root, FakeExplorer::showing(shown.clone()));
    assert!(!first.system_icons_to_restore());
    // Not written down, nothing is put away.
    assert!(first.hide_system_icons().is_err());
    first.journal_system_icons().expect("written down");
    assert!(first.system_icons_to_restore());
    first.hide_system_icons().expect("put away");
    let written =
        std::fs::read_to_string(Path::new(&root).join("state").join(explorer::FILE)).expect("the journal");
    let journal = explorer::Journal::decode(&written).expect("a journal");
    assert!(journal.open && journal.hidden);
    assert_eq!(journal.desktop, shown);
    drop(first);

    // The next run finds it open, and puts back what its Explorer (the
    // same desktop, icons still away) shows.
    let mut hidden = shown.clone();
    hidden.view.icons_shown = false;
    let (mut next, _wakes) = backend(&root, FakeExplorer::showing(hidden));
    assert!(next.system_icons_to_restore());
    next.restore_system_icons().expect("put back");
    assert!(!next.system_icons_to_restore());
    next.restore_system_icons().expect("nothing to do");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn what_is_dragged_out_is_handed_over_once_and_what_is_over_is_kept() {
    let root = folder("backend-drag");
    let file = format!("{root}\\carried.txt");
    std::fs::write(&file, b"x").expect("written");
    let _com = crate::com::Com::new();
    let (mut backend, _wakes) = backend(&root, FakeExplorer::default());
    let items = [ItemId::new(file.clone())];
    assert!(backend.take_drag().is_none());
    backend
        .start_drag(&items, DragActions { copy: true, moving: true, link: false })
        .expect("begun");
    let carried = backend.take_drag().expect("what is dragged");
    assert!(backend.take_drag().is_none());
    assert_eq!(dnd::effects_of(carried.allowed), 3);
    // Dropped on a zone of this program, it is the items again.
    let payload = dnd::payload_of(&carried.object, carried.allowed);
    assert_eq!((payload.items.as_slice(), payload.paths.as_slice()), (&items[..], &[file][..]));
    assert_eq!(backend.drop_payload(), None);
    backend.set_drop(Some(payload.clone()));
    assert_eq!(backend.drop_payload(), Some(payload));
    backend.set_drop(None);
    assert_eq!(backend.drop_payload(), None);
    assert!(backend
        .start_drag(&[ItemId::new(format!("{root}\\gone.txt"))], DragActions::ALL)
        .is_err());
    drop(carried);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn displays_the_window_manager_tells_are_told_on_and_replace_the_systems() {
    let root = folder("backend-displays");
    let (mut backend, wakes) = backend(&root, FakeExplorer::default());
    let mut left = Display::new("no display of this machine", gevrek_zones::Rect::new(0, 0, 1280, 720));
    left.primary = true;
    left.scale = 1.5;
    let kept = vec![ReservedArea {
        display: left.id.clone(),
        edge: gevrek_zones::geometry::Edge::Top,
        thickness: 40,
    }];
    backend.set_displays(vec![left.clone()], kept.clone());
    wakes.recv_timeout(Duration::from_secs(10)).expect("woken");
    assert_eq!(
        backend.drain_events(),
        [BackendEvent::Displays { displays: vec![left.clone()], reserved: kept.clone() }]
    );
    // The same again is no news.
    backend.set_displays(vec![left.clone()], kept.clone());
    assert_eq!(backend.drain_events(), []);
    assert_eq!(backend.displays.displays(), [left.clone()]);
    assert_eq!(backend.displays.reserved(), kept);
    // A display Windows does not know by that id is where its rectangle
    // says, at its scale.
    assert_eq!(backend.pixels_of(&left.id), Some((0, 0, 1920, 1080)));
    assert_eq!(backend.pixels_of(&DisplayId::new("nobody")), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// The desktop of whoever runs the tests is read, never changed.
#[test]
fn the_desktop_is_listed_by_the_shell_with_ids_it_takes_back() {
    let root = folder("backend-desktop");
    let (mut backend, _wakes) = backend(&root, FakeExplorer::default());
    // Watching it only asks to be told: the shell notes this program,
    // the file system the desktop's two folders. Twice is one watching.
    let watch = backend.watch(&Location::Desktop).expect("watched");
    let again = backend.watch(&Location::Desktop).expect("watched");
    assert_ne!(watch, again);
    let items = backend.list(&Location::Desktop).expect("the desktop");
    backend.unwatch(watch);
    backend.unwatch(again);
    assert!(backend.desktop.is_none());
    let mut ids: Vec<String> = items.iter().map(|item| item.id.as_str().to_lowercase()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), items.len(), "an id is there twice");
    let folders = shell::desktop_folders();
    for item in &items {
        assert!(!item.name.is_empty(), "{item:?}");
        assert!(shell::on_the_desktop(item.id.as_str(), &folders), "{} is not on the desktop", item.id);
        // What is a file has a path; what is none has none.
        assert_eq!(item.path.is_some(), item.kind != ItemKind::Virtual, "{item:?}");
        assert_eq!(facts::is_path(item.id.as_str()), item.kind != ItemKind::Virtual, "{item:?}");
        // The shell finds the same item by its id again.
        let again = shell::item_by_id(&item.id, &mut shell::Types::default())
            .unwrap_or_else(|| panic!("{} is not found by its id", item.id));
        assert_eq!((again.id.as_str(), again.kind), (item.id.as_str(), item.kind));
    }
    // Only what lies on the desktop itself is the desktop's.
    for (id, on) in [
        (format!("{}\\a.txt", folders[0]), true),
        (format!("{}\\A.TXT", folders[0].to_uppercase()), true),
        (format!("{}\\Projects\\deep.txt", folders[0]), false),
        ("C:\\gevrek-no-such-folder\\a.txt".to_string(), false),
        ("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\\::{X}".to_string(), false),
        (String::new(), false),
    ] {
        assert_eq!(shell::on_the_desktop(&id, &folders), on, "{id}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
