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
    // Watched, it is told as away instead (see the test of that), and
    // listing it goes nowhere.
    let watching = session.backend.watch(&gone).expect("watched");
    assert_eq!(session.backend.list(&gone).expect("known"), []);
    session.backend.unwatch(watching);
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

impl Session {
    /// Drain until `wanted` holds of everything told so far (kept in
    /// `all`). The wait ends when the backend wakes its host.
    fn until(&mut self, all: &mut Vec<BackendEvent>, wanted: impl Fn(&[BackendEvent]) -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            all.extend(self.backend.drain_events());
            if wanted(all) {
                return;
            }
            let left = end.saturating_duration_since(std::time::Instant::now());
            assert!(!left.is_zero(), "not told in time; told so far: {:?}", said(all));
            let _ = self.woken.recv_timeout(left);
        }
    }
}

/// The names in the listing of `at` that was told last; nothing in it
/// where the last word was that the folder is away.
fn last_listing(all: &[BackendEvent], at: &Location) -> Option<Vec<String>> {
    all.iter().rev().find_map(|event| match event {
        BackendEvent::Listed { location, items } if location == at => {
            Some(items.iter().map(|item| item.name.clone()).collect())
        }
        BackendEvent::ListingFailed { location, .. } if location == at => Some(Vec::new()),
        _ => None,
    })
}

#[test]
fn a_watched_folder_is_read_off_this_thread_and_followed_when_it_goes_and_comes() {
    let mut session = session("backend-portal");
    let shown = session.root.join("deep/shown");
    let at = Location::Directory { path: text(&shown) };
    let mut all = Vec::new();

    // Not there yet: watched all the same, and told as away.
    let watch = session.backend.watch(&at).expect("watched");
    assert_eq!(session.backend.list(&at).expect("known"), []);
    session.until(&mut all, |all| all.iter().any(|event| matches!(event, BackendEvent::ListingFailed { location, .. } if location == &at)));
    assert_eq!(session.backend.unreachable().len(), 1);

    // Made one step at a time: followed down, and told whole.
    std::fs::create_dir(session.root.join("deep")).expect("made");
    std::fs::create_dir(&shown).expect("made");
    write(&shown.join("a.txt"), "a");
    session.until(&mut all, |all| {
        last_listing(all, &at) == Some(vec!["a.txt".to_string()])
            || all.iter().any(|event| matches!(event, BackendEvent::ItemAdded { item, .. } if item.name == "a.txt"))
    });
    assert_eq!(session.backend.unreachable(), []);
    assert_eq!(session.backend.list(&at).expect("listed").len(), 1);
    // What changes in it is told as for any folder.
    write(&shown.join("b.txt"), "b");
    session.until(&mut all, |all| all.iter().any(|event| matches!(event, BackendEvent::ItemAdded { item, .. } if item.name == "b.txt")));

    // Moved away: nothing is in it any more.
    all.clear();
    std::fs::rename(&shown, session.root.join("deep/elsewhere")).expect("renamed");
    session.until(&mut all, |all| all.iter().any(|event| matches!(event, BackendEvent::ListingFailed { location, .. } if location == &at)));
    assert_eq!(session.backend.unreachable().len(), 1);
    // What happens where it went is nobody's business here.
    write(&session.root.join("deep/elsewhere/c.txt"), "c");

    // Another folder of that name, with other things in it.
    all.clear();
    std::fs::create_dir(&shown).expect("made");
    write(&shown.join("new.txt"), "new");
    session.until(&mut all, |all| {
        last_listing(all, &at) == Some(vec!["new.txt".to_string()])
            || all.iter().any(|event| matches!(event, BackendEvent::ItemAdded { item, .. } if item.name == "new.txt"))
    });
    assert!(!said(&all).iter().any(|word| word.contains("c.txt")), "{:?}", said(&all));

    // Deleted with everything in it; asked to try again while it is
    // away, it is still away.
    all.clear();
    std::fs::remove_dir_all(&shown).expect("removed");
    session.until(&mut all, |all| all.iter().any(|event| matches!(event, BackendEvent::ListingFailed { .. })));
    all.clear();
    assert!(session.backend.begin_listing(&at));
    session.until(&mut all, |all| all.iter().any(|event| matches!(event, BackendEvent::ListingFailed { .. })));

    // A folder nobody watches is read once, off this thread too.
    let other = Location::Directory { path: session.path("Folder") };
    assert!(session.backend.begin_listing(&other));
    session.until(&mut all, |all| last_listing(all, &other) == Some(Vec::new()));
    let nowhere = Location::Directory { path: session.path("Nowhere") };
    assert!(session.backend.begin_listing(&nowhere));
    session.until(&mut all, |all| {
        all.iter().any(|event| matches!(event, BackendEvent::ListingFailed { location, error: BackendError::NotFound { .. } } if location == &nowhere))
    });
    // The desktop is this machine's own and is listed at once.
    assert!(!session.backend.begin_listing(&Location::Desktop));
    session.backend.unwatch(watch);
    session.end();
}

#[test]
fn a_folder_that_does_not_answer_holds_nobody_and_is_shown_when_it_does() {
    let mut session = session("backend-slow");
    let shown = session.path("Folder");
    write(&Path::new(&shown).join("inner.txt"), "x");
    let at = Location::Directory { path: shown.clone() };
    // A mount that takes its time: the going to it waits until the test
    // lets it through.
    let (let_through, gate) = std::sync::mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    session.backend.set_reach(portal::How {
        patience: Duration::from_millis(100),
        reach: Box::new(move |path| {
            let _ = gate.lock().expect("the gate").recv();
            path.is_dir()
        }),
    });
    let asked = std::time::Instant::now();
    session.backend.watch(&at).expect("watched");
    assert_eq!(session.backend.list(&at).expect("known"), []);
    assert!(asked.elapsed() < Duration::from_millis(50), "{:?}", asked.elapsed());
    let mut all = Vec::new();
    session.until(&mut all, |all| {
        all.iter().any(|event| matches!(event, BackendEvent::ListingFailed { error: BackendError::NotFound { what }, .. } if what.contains("does not answer")))
    });
    assert_eq!(session.backend.unreachable().len(), 1);
    drop(let_through);
    session.until(&mut all, |all| last_listing(all, &at) == Some(vec!["inner.txt".to_string()]));
    assert_eq!(session.backend.unreachable(), []);
    session.end();
}

/// A folder of five thousand files: the caller is not held, and the
/// listing comes in one piece. How long it took is printed (and written
/// into the notes); only a bound far above it is asserted.
#[test]
fn five_thousand_files_are_listed_without_holding_the_caller() {
    let mut session = session("backend-many");
    let many = session.root.join("many");
    std::fs::create_dir_all(&many).expect("made");
    for number in 0..5000 {
        std::fs::write(many.join(format!("file {number:04}.txt")), b"x").expect("written");
    }
    let at = Location::Directory { path: text(&many) };
    let asked = std::time::Instant::now();
    session.backend.watch(&at).expect("watched");
    let held = asked.elapsed();
    let mut all = Vec::new();
    session.until(&mut all, |all| last_listing(all, &at).is_some_and(|names| names.len() == 5000));
    let took = asked.elapsed();
    println!("5000 files: the caller was held {held:?}; listed after {took:?}");
    assert!(held < Duration::from_millis(100), "{held:?}");
    assert!(took < Duration::from_secs(60), "{took:?}");
    session.end();
}

#[test]
fn recent_files_and_downloads_are_told_whole_from_the_sessions_own_folders() {
    let mut session = session("backend-smart");
    let mut all = Vec::new();
    let listed = |all: &[BackendEvent], of: &SmartSource| -> Option<Vec<String>> {
        all.iter().rev().find_map(|event| match event {
            BackendEvent::SourceListed { source, items } if source == of => {
                Some(items.iter().map(|item| item.name.clone()).collect())
            }
            _ => None,
        })
    };
    assert_eq!(session.backend.watch_source(&SmartSource::Desktop), Err(BackendError::Unsupported));
    assert_eq!(
        session.backend.list_source(&SmartSource::Folder { directory: session.path("Folder") }),
        Err(BackendError::Unsupported)
    );

    // Recent files: no record yet, so none.
    let data = session.backend.system().places.data_home.clone();
    std::fs::create_dir_all(&data).expect("folders");
    let recent = session.backend.watch_source(&SmartSource::Recent).expect("watched");
    session.until(&mut all, |all| listed(all, &SmartSource::Recent) == Some(Vec::new()));
    // A program writes the record anew (beside it, then in its place,
    // as GTK does): the list follows.
    let record = format!(
        "<xbel>\n<bookmark href=\"{}\" visited=\"2026-11-19T09:00:00Z\"/>\n<bookmark href=\"{}\" visited=\"2026-11-18T09:00:00Z\"/>\n</xbel>\n",
        session.id("çay ☕.txt"),
        session.id("a.txt"),
    );
    write(&data.join("recently-used.xbel.tmp"), &record);
    std::fs::rename(data.join("recently-used.xbel.tmp"), data.join("recently-used.xbel")).expect("in place");
    session.until(&mut all, |all| listed(all, &SmartSource::Recent) == Some(vec!["çay ☕.txt".to_string(), "a.txt".to_string()]));
    let known = session.backend.list_source(&SmartSource::Recent).expect("known");
    assert_eq!(known[0].facts.accessed, Some(1_795_078_800));
    assert_eq!(known[0].id, session.id("çay ☕.txt"));

    // Downloads: the folder `user-dirs.dirs` names, the newest first.
    assert!(matches!(session.backend.watch_source(&SmartSource::Downloads), Err(BackendError::NotFound { .. })));
    let home = session.backend.system().places.home.clone();
    write(&home.join(".config/user-dirs.dirs"), "XDG_DOWNLOAD_DIR=\"$HOME/Incoming\"\n");
    std::fs::create_dir_all(home.join("Incoming")).expect("made");
    let downloads = session.backend.watch_source(&SmartSource::Downloads).expect("watched");
    session.until(&mut all, |all| listed(all, &SmartSource::Downloads) == Some(Vec::new()));
    write(&home.join("Incoming/setup.run"), "#!/bin/sh\n");
    session.until(&mut all, |all| listed(all, &SmartSource::Downloads) == Some(vec!["setup.run".to_string()]));

    // Let go of, nothing more is told of either.
    session.backend.unwatch(recent);
    session.backend.unwatch(downloads);
    std::thread::sleep(Duration::from_millis(300));
    let _ = session.backend.drain_events();
    write(&home.join("Incoming/late.txt"), "late");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!session.backend.drain_events().iter().any(|event| matches!(event, BackendEvent::SourceListed { .. })));
    session.end();
}

#[test]
fn programs_are_items_that_start_or_come_to_the_front() {
    let mut session = session("backend-programs");
    let mut all = Vec::new();
    let listed = |all: &[BackendEvent], of: &SmartSource| -> Option<Vec<String>> {
        all.iter().rev().find_map(|event| match event {
            BackendEvent::SourceListed { source, items } if source == of => {
                Some(items.iter().map(|item| item.id.to_string()).collect())
            }
            _ => None,
        })
    };
    // The launcher counted two starts of the viewer; the editor is
    // pinned. (The programs are the session's own four.)
    let state = session.backend.system().state.clone();
    write(&state.join("launcher-usage.json"), r#"{"viewer":2}"#);
    session.backend.pin_launcher("editor", true).expect("pinned");
    assert_eq!(session.backend.pinned_launchers(), ["editor"]);
    session.backend.watch_source(&SmartSource::Pinned).expect("watched");
    session.until(&mut all, |all| listed(all, &SmartSource::Pinned) == Some(vec!["app:editor".to_string(), "app:viewer".to_string()]));
    let names: Vec<String> = session.backend.installed_programs().into_iter().map(|item| item.name).collect();
    assert_eq!(names, ["Browser", "Editor", "Files", "Viewer"]);

    // Opening a launcher starts its program, by the shell's own way.
    let editor = ItemId::new("app:editor");
    assert_eq!(session.backend.facts(&editor).and_then(|facts| facts.shortcut_target).as_deref(), Some("editor"));
    session.backend.open(&editor).expect("started");
    let launch = session.started.borrow_mut().pop().expect("started");
    assert_eq!((launch.name.as_str(), launch.program.as_str(), launch.args.len()), ("Editor", "editor", 0));
    assert!(matches!(session.backend.open(&ItemId::new("app:nothing")), Err(BackendError::NotFound { .. })));
    assert_eq!(session.backend.open_with(&editor, None), Err(BackendError::Unsupported));
    assert_eq!(session.backend.context_menu(std::slice::from_ref(&editor), Point { x: 0, y: 0 }), Err(BackendError::Unsupported));
    assert!(session.backend.start_drag(std::slice::from_ref(&editor), DragActions::ALL).is_err());

    // The programs that run, out of the window manager's state; opening
    // one is a command for the window manager.
    session.backend.watch_source(&SmartSource::RunningApps).expect("watched");
    session.backend.set_wm_state(&serde_json::json!({"monitors": [{"workspaces": [{"detailed_windows": [
        {"window_id": 31, "title": "a.txt", "app_id": "editor"},
        {"window_id": 32, "title": "b.txt", "app_id": "editor", "is_focused": true}]}]}]}));
    session.until(&mut all, |all| listed(all, &SmartSource::RunningApps) == Some(vec!["running:editor".to_string()]));
    let running = ItemId::new("running:editor");
    assert_eq!(session.backend.open(&running), Err(BackendError::Unsupported));
    let (sent, commands) = std::sync::mpsc::channel();
    session.backend.set_commander(move |command| {
        let _ = sent.send(command);
    });
    session.backend.open(&running).expect("sent");
    assert_eq!(commands.try_recv().expect("a command"), crate::zones::programs::focus_command(32));
    session.end();
}

/// Reading a file for its kind or its small picture is not the person
/// using it: where the kernel allows, the time of last access stays.
#[test]
fn a_file_read_for_the_zones_own_purposes_keeps_its_time_of_last_access() {
    use std::io::Read;
    let root = testing::folder("quiet");
    let file = root.join("read.bin");
    std::fs::write(&file, vec![7u8; 4096]).expect("written");
    // Last used long ago, and written before that (so that a kernel
    // which moves the time only once a day would move it now).
    let long_ago = std::time::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    let times = std::fs::FileTimes::new().set_accessed(long_ago).set_modified(long_ago - Duration::from_secs(60));
    std::fs::File::options().write(true).open(&file).expect("opened").set_times(times).expect("times");
    let mut read = Vec::new();
    open_quietly(&file).expect("opened").read_to_end(&mut read).expect("read");
    assert_eq!(read.len(), 4096);
    let accessed = std::fs::metadata(&file).expect("there").accessed().expect("a time");
    assert_eq!(accessed, long_ago, "reading moved the time of last access");
    // A file that is not there is said to be not there.
    assert!(open_quietly(&root.join("gone")).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

/// What rules by "where a shortcut leads" and by type are asked of, on
/// real links: to a folder, to nothing, and an entry that is a link to
/// an address.
#[test]
fn links_say_where_they_lead_also_when_that_is_gone() {
    let mut session = session("backend-links");
    std::os::unix::fs::symlink(session.path("Folder"), session.desktop.join("to folder")).expect("a link");
    std::os::unix::fs::symlink("nowhere.txt", session.desktop.join("broken")).expect("a link");
    write(&session.desktop.join("site.desktop"), "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org/page\n");
    let site = session.id("site.desktop");
    session.backend.allow_launching(&site).expect("allowed");
    let items = session.backend.list(&Location::Desktop).expect("listed");
    let by_name = |name: &str| items.iter().find(|item| item.id == session.id(name)).unwrap_or_else(|| panic!("{name}"));

    // A link to a folder is a shortcut of a folder's type.
    let folder = by_name("to folder");
    assert_eq!(folder.kind, ItemKind::Shortcut);
    assert_eq!(folder.facts.shortcut_target.as_deref(), Some(session.path("Folder").as_str()));
    assert_eq!(folder.facts.mime.as_deref(), Some("inode/directory"));
    // A link to nothing is still a shortcut, and says where it would
    // lead; what it has no type of its own.
    let broken = by_name("broken");
    assert_eq!(broken.kind, ItemKind::Shortcut);
    assert_eq!(broken.facts.shortcut_target.as_deref(), Some(session.path("nowhere.txt").as_str()));
    assert_eq!(broken.facts.size, None);
    assert!(broken.facts.modified.is_some());
    // An entry that is a link leads to its address, under its own name.
    let link = by_name("site.desktop");
    assert_eq!((link.kind, link.name.as_str()), (ItemKind::Shortcut, "Site"));
    assert_eq!(link.facts.shortcut_target.as_deref(), Some("https://example.org/page"));
    assert_eq!(link.facts.mime.as_deref(), Some("application/x-desktop"));
    assert_eq!(link.facts.extension.as_deref(), Some("desktop"));
    session.end();
}

#[test]
fn an_operation_that_ends_later_is_told_when_it_ends_and_can_be_called_off() {
    let mut session = session("backend-later");
    let archive = session.root.join("archive");
    std::fs::create_dir_all(&archive).expect("made");
    // Large enough to be copied in several pieces.
    std::fs::write(session.desktop.join("large.bin"), vec![3u8; 5 << 20]).expect("written");
    let mut all = Vec::new();
    let done = |all: &[BackendEvent], wanted: u64| {
        all.iter().find_map(|event| match event {
            BackendEvent::OperationDone { request, undo, error } if *request == wanted => Some((undo.clone(), error.clone())),
            _ => None,
        })
    };

    // Begun: the caller is answered at once; how far it is, is told on
    // the way, and its end with what takes it back.
    let copy = FileOperation::Copy { from: session.path("large.bin"), to_folder: text(&archive) };
    assert!(session.backend.begin_operation(7, &copy));
    session.until(&mut all, |all| done(all, 7).is_some());
    assert_eq!(done(&all, 7), Some((Some(FileOperation::Trash { path: text(&archive.join("large.bin")) }), None)));
    let total = 5u64 << 20;
    assert!(all.iter().any(|event| matches!(event, BackendEvent::OperationProgress { request: 7, done, total: of } if *of == total && *done == total)), "{:?}", said(&all));
    assert_eq!(std::fs::metadata(archive.join("large.bin")).expect("copied").len(), total);

    // What goes wrong is told the same way.
    let gone = FileOperation::Copy { from: session.path("gone.bin"), to_folder: text(&archive) };
    assert!(session.backend.begin_operation(8, &gone));
    session.until(&mut all, |all| done(all, 8).is_some());
    assert!(matches!(done(&all, 8), Some((None, Some(BackendError::NotFound { .. })))));

    // Called off before it began: told as called off, and nothing of
    // it is there.
    assert!(session.backend.begin_operation(9, &copy));
    assert!(session.backend.begin_operation(10, &copy));
    session.backend.cancel_operation(10);
    session.until(&mut all, |all| done(all, 10).is_some());
    assert!(done(&all, 9).is_some_and(|(undo, _)| undo.is_some()));
    if done(&all, 10) == Some((None, Some(BackendError::Cancelled))) {
        let copies = std::fs::read_dir(&archive).expect("listed").count();
        assert_eq!(copies, 2, "half a copy was left");
    }
    session.end();
}

/// A copy that is called off half way leaves nothing behind.
#[test]
fn a_copy_that_is_called_off_half_way_leaves_nothing() {
    let root = testing::folder("called-off");
    let (from, into) = (root.join("large.bin"), root.join("into"));
    std::fs::create_dir_all(&into).expect("made");
    std::fs::write(&from, vec![5u8; 3 << 20]).expect("written");
    let names = |folder: &Path| -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(folder).expect("listed").flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    };
    let called_off = || std::io::Error::new(std::io::ErrorKind::Interrupted, "called off");
    let mut told = Vec::new();
    let mut while_copying = Vec::new();
    let copied = files::copy_told(&from, &into, &mut |done, total| {
        told.push((done, total));
        // While it is written it has no real name: only one that begins
        // with a dot, which nothing shows.
        while_copying = names(&into);
        if done < 2 << 20 { Ok(()) } else { Err(called_off()) }
    });
    assert_eq!(copied.expect_err("called off").kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(told, [(1 << 20, 3 << 20), (2 << 20, 3 << 20)]);
    assert_eq!(while_copying.len(), 1);
    assert!(while_copying[0].starts_with(".large.bin.gevrek-part-"), "{while_copying:?}");
    assert_eq!(names(&into), Vec::<String>::new());
    assert!(from.is_file());

    // The disk runs full half way (played): the same, with that error.
    let full = files::copy_told(&from, &into, &mut |done, _| {
        if done < 2 << 20 { Ok(()) } else { Err(std::io::Error::new(std::io::ErrorKind::StorageFull, "no room")) }
    });
    assert_eq!(full.expect_err("no room").kind(), std::io::ErrorKind::StorageFull);
    assert_eq!(names(&into), Vec::<String>::new());

    // The folder it was going to is taken away half way: an error, and
    // the file is where it was.
    let leaving = root.join("leaving");
    std::fs::create_dir_all(&leaving).expect("made");
    let vanished = files::move_into_told(&from, &leaving, &mut |_, _| Ok(())).map(|_| ());
    // (Within one file system a move is a renaming and cannot be caught
    // half way; a copy can.)
    assert!(vanished.is_ok());
    let back = files::move_into(&leaving.join("large.bin"), &root).expect("back");
    assert_eq!(back, from);
    let vanished = files::copy_told(&from, &leaving, &mut |_, _| {
        let _ = std::fs::remove_dir_all(&leaving);
        Ok(())
    });
    assert!(vanished.is_err());
    assert!(!leaving.exists());
    assert!(from.is_file());

    // A folder of many files is called off between two of them, and
    // nothing of it is left; let through, it arrives whole and a second
    // one beside it under the next free name.
    let tree = root.join("tree");
    for number in 0..20 {
        std::fs::create_dir_all(tree.join("inner")).expect("made");
        std::fs::write(tree.join(format!("inner/{number}.txt")), b"").expect("written");
    }
    let mut asked = 0;
    let stopped = files::copy_told(&tree, &into, &mut |_, _| {
        asked += 1;
        if asked < 10 { Ok(()) } else { Err(called_off()) }
    });
    assert_eq!(stopped.expect_err("called off").kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(names(&into), Vec::<String>::new());
    assert_eq!(files::copy(&tree, &into).expect("copied"), into.join("tree"));
    assert_eq!(files::copy(&tree, &into).expect("copied"), into.join("tree (2)"));
    assert_eq!(names(&into.join("tree (2)/inner")).len(), 20);
    assert_eq!(names(&into), ["tree", "tree (2)"]);
    let _ = std::fs::remove_dir_all(&root);
}
