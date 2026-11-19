//! The backend as a whole, against a session a test builds under a
//! folder of its own: a home, a desktop, a MIME database, programs, an
//! icon theme. Nothing of the machine the test runs on is read.

use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

const TOOL: &str = "[Desktop Entry]\nType=Application\nName=Tool\nExec=tool --open %f\nIcon=tool\n";
const DRAWING: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#0000ff"/></svg>"##;

/// A session under a folder, and a backend of it.
struct Session {
    root: PathBuf,
    desktop: PathBuf,
    backend: SystemBackend,
    woken: Receiver<()>,
    /// What the backend handed to the launcher.
    started: Rc<RefCell<Vec<Launch>>>,
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("folders");
    std::fs::write(path, text).expect("written");
}

fn session(name: &str) -> Session {
    let root = testing::folder(name);
    let places = Places::under(&root);
    let desktop = places.desktop();
    let share = places.data_dirs[1].clone();
    mime::tests::write_database(&share);
    // Programs, and what they say they open.
    for (id, name, exec) in [
        ("editor.desktop", "Editor", "editor %f"),
        ("viewer.desktop", "Viewer", "viewer %F"),
        ("browser.desktop", "Browser", "browser %u"),
        ("files.desktop", "Files", "files %U"),
    ] {
        write(
            &share.join("applications").join(id),
            &format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={}\n", name.to_lowercase()),
        );
    }
    write(
        &share.join("applications/mimeinfo.cache"),
        "[MIME Cache]\ntext/plain=editor.desktop;viewer.desktop;\nimage/png=viewer.desktop;\nx-scheme-handler/https=browser.desktop;\ninode/directory=files.desktop;\n",
    );
    // An icon theme: a type's icon and a program's.
    write(&share.join("icons/hicolor/scalable/mimetypes/text-plain.svg"), DRAWING);
    write(&share.join("icons/hicolor/scalable/apps/tool.svg"), DRAWING);
    // The desktop.
    write(&desktop.join("a.txt"), "hello");
    write(&desktop.join("çay ☕.txt"), "tea");
    write(&desktop.join(".hidden"), "unseen");
    write(&desktop.join("a.txt~"), "an editor's copy");
    write(&desktop.join("tool.desktop"), TOOL);
    std::fs::create_dir_all(desktop.join("Folder")).expect("a folder");
    std::os::unix::fs::symlink("a.txt", desktop.join("link")).expect("a link");
    image::save_buffer(
        desktop.join("photo.png"),
        &[10u8, 200, 10, 255].repeat(300 * 200),
        300,
        200,
        image::ExtendedColorType::Rgba8,
    )
    .expect("saved");
    let (wake, woken) = std::sync::mpsc::channel();
    let mut backend = SystemBackend::with(System::at(places), move || {
        let _ = wake.send(());
    });
    let started = Rc::new(RefCell::new(Vec::new()));
    let noted = Rc::clone(&started);
    backend.set_launcher(move |launch| {
        noted.borrow_mut().push(launch.clone());
        Ok(())
    });
    Session { root, desktop, backend, woken, started }
}

impl Session {
    fn id(&self, name: &str) -> ItemId {
        uris::item_id(&self.desktop.join(name))
    }

    fn path(&self, name: &str) -> String {
        self.desktop.join(name).to_string_lossy().into_owned()
    }

    /// What the backend tells once it woke and things are quiet.
    fn told(&mut self) -> Vec<BackendEvent> {
        let mut events = Vec::new();
        let mut wait = Duration::from_secs(10);
        while self.woken.recv_timeout(wait).is_ok() {
            events.extend(self.backend.drain_events());
            wait = Duration::from_millis(300);
        }
        events.extend(self.backend.drain_events());
        events
    }

    fn end(self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Events as short words.
fn said(events: &[BackendEvent]) -> Vec<String> {
    let name = |id: &ItemId| {
        uris::path_of_item(id)
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .unwrap_or_default()
    };
    events
        .iter()
        .map(|event| match event {
            BackendEvent::ItemAdded { item, .. } => format!("added {}", name(&item.id)),
            BackendEvent::ItemRemoved { item, .. } => format!("removed {}", name(item)),
            BackendEvent::ItemChanged { item, .. } => format!("changed {}", name(&item.id)),
            BackendEvent::ItemRenamed { from, item, .. } => format!("renamed {} to {}", name(from), name(&item.id)),
            other => format!("{other:?}"),
        })
        .collect()
}

/// A file that came and was written to: told as new once, and after
/// that (the kernel may tell of the writing in a later breath) only as
/// changed.
fn came(events: &[BackendEvent], name: &str) {
    let said = said(events);
    assert_eq!(said.first(), Some(&format!("added {name}")), "{said:?}");
    assert!(said[1..].iter().all(|later| later == &format!("changed {name}")), "{said:?}");
}

#[test]
fn the_desktop_is_listed_with_what_rules_ask_about() {
    let mut session = session("backend-list");
    let items = session.backend.list(&Location::Desktop).expect("listed");
    let by_name = |name: &str| items.iter().find(|item| item.id == session.id(name)).unwrap_or_else(|| panic!("{name}"));
    // What a desktop does not show is not there.
    assert_eq!(items.len(), 6, "{items:?}");
    // An id is the file's address, and leads back to it.
    let tea = by_name("çay ☕.txt");
    assert!(tea.id.as_str().starts_with("file:///"));
    assert!(tea.id.as_str().ends_with("/home/Desktop/%C3%A7ay%20%E2%98%95.txt"));
    assert_eq!(tea.path.as_deref(), Some(session.path("çay ☕.txt").as_str()));
    assert_eq!(tea.name, "çay ☕.txt");

    let text = by_name("a.txt");
    assert_eq!(text.kind, ItemKind::File);
    assert_eq!(text.facts.extension.as_deref(), Some("txt"));
    assert_eq!(text.facts.mime.as_deref(), Some("text/plain"));
    assert_eq!(text.facts.size, Some(5));
    let now = seconds(Some(std::time::SystemTime::now())).expect("a time");
    let modified = text.facts.modified.expect("a time");
    assert!(modified <= now && now - modified < 3600);
    assert!(text.facts.accessed.is_some());

    assert_eq!(by_name("photo.png").facts.mime.as_deref(), Some("image/png"));
    let folder = by_name("Folder");
    assert_eq!((folder.kind, folder.facts.mime.as_deref()), (ItemKind::Folder, Some("inode/directory")));
    assert_eq!((folder.facts.size, folder.facts.extension.clone()), (None, None));
    // A link is a shortcut to what it leads to, and of that thing's type.
    let link = by_name("link");
    assert_eq!(link.kind, ItemKind::Shortcut);
    assert_eq!(link.facts.shortcut_target.as_deref(), Some(session.path("a.txt").as_str()));
    assert_eq!(link.facts.mime.as_deref(), Some("text/plain"));
    // A launcher nobody allowed is a file with its file's name.
    let tool = by_name("tool.desktop");
    assert_eq!((tool.kind, tool.name.as_str()), (ItemKind::File, "tool.desktop"));
    assert_eq!(tool.facts.mime.as_deref(), Some("application/x-desktop"));
    assert_eq!(tool.facts.shortcut_target, None);

    // Allowed, it is a launcher with its own name and what it starts.
    let tool_id = session.id("tool.desktop");
    session.backend.allow_launching(&tool_id).expect("allowed");
    let items = session.backend.list(&Location::Desktop).expect("listed");
    let tool = items.iter().find(|item| item.id == tool_id).expect("there");
    assert_eq!((tool.kind, tool.name.as_str()), (ItemKind::Launcher, "Tool"));
    assert_eq!(tool.facts.shortcut_target.as_deref(), Some("tool"));
    assert_eq!(session.backend.facts(&tool_id), Some(tool.facts.clone()));

    // A folder is listed the same way; one that is not there is an error.
    let inside = session.path("Folder");
    write(&Path::new(&inside).join("inner.txt"), "x");
    let listed = session.backend.list(&Location::Directory { path: inside }).expect("listed");
    assert_eq!(listed.len(), 1);
    let gone = Location::Directory { path: session.path("Nowhere") };
    assert!(matches!(session.backend.list(&gone), Err(BackendError::NotFound { .. })));
    assert!(matches!(session.backend.watch(&gone), Err(BackendError::NotFound { .. })));
    assert_eq!(session.backend.facts(&session.id("nothing.txt")), None);
    session.end();
}

#[test]
fn opening_hands_the_right_program_to_the_launcher() {
    let mut session = session("backend-open");
    let started = Rc::clone(&session.started);
    let last = move || started.borrow_mut().pop().expect("something was started");

    // A file: the program for its type, with the file where it asks.
    session.backend.open(&session.id("çay ☕.txt")).expect("opened");
    let launch = last();
    assert_eq!((launch.name.as_str(), launch.program.as_str()), ("Editor", "editor"));
    assert_eq!(launch.args, [session.path("çay ☕.txt")]);
    assert_eq!(launch.addresses, [session.id("çay ☕.txt").to_string()]);
    assert_eq!((launch.terminal, launch.bus_name.clone()), (false, None));
    // A folder: the file manager, by address as it asks.
    session.backend.open(&session.id("Folder")).expect("opened");
    let launch = last();
    assert_eq!(launch.program, "files");
    assert_eq!(launch.args, [session.id("Folder").to_string()]);

    // A launcher that was not allowed starts nothing.
    let tool = session.id("tool.desktop");
    assert!(matches!(session.backend.open(&tool), Err(BackendError::Refused { .. })));
    assert!(session.started.borrow().is_empty());
    session.backend.allow_launching(&tool).expect("allowed");
    session.backend.open(&tool).expect("started");
    let launch = last();
    assert_eq!((launch.program.as_str(), launch.args.clone()), ("tool", vec!["--open".to_string()]));

    // A link entry opens its address with the program for such addresses.
    write(
        &session.desktop.join("site.desktop"),
        "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org/a b\n",
    );
    let site = session.id("site.desktop");
    assert!(session.backend.open(&site).is_err());
    session.backend.allow_launching(&site).expect("allowed");
    session.backend.open(&site).expect("opened");
    let launch = last();
    assert_eq!((launch.program.as_str(), launch.args.clone()), ("browser", vec!["https://example.org/a b".to_string()]));

    // With a program that is named.
    session.backend.open_with(&session.id("a.txt"), Some("viewer.desktop")).expect("opened");
    assert_eq!(last().program, "viewer");
    session.backend.open_with(&session.id("a.txt"), Some("viewer")).expect("opened");
    assert_eq!(last().args, [session.path("a.txt")]);
    assert!(matches!(
        session.backend.open_with(&session.id("a.txt"), Some("nothing.desktop")),
        Err(BackendError::NotFound { .. })
    ));
    // Without one: the programs to choose from, for the renderer.
    session.backend.open_with(&session.id("a.txt"), None).expect("asked");
    let choice = session.backend.take_program_choice().expect("a choice");
    assert_eq!(choice.item, session.id("a.txt"));
    let names: Vec<&str> = choice.programs.iter().map(|program| program.name.as_str()).collect();
    assert_eq!(names, ["Editor", "Viewer"]);
    assert_eq!(session.backend.take_program_choice(), None);

    // Nothing opens it, or it is not there: said, and nothing started.
    write(&session.desktop.join("data.bin"), "\u{0}\u{1}");
    assert!(matches!(session.backend.open(&session.id("data.bin")), Err(BackendError::NotFound { .. })));
    assert!(matches!(session.backend.open(&session.id("gone.txt")), Err(BackendError::NotFound { .. })));
    assert!(session.started.borrow().is_empty());
    session.end();
}

#[test]
fn the_menu_says_what_can_be_done_with_the_items() {
    let mut session = session("backend-menu");
    let at = Point { x: 40, y: 50 };
    let desktop = session.desktop.to_string_lossy().into_owned();
    let kinds = |menu: &ContextMenu| -> Vec<&'static str> {
        menu.actions
            .iter()
            .map(|action| match action {
                MenuAction::Open => "open",
                MenuAction::OpenWith { .. } => "open with",
                MenuAction::AllowLaunching => "allow",
                MenuAction::Rename => "rename",
                MenuAction::MoveToTrash => "trash",
                MenuAction::NewFolder { .. } => "new folder",
                MenuAction::Properties => "properties",
            })
            .collect()
    };

    // One file. The menu is kept for the renderer, and woken for.
    session.backend.context_menu(&[session.id("a.txt")], at).expect("made");
    assert!(session.woken.try_recv().is_ok());
    let menu = session.backend.take_context_menu().expect("a menu");
    assert_eq!(session.backend.take_context_menu(), None);
    assert_eq!((menu.at, menu.items.clone()), (at, vec![session.id("a.txt")]));
    assert_eq!(kinds(&menu), ["open", "open with", "rename", "trash", "new folder", "properties"]);
    match &menu.actions[1] {
        MenuAction::OpenWith { programs } => {
            let ids: Vec<&str> = programs.iter().map(|program| program.id.as_str()).collect();
            assert_eq!(ids, ["editor.desktop", "viewer.desktop"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(menu.actions.contains(&MenuAction::NewFolder { parent: desktop.clone() }));
    let properties = &menu.properties[0];
    assert_eq!(properties.item.facts.size, Some(5));
    assert_eq!(properties.opens_with.as_deref(), Some("Editor"));
    assert_eq!((properties.mode & 0o600, properties.trusted), (0o600, None));

    // Two of different types: no one program for both, no renaming.
    let menu = session.backend.menu_for(&[session.id("a.txt"), session.id("photo.png")], at);
    assert_eq!(kinds(&menu), ["open", "trash", "new folder", "properties"]);
    assert_eq!(menu.properties.len(), 2);
    // A launcher that was not allowed.
    let menu = session.backend.menu_for(&[session.id("tool.desktop")], at);
    assert_eq!(kinds(&menu), ["open", "allow", "rename", "trash", "new folder", "properties"]);
    assert_eq!(menu.properties[0].trusted, Some(false));
    // The bare ground, and items that went meanwhile.
    for nothing in [vec![], vec![session.id("gone.txt")]] {
        let menu = session.backend.menu_for(&nothing, at);
        assert_eq!(menu.actions, [MenuAction::NewFolder { parent: desktop.clone() }]);
        assert!(menu.items.is_empty() && menu.properties.is_empty());
    }
    session.end();
}

#[test]
fn file_operations_are_told_of_and_each_is_taken_back_by_its_answer() {
    let mut session = session("backend-files");
    let watch = session.backend.watch(&Location::Desktop).expect("watched");
    assert!(session.backend.drain_events().is_empty());
    let desktop = session.desktop.to_string_lossy().into_owned();
    let folder = session.path("Folder");

    // A new folder; where the name is taken, the next one.
    let new = FileOperation::NewFolder { parent: desktop.clone(), name: "Folder".to_string() };
    let undo_new = session.backend.perform(&new).expect("made");
    assert_eq!(undo_new, FileOperation::Trash { path: session.path("Folder (2)") });
    assert_eq!(said(&session.told()), ["added Folder (2)"]);

    // Renamed: one event, and the answer names it back.
    let rename = FileOperation::Rename { path: session.path("a.txt"), new_name: "b.txt".to_string() };
    let undo_rename = session.backend.perform(&rename).expect("renamed");
    assert_eq!(undo_rename, FileOperation::Rename { path: session.path("b.txt"), new_name: "a.txt".to_string() });
    let events = session.told();
    assert_eq!(said(&events), ["renamed a.txt to b.txt"]);
    assert!(matches!(&events[0], BackendEvent::ItemRenamed { location: Location::Desktop, .. }));
    let taken = FileOperation::Rename { path: session.path("b.txt"), new_name: "photo.png".to_string() };
    assert_eq!(session.backend.perform(&taken), Err(BackendError::Exists { what: "photo.png".to_string() }));
    session.backend.perform(&undo_rename).expect("renamed back");
    assert_eq!(said(&session.told()), ["renamed b.txt to a.txt"]);

    // Moved into a folder: gone from the desktop. Back: there again.
    let moving = FileOperation::Move { from: session.path("a.txt"), to_folder: folder.clone() };
    let undo_move = session.backend.perform(&moving).expect("moved");
    assert_eq!(undo_move, FileOperation::Move { from: format!("{folder}/a.txt"), to_folder: desktop.clone() });
    assert_eq!(said(&session.told()), ["removed a.txt"]);
    session.backend.perform(&undo_move).expect("moved back");
    came(&session.told(), "a.txt");

    // Copied; the answer throws the copy away.
    let copying = FileOperation::Copy { from: session.path("photo.png"), to_folder: desktop.clone() };
    let undo_copy = session.backend.perform(&copying).expect("copied");
    assert_eq!(undo_copy, FileOperation::Trash { path: session.path("photo (2).png") });
    came(&session.told(), "photo (2).png");
    let restore_copy = session.backend.perform(&undo_copy).expect("thrown away");
    assert_eq!(said(&session.told()), ["removed photo (2).png"]);
    assert_eq!(restore_copy, FileOperation::Restore { path: session.path("photo (2).png") });

    // Thrown away into the session's own trash, and put back from it.
    let throw = FileOperation::Trash { path: session.path("çay ☕.txt") };
    let undo_throw = session.backend.perform(&throw).expect("thrown away");
    assert_eq!(undo_throw, FileOperation::Restore { path: session.path("çay ☕.txt") });
    assert_eq!(said(&session.told()), ["removed çay ☕.txt"]);
    let can = session.root.join("home/.local/share/Trash");
    assert_eq!(std::fs::read_to_string(can.join("files/çay ☕.txt")).expect("in the can"), "tea");
    assert_eq!(session.backend.perform(&undo_throw).expect("put back"), throw);
    came(&session.told(), "çay ☕.txt");
    assert_eq!(std::fs::read_to_string(session.desktop.join("çay ☕.txt")).expect("back"), "tea");
    // The undo of the new folder.
    session.backend.perform(&undo_new).expect("thrown away");
    assert_eq!(said(&session.told()), ["removed Folder (2)"]);

    // What is not there.
    for missing in [
        FileOperation::Trash { path: session.path("none") },
        FileOperation::Restore { path: session.path("never-was") },
        FileOperation::Copy { from: session.path("none"), to_folder: desktop.clone() },
        FileOperation::Move { from: session.path("a.txt"), to_folder: session.path("Nowhere") },
    ] {
        assert!(matches!(session.backend.perform(&missing), Err(BackendError::NotFound { .. })), "{missing:?}");
    }

    // Something from outside: written, written to again, removed.
    write(&session.desktop.join("download.bin"), "one");
    came(&session.told(), "download.bin");
    write(&session.desktop.join("download.bin"), "one and more");
    let events = session.told();
    assert!(!events.is_empty() && said(&events).iter().all(|said| said == "changed download.bin"), "{events:?}");
    match events.last() {
        Some(BackendEvent::ItemChanged { item, .. }) => assert_eq!(item.facts.size, Some(12)),
        other => panic!("{other:?}"),
    }
    std::fs::remove_file(session.desktop.join("download.bin")).expect("removed");
    assert_eq!(said(&session.told()), ["removed download.bin"]);

    // A launcher that is allowed changes what it is: told as a change.
    session.backend.allow_launching(&session.id("tool.desktop")).expect("allowed");
    let events = session.told();
    match events.last() {
        Some(BackendEvent::ItemChanged { item, .. }) => {
            assert_eq!((item.kind, item.name.as_str()), (ItemKind::Launcher, "Tool"));
        }
        other => panic!("{other:?}"),
    }

    // Let go: nothing more is told.
    session.backend.unwatch(watch);
    write(&session.desktop.join("late.txt"), "late");
    std::thread::sleep(Duration::from_millis(300));
    assert!(session.backend.drain_events().is_empty());
    session.end();
}

#[test]
fn pictures_come_by_handle_from_a_thread_of_their_own() {
    let mut session = session("backend-pictures");
    let answer = |events: &[BackendEvent], asked: IconRequest| {
        events.iter().find_map(|event| match event {
            BackendEvent::IconReady { request, icon } if *request == asked => Some(*icon),
            _ => None,
        })
    };
    let size = IconSize { size: 24, scale: 2.0 };
    assert_eq!(pixels(size), 48);
    assert_eq!(pixels(IconSize { size: 24, scale: f64::NAN }), 24);

    // The icon of a type, from the theme, at the display's pixels.
    let asked = session.backend.request_icon(&session.id("a.txt"), size);
    let events = session.told();
    let handle = answer(&events, asked).expect("answered").expect("an icon");
    let picture = session.backend.picture(handle).expect("pixels").clone();
    assert_eq!((picture.width, picture.height), (48, 48));
    assert_eq!(&picture.pixels[..4], &[0, 0, 255, 255]);
    // Asked again: the same picture, answered at once.
    let again = session.backend.request_icon(&session.id("a.txt"), size);
    assert_eq!(answer(&session.told(), again), Some(Some(handle)));
    // A type the theme has nothing for: answered, with nothing.
    let none = session.backend.request_icon(&session.id("photo.png"), size);
    assert_eq!(answer(&session.told(), none), Some(None));

    // A small picture of a picture; a text has none.
    let thumb = session.backend.request_thumbnail(&session.id("photo.png"), IconSize { size: 96, scale: 1.0 });
    let text = session.backend.request_thumbnail(&session.id("a.txt"), IconSize { size: 96, scale: 1.0 });
    let events = session.told();
    let thumb_handle = answer(&events, thumb).expect("answered").expect("a picture");
    assert_eq!(answer(&events, text), Some(None));
    let small = session.backend.picture(thumb_handle).expect("pixels").clone();
    assert_eq!((small.width, small.height), (96, 64));
    assert_eq!(&small.pixels[..4], &[10, 200, 10, 255]);
    // It was put into the session's store for others to find.
    let kept = session.backend.system().thumbnail_file(&session.desktop.join("photo.png"), 96).expect("kept");
    assert!(kept.starts_with(session.root.join("home/.cache/thumbnails/normal")));

    // A launcher's own icon, once it is allowed: another picture than
    // the one it had as a file.
    let tool = session.id("tool.desktop");
    let before = session.backend.request_icon(&tool, size);
    assert_eq!(answer(&session.told(), before), Some(None));
    session.backend.allow_launching(&tool).expect("allowed");
    let after = session.backend.request_icon(&tool, size);
    let tool_handle = answer(&session.told(), after).expect("answered").expect("its icon");
    assert_ne!(tool_handle, handle);
    assert!(session.backend.picture(tool_handle).is_some());
    assert!(session.backend.picture(IconHandle(999_999)).is_none());
    session.end();
}

#[test]
fn a_changed_file_gets_a_new_picture() {
    let mut session = session("backend-pictures-change");
    session.backend.watch(&Location::Desktop).expect("watched");
    let size = IconSize { size: 64, scale: 1.0 };
    let photo = session.id("photo.png");
    let first = session.backend.request_thumbnail(&photo, size);
    let events = session.told();
    let ready = |events: &[BackendEvent], asked: IconRequest| {
        events.iter().find_map(|event| match event {
            BackendEvent::IconReady { request, icon } if *request == asked => *icon,
            _ => None,
        })
    };
    let old = ready(&events, first).expect("a picture");
    assert_eq!(&session.backend.picture(old).expect("pixels").pixels[..4], &[10, 200, 10, 255]);
    // The file is another picture now.
    image::save_buffer(
        session.desktop.join("photo.png"),
        &[250u8, 0, 0, 255].repeat(40 * 40),
        40,
        40,
        image::ExtendedColorType::Rgba8,
    )
    .expect("saved");
    let events = session.told();
    assert!(said(&events).contains(&"changed photo.png".to_string()), "{events:?}");
    let second = session.backend.request_thumbnail(&photo, size);
    let new = ready(&session.told(), second).expect("a picture");
    assert_ne!(new, old);
    assert_eq!(&session.backend.picture(new).expect("pixels").pixels[..4], &[250, 0, 0, 255]);
    assert!(session.backend.picture(old).is_none());
    session.end();
}

#[test]
fn the_displays_are_the_window_managers_with_its_ids() {
    let state = serde_json::json!({
        "monitors": [
            {
                "name": "HDMI-A-1", "connector": "HDMI-A-1", "stable_id": "GSM-5B7F-0004E2A1",
                "position": {"x": 0.0, "y": 0.0}, "size": {"width": 2560, "height": 1440},
                "work_position": {"x": 0.0, "y": 36.0}, "work_size": {"width": 2560, "height": 1404},
                "is_primary": true, "scale": 1.5
            },
            {
                "name": "eDP-1", "connector": "eDP-1", "stable_id": "",
                "position": {"x": 2560.0, "y": 0.0}, "size": {"width": 1920, "height": 1080},
                "reserved_left": 48, "reserved_bottom": 0, "scale": 0.0
            },
            {"name": "broken", "size": {"width": 0, "height": 0}}
        ]
    });
    let (displays, reserved) = displays_of_state(&state);
    assert_eq!(displays.len(), 2);
    assert_eq!(displays[0].id.as_str(), "GSM-5B7F-0004E2A1");
    assert_eq!((displays[0].rect, displays[0].scale, displays[0].primary), (Rect::new(0, 0, 2560, 1440), 1.5, true));
    // No id from the display itself: the port's.
    assert_eq!(displays[1].id.as_str(), "port:eDP-1");
    assert_eq!((displays[1].rect, displays[1].scale), (Rect::new(2560, 0, 1920, 1080), 1.0));
    assert_eq!(
        reserved,
        [
            ReservedArea { display: displays[0].id.clone(), edge: Edge::Top, thickness: 36 },
            ReservedArea { display: displays[1].id.clone(), edge: Edge::Left, thickness: 48 },
        ]
    );
    assert_eq!(displays_of_state(&serde_json::json!({})), (Vec::new(), Vec::new()));

    // Given to the backend, they are what it answers, and told once.
    let mut session = session("backend-displays");
    assert!(session.backend.displays().is_empty());
    session.backend.set_displays(displays.clone(), reserved.clone());
    session.backend.set_displays(displays.clone(), reserved.clone());
    assert_eq!(session.backend.displays(), displays);
    assert_eq!(session.backend.reserved_areas(), reserved);
    assert_eq!(session.backend.drain_events(), [BackendEvent::Displays { displays, reserved }]);
    session.end();
}

#[test]
fn a_drag_is_kept_for_the_surface_and_a_drop_for_the_core() {
    let mut session = session("backend-drag");
    let items = [session.id("çay ☕.txt"), session.id("gone.txt")];
    session.backend.start_drag(&items, DragActions::ALL).expect("begun");
    let drag = session.backend.take_drag().expect("a drag");
    assert_eq!(session.backend.take_drag(), None);
    // Only what is there is dragged.
    assert_eq!(drag.items, items[..1]);
    assert_eq!(drag.kinds()[0], "text/uri-list");
    let list = String::from_utf8(drag.data("text/uri-list").expect("data")).expect("text");
    assert_eq!(list, format!("{}\r\n", items[0]));
    assert!(session.backend.start_drag(&items[1..], DragActions::ALL).is_err());

    // What another program drags over a zone.
    assert_eq!(session.backend.drop_payload(), None);
    let carried = uris::payload("text/uri-list", list.as_bytes(), DragActions::ALL);
    assert_eq!(carried.paths, [session.path("çay ☕.txt")]);
    session.backend.set_drop(Some(carried.clone()));
    assert_eq!(session.backend.drop_payload(), Some(carried));
    session.backend.set_drop(None);
    assert_eq!(session.backend.drop_payload(), None);
    session.end();
}

#[test]
fn the_core_runs_on_the_backend_through_the_host() {
    use gevrek_zones::core::ZonesCore;
    use gevrek_zones::host::Host;
    let mut session = session("backend-host");
    let (displays, reserved) = displays_of_state(&serde_json::json!({"monitors": [{
        "stable_id": "TST-0001-00000001", "position": {"x": 0, "y": 0},
        "size": {"width": 1920, "height": 1080}, "is_primary": true, "scale": 1.0
    }]}));
    session.backend.set_displays(displays, reserved);
    let config = gevrek_zones::ZonesConfig { enabled: true, ..Default::default() };
    let mut core = ZonesCore::new(config, gevrek_zones::ZonesState::default());
    let mut host = Host::new();
    host.start(&mut core, &mut session.backend, 0);
    let known = |core: &ZonesCore, id: &ItemId| core.item(id).is_some();
    assert!(known(&core, &session.id("a.txt")));
    // What comes onto the desktop from outside reaches the core.
    write(&session.desktop.join("new.txt"), "new");
    assert!(session.woken.recv_timeout(Duration::from_secs(10)).is_ok());
    std::thread::sleep(Duration::from_millis(200));
    host.pump(&mut core, &mut session.backend, 1);
    assert!(known(&core, &session.id("new.txt")));
    host.stop(&mut session.backend);
    session.end();
}
