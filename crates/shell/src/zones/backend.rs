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
    let mut backend = SystemBackend::new(wake);
    // What the backend asks of the window manager goes the same way as
    // what it starts: a program that runs is brought forward by a
    // command to it (`focus_window`), since the windows are its own.
    let Starter { dispatch, terminal } = starter;
    let dispatch: std::rc::Rc<dyn Fn(String)> = std::rc::Rc::from(dispatch);
    let ask = std::rc::Rc::clone(&dispatch);
    backend.set_commander(move |command: serde_json::Value| ask(command.to_string()));
    #[cfg(not(windows))]
    backend.set_launcher(launcher(Starter { dispatch: Box::new(move |command| dispatch(command)), terminal }));
    #[cfg(windows)]
    let _ = (dispatch, terminal);
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
        // The room the state shows kept at a display's sides includes
        // the room kept for the zones themselves: the core is told what
        // the panels leave, and no more.
        let (displays, reserved) = displays_of_state(&panels_state(state));
        self.backend.set_displays(displays, reserved);
        // The programs that have a window, for the zones that show them.
        self.backend.set_wm_state(state);
    }

    fn pinned(&self) -> Vec<String> {
        self.backend.pinned_launchers()
    }

    fn pin(&mut self, program: &str, pin: bool) -> Result<(), BackendError> {
        self.backend.pin_launcher(program, pin)
    }

    fn spot_of(&mut self, display: &DisplayId) -> Option<Spot> {
        let output = self.outputs.iter().find(|(known, _)| known == display).map(|(_, output)| output.clone())?;
        // (Windows places a surface by the display's own name, not by
        // the path the window manager's state has for it: asked for by
        // the path, no surface was opened at all.)
        #[cfg(windows)]
        let output = self.backend.name_of(display).unwrap_or(output);
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
    fn has_system_icons(&self) -> bool {
        true
    }

    #[cfg(windows)]
    fn online_only(&mut self, item: &ItemId) -> bool {
        matches!(self.backend.cloud_state(item), Some(gevrek_system::zones::cloud::CloudState::OnlineOnly))
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
                    // The one it is opened with anyway, and the way to
                    // all the others: a menu of their own.
                    if let Some(usual) = programs.first() {
                        lines.push(DeskAction { id: format!("with:{}", usual.id), label: format!("Open with {}", usual.name) });
                    }
                    if programs.len() > 1 {
                        lines.push(line(super::CHOOSE, "Open with another program\u{2026}"));
                    }
                }
                MenuAction::AllowLaunching => lines.push(line("allow", "Allow launching")),
                MenuAction::Rename => lines.push(line(super::RENAME, "Rename\u{2026}")),
                MenuAction::MoveToTrash => lines.push(line(super::TRASH, "Move to the bin")),
                MenuAction::NewFolder { parent } => {
                    lines.push(DeskAction { id: format!("{}{parent}", super::NEW_FOLDER), label: "New folder\u{2026}".to_string() })
                }
                MenuAction::Properties => lines.push(line(super::PROPERTIES, "Properties")),
            }
        }
        Some(lines)
    }

    #[cfg(not(windows))]
    fn programs(&mut self, items: &[ItemId]) -> Vec<DeskAction> {
        use gevrek_system::zones::MenuAction;
        let menu = self.backend.menu_for(items, gevrek_model::geometry::Point::default());
        menu.actions
            .into_iter()
            .find_map(|action| match action {
                MenuAction::OpenWith { programs } => Some(programs),
                _ => None,
            })
            .unwrap_or_default()
            .into_iter()
            .map(|program| DeskAction { id: format!("with:{}", program.id), label: program.name })
            .collect()
    }

    #[cfg(not(windows))]
    fn properties(&mut self, items: &[ItemId]) -> Vec<(String, String)> {
        let menu = self.backend.menu_for(items, gevrek_model::geometry::Point::default());
        menu.properties.iter().flat_map(properties_of).collect()
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

/// The window manager's state as it would be if no room were kept for
/// the zones themselves: each display's room for windows made larger
/// again by the zones' own room, so that what is kept at its sides is
/// what the panels keep, and no more.
///
/// The work area in the state is what is left for the windows, so the
/// zones' own room is taken off it too (by the window manager on
/// Windows, by the strip surfaces' exclusive zones on Linux). Told to
/// the core as it is, a zone that keeps room would find its own room
/// taken, be moved out of it, keep other room, and so on. The core is to
/// know only what panels leave (`core-milestones-3-5.md`, "Tiling
/// awareness").
///
/// Which part is the zones' is read from the state itself
/// (`monitors[].zones_room`: `left`, `top`, `right`, `bottom`): the room
/// that was in force when that state was made. Not from what the zones
/// last asked for: the state follows a request by a round trip, and in
/// between the two differ. It is in the units of the state's own work
/// area (pixels on Windows, logical units on Linux), which is why it is
/// given back to the work area here, before anything is reckoned into
/// the zones' units: the panels' room then comes out exactly, at every
/// scale. A state that names no such room is the state as it is, and
/// nothing is copied.
pub fn panels_state(state: &serde_json::Value) -> std::borrow::Cow<'_, serde_json::Value> {
    const SIDES: [&str; 4] = ["left", "top", "right", "bottom"];
    let own = |monitor: &serde_json::Value, side: &str| monitor["zones_room"][side].as_f64().unwrap_or(0.0).max(0.0);
    let any = |monitor: &serde_json::Value| SIDES.iter().any(|side| own(monitor, side) > 0.0);
    if !state["monitors"].as_array().is_some_and(|monitors| monitors.iter().any(any)) {
        return std::borrow::Cow::Borrowed(state);
    }
    let mut without = state.clone();
    for monitor in without["monitors"].as_array_mut().into_iter().flatten() {
        let [left, top, right, bottom] = SIDES.map(|side| own(monitor, side));
        let number = |value: &serde_json::Value| value.as_f64().unwrap_or(0.0);
        let (wide, high) = (number(&monitor["work_size"]["width"]), number(&monitor["work_size"]["height"]));
        if wide > 0.0 && high > 0.0 {
            // (Never beyond the display itself: more of the zones' room
            // named than is kept there leaves none for panels, not less.)
            let (x, y) = (number(&monitor["position"]["x"]), number(&monitor["position"]["y"]));
            let (full_wide, full_high) = (number(&monitor["size"]["width"]), number(&monitor["size"]["height"]));
            let work_x = (number(&monitor["work_position"]["x"]) - left).max(x);
            let work_y = (number(&monitor["work_position"]["y"]) - top).max(y);
            let work_right = (number(&monitor["work_position"]["x"]) + wide + right).min(x + full_wide);
            let work_bottom = (number(&monitor["work_position"]["y"]) + high + bottom).min(y + full_high);
            monitor["work_position"] = serde_json::json!({ "x": work_x, "y": work_y });
            monitor["work_size"] = serde_json::json!({ "width": work_right - work_x, "height": work_bottom - work_y });
        }
        // (A state that names the sides themselves and no work area.)
        for (key, taken) in [("reserved_left", left), ("reserved_top", top), ("reserved_right", right), ("reserved_bottom", bottom)] {
            if let Some(kept) = monitor[key].as_f64() {
                monitor[key] = serde_json::json!((kept - taken).max(0.0));
            }
        }
    }
    std::borrow::Cow::Owned(without)
}

/// What is shown of one item's properties, as lines of a name and a value.
#[cfg(not(windows))]
fn properties_of(properties: &gevrek_system::zones::Properties) -> Vec<(String, String)> {
    let item = &properties.item;
    let line = |name: &str, value: String| (name.to_string(), value);
    let mut lines = vec![line("Name", item.name.clone())];
    if let Some(path) = &item.path {
        lines.push(line("Where", path.clone()));
    }
    if let Some(mime) = &item.facts.mime {
        lines.push(line("Type", mime.clone()));
    }
    if let Some(size) = item.facts.size {
        lines.push(line("Size", size_in_words(size)));
    }
    if let Some(target) = &item.facts.shortcut_target {
        lines.push(line("Leads to", target.clone()));
    }
    for (name, time) in [("Made", item.facts.created), ("Changed", item.facts.modified), ("Opened", item.facts.accessed)] {
        if let Some(time) = time {
            lines.push(line(name, date_in_words(time)));
        }
    }
    lines.push(line("Rights", rights_in_words(properties.mode)));
    if let Some(program) = &properties.opens_with {
        lines.push(line("Opens with", program.clone()));
    }
    if let Some(trusted) = properties.trusted {
        lines.push(line("May be started", if trusted { "yes" } else { "no (allow launching first)" }.to_string()));
    }
    lines
}

/// A size as a person reads it.
#[cfg_attr(windows, allow(dead_code))]
pub fn size_in_words(bytes: u64) -> String {
    const STEPS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut step = 0;
    while size >= 1024.0 && step + 1 < STEPS.len() {
        size /= 1024.0;
        step += 1;
    }
    format!("{size:.1} {} ({bytes} bytes)", STEPS[step])
}

/// A day and a time of day (in universal time) from seconds since 1970.
#[cfg_attr(windows, allow(dead_code))]
pub fn date_in_words(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let of_day = seconds % 86_400;
    // (The civil calendar from a count of days, after Howard Hinnant.)
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_shifted + 2) / 5 + 1;
    let month = if month_shifted < 10 { month_shifted + 3 } else { month_shifted - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", of_day / 3600, of_day % 3600 / 60)
}

/// A file's rights as letters and as the number they are set by.
#[cfg_attr(windows, allow(dead_code))]
pub fn rights_in_words(mode: u32) -> String {
    let letters: String = (0..9)
        .map(|bit| if mode & (0o400 >> bit) != 0 { ['r', 'w', 'x'][bit % 3] } else { '-' })
        .collect();
    format!("{letters} ({:03o})", mode & 0o777)
}

// Everything the core asks goes to the backend as it is.
impl ZonesBackend for System {
    fn begin_listing(&mut self, location: &gevrek_zones::backend::Location) -> bool {
        self.backend.begin_listing(location)
    }
    fn list_source(&mut self, source: &gevrek_zones::config::SmartSource) -> Result<Vec<gevrek_zones::Item>, BackendError> {
        self.backend.list_source(source)
    }
    fn watch_source(&mut self, source: &gevrek_zones::config::SmartSource) -> Result<gevrek_zones::backend::WatchId, BackendError> {
        self.backend.watch_source(source)
    }
    fn begin_operation(&mut self, request: u64, operation: &gevrek_zones::backend::FileOperation) -> bool {
        self.backend.begin_operation(request, operation)
    }
    fn cancel_operation(&mut self, request: u64) {
        self.backend.cancel_operation(request)
    }
    fn screen_shared(&mut self) -> Option<bool> {
        self.backend.screen_shared()
    }
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

    /// A length of the zones' own units in the units the window
    /// manager's state is written in: pixels of the display on Windows,
    /// the same logical units on Linux.
    fn in_the_states_units(units: i32, scale: f64) -> i32 {
        if cfg!(windows) {
            (f64::from(units) * scale).round() as i32
        } else {
            units
        }
    }

    /// A window manager as far as this needs one: a display of 1920 by
    /// 1080 units at a scale, with a panel (or none), which keeps the
    /// zones' room it was last told and says so in its state. Depths are
    /// given in the zones' units and written in the state's.
    fn state_of(
        panel: (&str, i32),
        zones_room: &std::collections::BTreeMap<&'static str, i32>,
        echo: bool,
        scale: f64,
    ) -> serde_json::Value {
        let units = |length: i32| in_the_states_units(length, scale);
        let own = |side: &str| units(zones_room.get(side).copied().unwrap_or(0));
        let kept = |side: &str| own(side) + if panel.0 == side { units(panel.1) } else { 0 };
        let (left, top, right, bottom) = (kept("left"), kept("top"), kept("right"), kept("bottom"));
        let (wide, high) = (units(1920), units(1080));
        let mut monitor = serde_json::json!({
            "name": "DP-1", "stable_id": "edid:DEL-A1", "is_primary": true, "scale": scale,
            "position": { "x": 0, "y": 0 }, "size": { "width": wide, "height": high },
            "work_position": { "x": left, "y": top },
            "work_size": { "width": wide - left - right, "height": high - top - bottom },
        });
        if echo {
            monitor["zones_room"] = serde_json::json!({
                "left": own("left"), "top": own("top"), "right": own("right"), "bottom": own("bottom"),
            });
        }
        serde_json::json!({ "monitors": [monitor] })
    }

    /// A zone that keeps windows off, at a side of the display, through
    /// ten states of the window manager, each made `lag` states after
    /// the zones' room was asked for (the room reaches the window
    /// manager with the zones' snapshot on Windows, and as a strip
    /// surface on Linux: a round trip either way). Gives where the zone
    /// was and the room it asked for, after each state.
    fn through_ten_states(
        panel: (&str, i32),
        zone: gevrek_zones::Rect,
        lag: usize,
        scale: f64,
        minded: bool,
    ) -> Vec<(gevrek_zones::Rect, Vec<(String, i32)>)> {
        use gevrek_zones::config::{ZoneKind, ZonesConfig};
        use gevrek_zones::core::{Command, ZonesCore};
        use gevrek_zones::host::Host;
        use gevrek_zones::testing::FakeBackend;
        const NOW: u64 = 1_800_000_000;
        let mut core = ZonesCore::new(ZonesConfig { enabled: true, ..ZonesConfig::default() }, Default::default());
        let mut backend = FakeBackend::new();
        let mut host = Host::new();
        let none = std::collections::BTreeMap::new();
        let tell = |core: &mut ZonesCore, backend: &mut FakeBackend, host: &mut Host, state: &serde_json::Value| {
            let (displays, reserved) = if minded { displays_of_state(&panels_state(state)) } else { displays_of_state(state) };
            backend.set_displays(displays);
            backend.set_reserved(reserved);
            host.pump(core, backend, NOW);
        };
        tell(&mut core, &mut backend, &mut host, &state_of(panel, &none, true, scale));
        host.start(&mut core, &mut backend, NOW);
        tell(&mut core, &mut backend, &mut host, &state_of(panel, &none, true, scale));
        host.command(&mut core, &mut backend, Command::CreateZone { title: "Dock".to_string(), kind: ZoneKind::Manual, rect: Some(zone) }, NOW);
        let id = core.config().zones[0].id.clone();
        let asked = Command::ModifyZone { zone: id.clone(), changes: serde_json::json!({ "reserves_space": true }) };
        host.command(&mut core, &mut backend, asked, NOW);
        // What the zones ask for, as the window manager is told it.
        let room_asked = |core: &ZonesCore| -> std::collections::BTreeMap<&'static str, i32> {
            let mut room = std::collections::BTreeMap::new();
            for region in core.reserved_regions() {
                let side = match region.edge {
                    gevrek_zones::geometry::Edge::Left => "left",
                    gevrek_zones::geometry::Edge::Top => "top",
                    gevrek_zones::geometry::Edge::Right => "right",
                    gevrek_zones::geometry::Edge::Bottom => "bottom",
                };
                let depth: &mut i32 = room.entry(side).or_default();
                *depth = (*depth).max(region.thickness);
            }
            room
        };
        // (The requests on their way to the window manager, the oldest first.)
        let mut on_the_way = vec![none.clone(); lag];
        let mut seen = Vec::new();
        for _ in 0..10 {
            on_the_way.push(room_asked(&core));
            let in_force = on_the_way.remove(0);
            tell(&mut core, &mut backend, &mut host, &state_of(panel, &in_force, minded, scale));
            let asks: Vec<(String, i32)> = room_asked(&core).into_iter().map(|(side, depth)| (side.to_string(), depth)).collect();
            seen.push((core.zone_rect(&id).expect("the zone has a place"), asks));
        }
        seen
    }

    #[test]
    fn a_zone_that_keeps_room_stays_where_it_is_whatever_the_window_manager_makes_of_its_room() {
        use gevrek_zones::Rect;
        // (the panel, the zone, the room it keeps)
        let cases = [
            ("at the top, under a panel", ("top", 40), Rect::new(300, 40, 600, 120), ("top", 120)),
            ("at the top, no panel", ("none", 0), Rect::new(300, 0, 600, 120), ("top", 120)),
            ("at the left, a panel at the top", ("top", 40), Rect::new(0, 200, 280, 500), ("left", 280)),
            ("at the bottom, above a panel", ("bottom", 48), Rect::new(400, 912, 800, 120), ("bottom", 120)),
            ("at the right, no panel", ("none", 0), Rect::new(1640, 300, 280, 400), ("right", 280)),
        ];
        // The room is in force at once, a state later (the usual: one
        // round trip to the window manager on Windows, one configure of
        // the strip on Linux), or two; on displays of several scales
        // (the state is in pixels on Windows, and what is the zones' in
        // it must come out of it in pixels).
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for lag in [0, 1, 2] {
                for (what, panel, zone, (side, depth)) in cases {
                    let seen = through_ten_states(panel, zone, lag, scale, true);
                    for (round, (rect, asks)) in seen.iter().enumerate() {
                        assert_eq!(*rect, zone, "{what}, scale {scale}, lag {lag}, state {round}: the zone moved");
                        assert_eq!(
                            asks,
                            &[(side.to_string(), depth)],
                            "{what}, scale {scale}, lag {lag}, state {round}: it asks for other room"
                        );
                    }
                }
            }
        }
        // What this guards against: told the state's room as it is, the
        // zone finds its own room taken and does not stay where it was.
        let unminded = through_ten_states(("top", 40), Rect::new(300, 40, 600, 120), 1, 1.0, false);
        assert!(
            unminded.iter().any(|(rect, _)| *rect != Rect::new(300, 40, 600, 120)),
            "the window manager's state did not disturb the zone: this test would hold nothing"
        );
    }

    #[test]
    fn without_the_zones_own_room_the_state_keeps_what_the_panels_keep() {
        let room = |side: &'static str, depth: i32| std::collections::BTreeMap::from([(side, depth)]);
        let none = std::collections::BTreeMap::new();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for panel in [("top", 40), ("none", 0), ("bottom", 48), ("left", 64)] {
                // What the panels alone keep: the state with no room for zones.
                let panels_alone = displays_of_state(&state_of(panel, &none, true, scale));
                for own in [room("top", 120), room("left", 280), room("bottom", 96), room("right", 333)] {
                    let state = state_of(panel, &own, true, scale);
                    assert_eq!(
                        displays_of_state(&panels_state(&state)),
                        panels_alone,
                        "a panel at {panel:?}, the zones' room {own:?}, scale {scale}"
                    );
                    // The state itself is not what the panels keep: the
                    // room of the zones is in it.
                    assert_ne!(displays_of_state(&state).1, panels_alone.1);
                }
            }
        }
        // A state that names no room of the zones' is taken as it is,
        // and is not copied to find that out.
        let plain = state_of(("top", 40), &room("top", 120), false, 1.0);
        assert!(matches!(panels_state(&plain), std::borrow::Cow::Borrowed(_)));
        let nothing_taken = state_of(("top", 40), &none, true, 1.0);
        assert!(matches!(panels_state(&nothing_taken), std::borrow::Cow::Borrowed(_)));
        // More named than is kept there: none is left for panels, and the
        // room for windows is never larger than the display.
        let mut odd = state_of(("top", 40), &room("top", 120), true, 1.0);
        odd["monitors"][0]["zones_room"]["top"] = serde_json::json!(5000);
        let (displays, kept) = displays_of_state(&panels_state(&odd));
        assert_eq!((displays.len(), kept), (1, Vec::new()));
        // A state that names the sides and no work area.
        let sides = serde_json::json!({ "monitors": [{
            "name": "DP-1", "stable_id": "a", "scale": 1.0,
            "position": { "x": 0, "y": 0 }, "size": { "width": 1920, "height": 1080 },
            "reserved_top": 160, "zones_room": { "left": 0, "top": 120, "right": 0, "bottom": 0 },
        }]});
        let (_, kept) = displays_of_state(&panels_state(&sides));
        assert_eq!(kept.iter().map(|area| area.thickness).collect::<Vec<_>>(), [40]);
    }

    #[test]
    fn what_is_shown_of_a_file_is_said_in_words() {
        assert_eq!(size_in_words(512), "512 bytes");
        assert_eq!(size_in_words(1536), "1.5 KiB (1536 bytes)");
        assert_eq!(size_in_words(5 * 1024 * 1024), "5.0 MiB (5242880 bytes)");
        assert_eq!(date_in_words(0), "1970-01-01 00:00 UTC");
        assert_eq!(date_in_words(1_800_000_000), "2027-01-15 08:00 UTC");
        assert_eq!(date_in_words(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(rights_in_words(0o644), "rw-r--r-- (644)");
        assert_eq!(rights_in_words(0o100755), "rwxr-xr-x (755)");
    }

    /// Every question the core has of a backend reaches the system's: a
    /// method of the trait that is not passed on here would answer with
    /// the trait's "cannot", and a portal would wait, a zone that fills
    /// itself stay empty, without any sign of why.
    #[test]
    fn nothing_the_core_asks_is_left_to_the_traits_own_answer() {
        let source = include_str!("backend.rs");
        let passed_on = source.split("impl ZonesBackend for System").nth(1).unwrap().split("#[cfg(test)]").next().unwrap();
        let of_the_trait = include_str!("../../../zones/src/backend.rs");
        let of_the_trait = of_the_trait.split("pub trait ZonesBackend").nth(1).unwrap().split("\n}\n").next().unwrap();
        let names: Vec<&str> = of_the_trait
            .lines()
            .filter_map(|line| line.trim().strip_prefix("fn "))
            .filter_map(|rest| rest.split('(').next())
            .collect();
        assert!(names.len() >= 25, "{names:?}");
        for name in names {
            assert!(passed_on.contains(&format!("fn {name}(")), "{name} is not passed on to the system's backend");
        }
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
