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
    for time in [notes.facts.created, notes.facts.modified] {
        let time = time.expect("a time");
        assert!(time <= now + 5 && time + 600 > now, "{time} is not about {now}");
    }
    // Nobody opened it: the file system's own time of last access is
    // not taken for that (see `recent`).
    assert_eq!(notes.facts.accessed, None);
    assert_eq!(notes.facts.last_touched(), notes.facts.created.max(notes.facts.modified));
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
    // Watching begins before the listing, as the host does it. Neither
    // goes to the folder on this thread: what is in it is told.
    let watch = backend.watch(&at).expect("watched");
    assert!(backend.list(&at).expect("listed").len() <= 1);
    let mut all = Vec::new();
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::Listed { location, items } if location == &at && items.len() == 1)
    });
    assert_eq!(backend.list(&at).expect("known").len(), 1);
    assert_eq!(backend.unreachable(), []);

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

    // A folder that is not there is watched all the same: it is told as
    // away, with nothing in it, and shown when it comes.
    let nowhere = Location::Directory { path: format!("{root}\\nowhere") };
    let waiting = backend.watch(&nowhere).expect("watched");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::ListingFailed { location, .. } if location == &nowhere)
    });
    assert_eq!(backend.list(&nowhere).expect("known"), []);
    assert_eq!(backend.unreachable().len(), 1);
    std::fs::create_dir(format!("{root}\\nowhere")).expect("made");
    std::fs::write(format!("{root}\\nowhere\\here.txt"), b"x").expect("written");
    until(&mut backend, &wakes, &mut all, |event| match event {
        BackendEvent::Listed { location, items } => location == &nowhere && items.len() == 1,
        BackendEvent::ItemAdded { location, .. } => location == &nowhere,
        _ => false,
    });
    assert_eq!(backend.unreachable(), []);
    backend.unwatch(waiting);
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

/// A backend whose Downloads, Recent and state folders are the test's
/// own (under `root`), and which knows no installed programs: nothing of
/// the person's is read.
fn backend_under(root: &str) -> (SystemBackend, mpsc::Receiver<()>) {
    let places = places::Places::under(Path::new(root));
    for folder in [&places.downloads, &places.recent] {
        std::fs::create_dir_all(folder.as_ref().expect("a folder")).expect("made");
    }
    let (woken, wakes) = mpsc::channel();
    let woken = Mutex::new(woken);
    let mut backend = SystemBackend::with(places, Box::new(FakeExplorer::default()), move || {
        let _ = woken.lock().expect("the channel").send(());
    });
    backend.set_quiet(true);
    backend.set_programs(Vec::new);
    (backend, wakes)
}

#[test]
fn recent_files_and_downloads_are_told_whole_from_folders_of_the_tests_own() {
    let root = folder("smart");
    let shown = format!("{root}\\shown");
    std::fs::create_dir_all(&shown).expect("made");
    std::fs::write(format!("{shown}\\report.txt"), b"report").expect("written");
    let (mut backend, wakes) = backend_under(&root);
    let mut all = Vec::new();
    let names = |items: &[Item]| items.iter().map(|item| item.name.clone()).collect::<Vec<_>>();

    // What is no list of the backend's is said to be none.
    assert_eq!(backend.watch_source(&SmartSource::Desktop), Err(BackendError::Unsupported));
    assert_eq!(backend.list_source(&SmartSource::Folder { directory: shown.clone() }), Err(BackendError::Unsupported));

    // A portal shows the folder; nobody has opened the file in it.
    let at = Location::Directory { path: shown.clone() };
    backend.watch(&at).expect("watched");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::Listed { location, items }
            if location == &at && items.len() == 1 && items[0].facts.accessed.is_none())
    });

    // Recent files: none yet, and told so.
    let recent = backend.watch_source(&SmartSource::Recent).expect("watched");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::SourceListed { source: SmartSource::Recent, items } if items.is_empty())
    });
    // The person opens the file: the shell writes a shortcut to it into
    // the Recent folder. The list has it, and the item in the portal
    // knows when it was opened.
    let link = Path::new(&root).join("Recent").join("report.lnk");
    assert!(recent::write_link(&link, &format!("{shown}\\report.txt")));
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::SourceListed { source: SmartSource::Recent, items }
            if items.len() == 1 && items[0].id.as_str() == format!("{shown}\\report.txt") && items[0].facts.accessed.is_some())
    });
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::ItemChanged { location, item }
            if location == &at && item.facts.accessed.is_some() && item.facts.last_touched() == item.facts.accessed)
    });
    assert_eq!(names(&backend.list_source(&SmartSource::Recent).expect("known")).len(), 1);
    let facts = backend.facts(&ItemId::new(format!("{shown}\\report.txt"))).expect("facts");
    assert!(facts.accessed.is_some());

    // Downloads: the newest first, told whole whenever a file arrives.
    let downloads = backend.watch_source(&SmartSource::Downloads).expect("watched");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::SourceListed { source: SmartSource::Downloads, items } if items.is_empty())
    });
    std::fs::write(format!("{root}\\Downloads\\setup.exe"), b"MZ").expect("written");
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::SourceListed { source: SmartSource::Downloads, items }
            if items.len() == 1 && items[0].facts.extension.as_deref() == Some("exe") && items[0].facts.created.is_some())
    });

    // Let go of, nothing more is told of either.
    backend.unwatch(recent);
    backend.unwatch(downloads);
    let _ = backend.drain_events();
    std::fs::write(format!("{root}\\Downloads\\late.txt"), b"late").expect("written");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!backend.drain_events().iter().any(|event| matches!(event, BackendEvent::SourceListed { .. })));
    drop(backend);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn programs_are_items_that_open_and_have_no_menu_of_the_shells() {
    let root = folder("programs");
    let (mut backend, wakes) = backend_under(&root);
    let mut all = Vec::new();
    backend.watch_source(&SmartSource::RunningApps).expect("watched");
    backend.set_wm_state(&serde_json::json!({"monitors": [{"workspaces": [{"detailed_windows": [
        {"window_id": 21, "title": "notes", "class": "Notepad", "exe": "C:\\Windows\\notepad.exe", "is_focused": true}]}]}]}));
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::SourceListed { source: SmartSource::RunningApps, items }
            if items.len() == 1 && items[0].kind == ItemKind::Launcher && items[0].name == "notepad")
    });
    let running = ItemId::new("running:c:\\windows\\notepad.exe");
    assert_eq!(
        backend.facts(&running).and_then(|facts| facts.shortcut_target).as_deref(),
        Some("C:\\Windows\\notepad.exe")
    );
    // Opening it is a command for the window manager, by the line the
    // shell gives; without one it cannot be done.
    assert_eq!(backend.open(&running), Err(BackendError::Unsupported));
    let (sent, commands) = mpsc::channel();
    backend.set_commander(move |command| {
        let _ = sent.send(command);
    });
    backend.open(&running).expect("sent");
    assert_eq!(commands.try_recv().expect("a command"), crate::zones::programs::focus_command(21));
    assert_eq!(backend.context_menu(std::slice::from_ref(&running), Point { x: 0, y: 0 }), Err(BackendError::Unsupported));
    assert_eq!(backend.open_with(&running, None), Err(BackendError::Unsupported));
    assert!(backend.start_drag(std::slice::from_ref(&running), DragActions::ALL).is_err());
    // Its picture is its program's file's, drawn by the shell.
    let asked = backend.request_icon(&running, IconSize { size: 32, scale: 1.0 });
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::IconReady { request, icon: Some(_) } if *request == asked)
    });

    // Pins are written down under the state folder of the test.
    backend.pin_launcher("editor", true).expect("pinned");
    assert_eq!(backend.pinned_launchers(), ["editor"]);
    assert!(Path::new(&root).join("state").join("zones").join("pinned-launchers.json").is_file());
    backend.set_pinned_launchers(&[]).expect("written");
    assert_eq!(backend.pinned_launchers(), Vec::<String>::new());
    assert_eq!(backend.installed_programs(), []);
    drop(backend);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_a_cloud_keeps_is_known_by_its_attributes_and_others_are_not() {
    let root = folder("cloud");
    std::fs::write(format!("{root}\\here.txt"), b"here").expect("written");
    let (backend, _wakes) = backend(&root, FakeExplorer::default());
    assert_eq!(backend.cloud_state(&ItemId::new(format!("{root}\\here.txt"))), None);
    assert_eq!(backend.cloud_state(&ItemId::new(format!("{root}\\gone.txt"))), None);
    assert_eq!(backend.cloud_state(&ItemId::new("::{645FF040-5081-101B-9F08-00AA002F954E}")), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// What rules by "where a shortcut leads" and by type are asked of, on
/// real shortcuts: to a folder, to a program, to an address, and one
/// whose target is gone.
#[test]
fn shortcuts_say_where_they_lead_also_when_that_is_gone() {
    let root = folder("shortcuts");
    let (shown, targets) = (format!("{root}\\shown"), format!("{root}\\targets"));
    std::fs::create_dir_all(&shown).expect("made");
    std::fs::create_dir_all(format!("{targets}\\Folder")).expect("made");
    std::fs::write(format!("{targets}\\gone.txt"), b"soon gone").expect("written");
    // (A program that is on every Windows: only its path is written
    // into the shortcut, nothing of it is read or started.)
    let program = std::env::var("SystemRoot").map(|windows| format!("{windows}\\System32\\notepad.exe")).expect("Windows");
    let link = |name: &str, target: &str| {
        assert!(recent::write_link(Path::new(&format!("{shown}\\{name}.lnk")), target), "{name}");
    };
    link("to folder", &format!("{targets}\\Folder"));
    link("to program", &program);
    link("broken", &format!("{targets}\\gone.txt"));
    std::fs::remove_file(format!("{targets}\\gone.txt")).expect("removed");
    std::fs::write(format!("{shown}\\site.url"), b"[InternetShortcut]\r\nURL=https://example.org/page\r\n").expect("written");

    let (mut backend, _wakes) = backend(&root, FakeExplorer::default());
    let items = backend.list(&Location::Directory { path: shown.clone() }).expect("listed");
    let target_of = |file: &str| {
        let item = items
            .iter()
            .find(|item| item.id.as_str() == format!("{shown}\\{file}"))
            .unwrap_or_else(|| panic!("{file} is not listed: {items:#?}"));
        assert_eq!(item.kind, ItemKind::Shortcut, "{file}");
        item.facts.shortcut_target.clone().unwrap_or_else(|| panic!("{file} leads nowhere: {item:#?}"))
    };
    assert!(facts::same_path(&target_of("to folder.lnk"), &format!("{targets}\\Folder")));
    assert!(facts::same_path(&target_of("to program.lnk"), &program));
    assert!(facts::same_path(&target_of("broken.lnk"), &format!("{targets}\\gone.txt")));
    assert_eq!(target_of("site.url"), "https://example.org/page");
    // The ending and the type are the shortcut's own, not its target's.
    let by_name = |file: &str| items.iter().find(|item| item.id.as_str() == format!("{shown}\\{file}")).expect("listed");
    assert_eq!(by_name("to program.lnk").facts.extension.as_deref(), Some("lnk"));
    assert_eq!(by_name("site.url").facts.extension.as_deref(), Some("url"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_operation_that_ends_later_is_told_when_it_ends_and_can_be_called_off() {
    let root = folder("later");
    let (shown, archive) = (format!("{root}/shown"), format!("{root}/archive"));
    std::fs::create_dir_all(&shown).expect("made");
    std::fs::create_dir_all(&archive).expect("made");
    std::fs::write(format!("{shown}/a.txt"), b"a").expect("written");
    std::fs::write(format!("{shown}/b.txt"), b"b").expect("written");
    let (mut backend, wakes) = backend(&root, FakeExplorer::default());
    let mut all = Vec::new();
    let plain = |path: String| path.replace('/', "\u{5c}");

    // Begun: the caller is answered at once, and the end is told with
    // what takes it back, as `perform` would have answered.
    assert!(backend.begin_operation(7, &FileOperation::Copy { from: plain(format!("{shown}/a.txt")), to_folder: plain(archive.clone()) }));
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::OperationDone { request: 7, undo: Some(FileOperation::Trash { path }), error: None }
            if facts::same_path(path, &format!("{archive}/a.txt")))
    });
    assert!(Path::new(&format!("{archive}/a.txt")).is_file());
    // What goes wrong is told the same way.
    assert!(backend.begin_operation(8, &FileOperation::Copy { from: plain(format!("{shown}/gone.txt")), to_folder: plain(archive.clone()) }));
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::OperationDone { request: 8, undo: None, error: Some(BackendError::NotFound { .. }) })
    });
    // Called off while it waits behind another: it is not begun, and
    // that is told. The one before it is carried out.
    assert!(backend.begin_operation(9, &FileOperation::Copy { from: plain(format!("{shown}/b.txt")), to_folder: plain(archive.clone()) }));
    assert!(backend.begin_operation(10, &FileOperation::Move { from: plain(format!("{shown}/b.txt")), to_folder: plain(archive.clone()) }));
    backend.cancel_operation(10);
    until(&mut backend, &wakes, &mut all, |event| {
        matches!(event, BackendEvent::OperationDone { request: 10, .. })
    });
    assert!(all.iter().any(|event| matches!(event, BackendEvent::OperationDone { request: 9, .. })));
    let called_off = all.iter().any(|event| {
        matches!(event, BackendEvent::OperationDone { request: 10, undo: None, error: Some(BackendError::Cancelled) })
    });
    // (Called off in time, the file is where it was; too late, it was
    // moved and the answer says so.)
    assert_eq!(Path::new(&format!("{shown}/b.txt")).is_file(), called_off, "{all:#?}");
    drop(backend);
    let _ = std::fs::remove_dir_all(&root);
}

/// The shell itself is stopped when an operation is called off: asked
/// before the item whether to go on, it is told no, nothing arrives,
/// and the answer is that it was called off.
#[test]
fn the_shell_is_told_to_stop_when_an_operation_was_called_off() {
    let root = folder("called-off");
    let (shown, archive) = (format!("{root}/shown"), format!("{root}/archive"));
    std::fs::create_dir_all(&shown).expect("made");
    std::fs::create_dir_all(&archive).expect("made");
    std::fs::write(format!("{shown}/a.txt"), b"a").expect("written");
    let plain = |path: String| path.replace('/', "\u{5c}");
    let copy = FileOperation::Copy { from: plain(format!("{shown}/a.txt")), to_folder: plain(archive.clone()) };
    let quiet: crate::zones::sources::Tell = Arc::new(|_| {});
    let mut files = fileops::FileOperations::default();
    files.quiet = true;
    files.watched = Some(crate::zones::operations::Watched::alone(1, Arc::clone(&quiet), true));
    assert_eq!(files.perform(&copy), Err(BackendError::Cancelled));
    assert_eq!(std::fs::read_dir(&archive).expect("listed").count(), 0);
    assert!(Path::new(&format!("{shown}/a.txt")).is_file());
    // Not called off, the same operation is carried out.
    files.watched = Some(crate::zones::operations::Watched::alone(2, quiet, false));
    assert!(matches!(files.perform(&copy), Ok(FileOperation::Trash { .. })));
    assert!(Path::new(&format!("{archive}/a.txt")).is_file());
    let _ = std::fs::remove_dir_all(&root);
}
