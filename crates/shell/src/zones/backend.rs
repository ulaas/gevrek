//! The system's backend, as the zones here use it: `gevrek_system::zones::
//! SystemBackend` with the little a renderer asks besides ([`Desk`]).
//! Each system's own part is a few lines; everything else is the same.
//!
//! Nothing here changes anything of the system's own desktop: the
//! backend's call that puts Explorer's icons away is not made from the
//! shell.

#[cfg(not(windows))]
use super::DeskAction;
use super::{Desk, Spot};
use gevrek_system::icons::Picture;
use gevrek_system::zones::{displays_of_state, SystemBackend};
use gevrek_zones::backend::{BackendError, IconHandle, ZonesBackend};
use gevrek_zones::{DisplayId, ItemId};

/// How a program is started for the zones: by the window manager, as
/// everything the shell starts is (`{"command": "exec", ...}` over the
/// line to it), so that it is no child of the shell and does not end
/// with it.
// (Windows starts what an item opens through its own shell, and asks
// nothing of this.)
#[cfg_attr(windows, allow(dead_code))]
pub struct Starter {
    /// Gives the window manager a command, as JSON.
    pub dispatch: Box<dyn Fn(String)>,
    /// The terminal a program that wants one is started in.
    pub terminal: String,
}

impl Starter {
    /// One that starts nothing (where nothing will be opened).
    pub fn none() -> Self {
        Self { dispatch: Box::new(|_| {}), terminal: String::new() }
    }
}

/// The command that starts a program for the window manager: the file
/// and its arguments as they are, in the terminal when it asks for one
/// (as `launcher::start_command` does for an application). Nothing for
/// what has no program to run.
#[cfg_attr(windows, allow(dead_code))]
pub fn exec_command(program: &str, args: &[String], terminal: Option<&str>) -> Option<serde_json::Value> {
    if program.trim().is_empty() {
        return None;
    }
    let mut words: Vec<String> = std::iter::once(program.to_string()).chain(args.iter().cloned()).collect();
    if let Some(terminal) = terminal {
        if terminal.trim().is_empty() {
            return None;
        }
        words.splice(0..0, [terminal.to_string(), "-e".to_string()]);
    }
    let (program, args) = words.split_first()?;
    Some(serde_json::json!({"command": "exec", "program": program, "args": args}))
}

/// What the Linux backend starts programs with: the command goes to the
/// window manager, and no process is made here.
#[cfg(not(windows))]
fn launcher(starter: Starter) -> impl FnMut(&gevrek_system::zones::Launch) -> std::io::Result<()> {
    move |launch| {
        let terminal = launch.terminal.then_some(starter.terminal.as_str());
        let command = exec_command(&launch.program, &launch.args, terminal).ok_or_else(|| {
            // (Started only by a message on the session's bus, or it
            // wants a terminal and none is known.)
            std::io::Error::new(std::io::ErrorKind::Unsupported, format!("{} has no program to run", launch.name))
        })?;
        (starter.dispatch)(command.to_string());
        Ok(())
    }
}

/// The backend of this system. `wake` is for it to call (from any thread)
/// when it has news; `starter` is how it starts programs.
pub fn desk(wake: impl Fn() + Send + Sync + 'static, starter: Starter) -> Option<Box<dyn Desk>> {
    #[cfg_attr(windows, allow(unused_mut))]
    let mut backend = SystemBackend::new(wake);
    #[cfg(not(windows))]
    backend.set_launcher(launcher(starter));
    #[cfg(windows)]
    let _ = starter;
    Some(Box::new(System { backend, outputs: Vec::new() }))
}

/// The backend, and what the window manager said of the displays that
/// only the surfaces need.
pub struct System {
    backend: SystemBackend,
    /// Each display's id with the name its output is known by.
    outputs: Vec<(DisplayId, String)>,
}

/// The id a display of the state has for the zones, as the backends make
/// it (`displays_of_state`), with the name of its output.
fn outputs_of(state: &serde_json::Value) -> Vec<(DisplayId, String)> {
    state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|monitor| {
            let said = |key: &str| monitor[key].as_str().filter(|text| !text.is_empty());
            let port = said("connector").or_else(|| said("name"));
            let id = said("stable_id").map(str::to_string).or_else(|| port.map(|port| format!("port:{port}")))?;
            Some((DisplayId::new(id), port.unwrap_or_default().to_string()))
        })
        .collect()
}

impl Desk for System {
    fn picture(&mut self, icon: IconHandle) -> Option<Picture> {
        self.backend.picture(icon).cloned()
    }

    fn told(&mut self, state: &serde_json::Value) {
        self.outputs = outputs_of(state);
        let (displays, reserved) = displays_of_state(state);
        self.backend.set_displays(displays, reserved);
    }

    fn spot_of(&mut self, display: &DisplayId) -> Option<Spot> {
        let output = self.outputs.iter().find(|(known, _)| known == display).map(|(_, output)| output.clone())?;
        #[cfg(windows)]
        let pixels = self.backend.pixels_of(display)?;
        // (A Wayland compositor places a surface by its output's name.)
        #[cfg(not(windows))]
        let pixels = (0, 0, 0, 0);
        Some(Spot { output, pixels })
    }

    fn desktop(&mut self) -> Option<String> {
        if let Some(folder) = self.backend.desktop_folder() {
            return Some(folder);
        }
        // (Until the backend says it itself.)
        #[cfg(windows)]
        {
            gevrek_system::zones::shell::desktop_folders().into_iter().next()
        }
        #[cfg(not(windows))]
        {
            Some(self.backend.system().places.desktop().to_string_lossy().into_owned())
        }
    }

    #[cfg(windows)]
    fn hide_system_icons(&mut self) -> Result<(), BackendError> {
        self.backend.hide_system_icons()
    }

    #[cfg(windows)]
    fn set_owner(&mut self, window: isize) {
        self.backend.set_owner(window);
    }

    #[cfg(windows)]
    fn drags_everything(&self) -> bool {
        true
    }

    #[cfg(windows)]
    fn begin_drag(
        &mut self,
        items: &[ItemId],
        control: &mut gevrek_shellkit::host::Control,
        from: gevrek_shellkit::host::SurfaceId,
        image: Option<gevrek_shellkit::dnd::DragImage>,
    ) -> bool {
        use gevrek_zones::backend::DragActions;
        // (A link to something of the desktop is not ours to offer.)
        let allowed = DragActions { copy: true, moving: true, link: false };
        if let Err(error) = self.backend.start_drag(items, allowed) {
            log::warn!("the system's description of what is dragged could not be made: {error}");
            return false;
        }
        let Some(out) = self.backend.take_drag() else {
            return false;
        };
        let allowed = gevrek_shellkit::dnd::DragActions {
            copy: out.allowed.copy,
            moving: out.allowed.moving,
            link: out.allowed.link,
        };
        control.start_drag_of(from, out.object, allowed, image);
        true
    }

    #[cfg(windows)]
    fn dropped(
        &mut self,
        control: &mut gevrek_shellkit::host::Control,
        on: gevrek_shellkit::host::SurfaceId,
        allowed: gevrek_zones::backend::DragActions,
    ) -> Option<gevrek_zones::backend::DropPayload> {
        let object = control.dropped_object(on)?;
        let mut payload = gevrek_system::zones::dnd::payload_of(&object, allowed);
        // (What has no path is named in the list of shell ids only.)
        if let Some(list) = gevrek_system::zones::dnd::id_list_of(&object) {
            for item in gevrek_system::zones::dnd::items_of_id_list(&list) {
                if !payload.items.contains(&item) {
                    payload.items.push(item);
                }
            }
        }
        Some(payload)
    }

    #[cfg(windows)]
    fn items_named(&mut self, kind: &str, bytes: &[u8]) -> Vec<ItemId> {
        if kind.contains("IDList") {
            gevrek_system::zones::dnd::items_of_id_list(bytes)
        } else {
            Vec::new()
        }
    }

    #[cfg(not(windows))]
    fn own_menu(&mut self, items: &[ItemId]) -> Option<Vec<DeskAction>> {
        use gevrek_system::zones::MenuAction;
        let menu = self.backend.menu_for(items, gevrek_model::geometry::Point::default());
        let line = |id: &str, label: &str| DeskAction { id: id.to_string(), label: label.to_string() };
        let mut lines = Vec::new();
        for action in menu.actions {
            match action {
                MenuAction::Open => lines.push(line("open", "Open")),
                MenuAction::OpenWith { programs } => {
                    for program in programs.into_iter().take(6) {
                        lines.push(DeskAction {
                            id: format!("with:{}", program.id),
                            label: format!("Open with {}", program.name),
                        });
                    }
                }
                MenuAction::AllowLaunching => lines.push(line("allow", "Allow launching")),
                MenuAction::Rename => lines.push(line(super::RENAME, "Rename\u{2026}")),
                MenuAction::MoveToTrash => lines.push(line(super::TRASH, "Move to the bin")),
                // (A new folder and the properties are not drawn yet.)
                MenuAction::NewFolder { .. } | MenuAction::Properties => {}
            }
        }
        Some(lines)
    }

    #[cfg(not(windows))]
    fn act(&mut self, items: &[ItemId], action: &str) -> Result<(), BackendError> {
        for item in items {
            match action {
                "open" => self.backend.open(item)?,
                "allow" => self.backend.allow_launching(item)?,
                with => match with.strip_prefix("with:") {
                    Some(program) => self.backend.open_with(item, Some(program))?,
                    None => return Err(BackendError::Unsupported),
                },
            }
        }
        Ok(())
    }
}

// Everything the core asks goes to the backend as it is.
impl ZonesBackend for System {
    fn list(&mut self, location: &gevrek_zones::backend::Location) -> Result<Vec<gevrek_zones::Item>, BackendError> {
        self.backend.list(location)
    }
    fn watch(&mut self, location: &gevrek_zones::backend::Location) -> Result<gevrek_zones::backend::WatchId, BackendError> {
        self.backend.watch(location)
    }
    fn unwatch(&mut self, watch: gevrek_zones::backend::WatchId) {
        self.backend.unwatch(watch)
    }
    fn drain_events(&mut self) -> Vec<gevrek_zones::BackendEvent> {
        self.backend.drain_events()
    }
    fn facts(&mut self, item: &ItemId) -> Option<gevrek_zones::item::ItemFacts> {
        self.backend.facts(item)
    }
    fn request_icon(&mut self, item: &ItemId, size: gevrek_zones::backend::IconSize) -> gevrek_zones::backend::IconRequest {
        self.backend.request_icon(item, size)
    }
    fn request_thumbnail(&mut self, item: &ItemId, size: gevrek_zones::backend::IconSize) -> gevrek_zones::backend::IconRequest {
        self.backend.request_thumbnail(item, size)
    }
    fn open(&mut self, item: &ItemId) -> Result<(), BackendError> {
        self.backend.open(item)
    }
    fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
        self.backend.open_with(item, program)
    }
    fn context_menu(&mut self, items: &[ItemId], at: gevrek_model::geometry::Point) -> Result<(), BackendError> {
        self.backend.context_menu(items, at)
    }
    fn perform(&mut self, operation: &gevrek_zones::backend::FileOperation) -> Result<gevrek_zones::backend::FileOperation, BackendError> {
        self.backend.perform(operation)
    }
    fn drop_payload(&mut self) -> Option<gevrek_zones::backend::DropPayload> {
        self.backend.drop_payload()
    }
    fn start_drag(&mut self, items: &[ItemId], allowed: gevrek_zones::backend::DragActions) -> Result<(), BackendError> {
        self.backend.start_drag(items, allowed)
    }
    fn displays(&mut self) -> Vec<gevrek_zones::geometry::Display> {
        self.backend.displays()
    }
    fn reserved_areas(&mut self) -> Vec<gevrek_zones::geometry::ReservedArea> {
        self.backend.reserved_areas()
    }
    fn desktop_folder(&mut self) -> Option<String> {
        self.backend.desktop_folder()
    }
    fn journal_system_icons(&mut self) -> Result<(), BackendError> {
        self.backend.journal_system_icons()
    }
    fn system_icons_to_restore(&mut self) -> bool {
        self.backend.system_icons_to_restore()
    }
    fn restore_system_icons(&mut self) -> Result<(), BackendError> {
        self.backend.restore_system_icons()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_of_the_state_has_the_backends_id_and_its_outputs_name() {
        let state = serde_json::json!({ "monitors": [
            { "name": "DP-1", "stable_id": "GSM-5B7F-0004E2A1" },
            { "name": "HDMI-A-1", "connector": "HDMI-A-1" },
            { "name": "", "stable_id": "" },
        ]});
        assert_eq!(
            outputs_of(&state),
            [
                (DisplayId::new("GSM-5B7F-0004E2A1"), "DP-1".to_string()),
                (DisplayId::new("port:HDMI-A-1"), "HDMI-A-1".to_string()),
            ]
        );
        // The same ids the backend gives the core for these displays.
        let sized = serde_json::json!({ "monitors": [{
            "name": "DP-1", "stable_id": "GSM-5B7F-0004E2A1",
            "position": { "x": 0, "y": 0 }, "size": { "width": 1920, "height": 1080 }, "scale": 1.0
        }]});
        let (displays, _) = displays_of_state(&sized);
        assert_eq!(displays[0].id, outputs_of(&sized)[0].0);
    }

    #[test]
    fn a_program_is_started_by_a_command_to_the_window_manager() {
        let args = vec!["--new-window".to_string(), "/home/ada/a b.txt".to_string()];
        assert_eq!(
            exec_command("/usr/bin/editor", &args, None),
            Some(serde_json::json!({
                "command": "exec", "program": "/usr/bin/editor",
                "args": ["--new-window", "/home/ada/a b.txt"]
            }))
        );
        // In the terminal, when it wants one.
        assert_eq!(
            exec_command("top", &[], Some("kitty")),
            Some(serde_json::json!({"command": "exec", "program": "kitty", "args": ["-e", "top"]}))
        );
        // Nothing to run, or a terminal wanted and none known: no command.
        assert_eq!(exec_command("", &args, None), None);
        assert_eq!(exec_command("top", &[], Some("")), None);
        // What comes out is a command the window manager reads.
        let command = exec_command("top", &[], Some("kitty")).unwrap();
        let read: gevrek_model::command::DispatchCommand = serde_json::from_value(command).unwrap();
        assert!(matches!(read, gevrek_model::command::DispatchCommand::Exec { program, .. } if program == "kitty"));
    }

    /// An item opened on Linux is started through the window manager: the
    /// backend's launcher gives the command and makes no process (there
    /// is no such program; made a child, starting it would fail).
    #[cfg(not(windows))]
    #[test]
    fn what_the_backend_opens_is_asked_of_the_window_manager_and_is_no_child() {
        use std::cell::RefCell;
        use std::rc::Rc;
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let told = Rc::clone(&asked);
        let starter = Starter {
            dispatch: Box::new(move |command| told.borrow_mut().push(command)),
            terminal: "kitty".to_string(),
        };
        let mut start = launcher(starter);
        let launch = |program: &str, terminal: bool| gevrek_system::zones::Launch {
            name: "A program".to_string(),
            program: program.to_string(),
            args: vec!["/home/ada/Desktop/a.txt".to_string()],
            directory: None,
            terminal,
            bus_name: None,
            addresses: Vec::new(),
        };
        start(&launch("/no/such/program-of-gevreks-test", false)).expect("asked, not started here");
        start(&launch("/no/such/program-of-gevreks-test", true)).expect("asked, in the terminal");
        let commands: Vec<serde_json::Value> =
            asked.borrow().iter().map(|text| serde_json::from_str(text).unwrap()).collect();
        assert_eq!(
            commands,
            [
                serde_json::json!({"command": "exec", "program": "/no/such/program-of-gevreks-test",
                    "args": ["/home/ada/Desktop/a.txt"]}),
                serde_json::json!({"command": "exec", "program": "kitty",
                    "args": ["-e", "/no/such/program-of-gevreks-test", "/home/ada/Desktop/a.txt"]}),
            ]
        );
        // Only over the bus: refused as what this cannot do, and nothing asked.
        let refused = start(&launch("", false)).expect_err("no program");
        assert_eq!(refused.kind(), std::io::ErrorKind::Unsupported);
        assert_eq!(asked.borrow().len(), 2);
        // And the backend is given this launcher, not left with its own.
        let source = include_str!("backend.rs");
        assert!(source.contains("backend.set_launcher(launcher(starter));"));
    }
}
