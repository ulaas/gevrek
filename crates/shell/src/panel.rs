//! The panel: the bar at an edge of each display.
//!
//! What it shows comes from the window manager: the workspaces of the
//! display (a click shows one), the layout, the title of the focused
//! window, and the time. How it looks is `desktop.shell.panel` of the
//! configuration, as it was for the bar this replaces.
//!
//! Everything here is the same on every system: what a display's part of
//! the window manager's state means for its panel, which of the window
//! manager's events change that, and the drawing of the modules. What
//! differs is who keeps the panel's room free, and that is with the
//! program's half for each system: on Linux the surface asks the
//! compositor for its room; on Windows the window manager keeps the room
//! and says where it is, and the surface is put there.

use gevrek_model::settings::{
    PanelAlign, PanelEdge, PanelFit, PanelModule, PanelProfile, PanelStyle,
};
use gevrek_ui::brand_icons::BrandIcon;
use gevrek_ui::theme::{self, icons};
use serde_json::Value;

/// One workspace as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    /// What the workspace is known by to the window manager (what a
    /// command names it by). It stays when the workspace is renamed.
    pub name: String,
    /// What stands on its button: its title, or its name where it has
    /// none (its number where that is only a placeholder).
    pub label: String,
    /// The configuration has it for this display (one it has not is
    /// there for the moment, and goes when it is left empty).
    pub configured: bool,
    /// It is the one the display shows.
    pub shown: bool,
    pub windows: usize,
}

/// One window of the shown workspace, as the taskbar shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub class: String,
    /// What its picture is looked for by: the one its program gave the
    /// window itself (a name, or a file's path), else the program's own
    /// (see [`task_icon`]). Empty: no picture, the words alone.
    pub icon: String,
    /// It is the one its workspace gives the keyboard to.
    pub focused: bool,
    /// Its workspace is the one the display shows.
    pub shown: bool,
}

/// What a window's picture is looked for by. The picture its program
/// gave the window itself comes first. Else the program's own: on Linux
/// by what the program calls itself (the name its icon has in the icon
/// theme), on Windows by its file (whose icon the system draws: the
/// window manager names the whole path there as the window's own
/// picture, and the name alone is only what is left without it).
pub fn task_icon(own: &str, app_id: &str, class: &str, exe: &str) -> String {
    if !own.is_empty() {
        return own.to_string();
    }
    if cfg!(target_os = "windows") {
        return exe.to_string();
    }
    let _ = exe;
    if app_id.is_empty() { class } else { app_id }.to_string()
}

/// How a window's picture stands with the taskbar.
#[derive(Clone)]
enum TaskPicture {
    /// It is being looked for (off the drawing thread).
    Looking,
    None,
    Found(std::sync::Arc<gevrek_system::icons::Picture>),
}

/// The pictures of windows, by what they were looked for by: found once
/// for the whole program, on a thread of their own.
fn task_pictures() -> &'static std::sync::Mutex<std::collections::HashMap<String, TaskPicture>> {
    static PICTURES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, TaskPicture>>,
    > = std::sync::OnceLock::new();
    PICTURES.get_or_init(Default::default)
}

/// The side a window's picture is looked for at, in pixels: enough for a
/// panel on a display scaled to twice its size.
const TASK_PICTURE: u32 = 40;

/// The picture of a window for its button in the taskbar, when there is
/// one and it was found; while it is looked for the button has its words
/// alone, and is drawn again a moment later.
fn task_picture(context: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
    if name.is_empty() {
        return None;
    }
    let state = {
        let mut known = task_pictures()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match known.get(name) {
            Some(state) => state.clone(),
            None => {
                known.insert(name.to_string(), TaskPicture::Looking);
                let wanted = name.to_string();
                let spawned = std::thread::Builder::new()
                    .name("gevrek-task-icon".to_string())
                    .spawn(move || {
                        let found = look_for_picture(&wanted);
                        // (Said when asked to: the desktop check reads it.)
                        log::debug!(
                            "the taskbar's picture for {wanted:?} was found={}",
                            found.is_some()
                        );
                        task_pictures()
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .insert(
                                wanted,
                                found.map_or(TaskPicture::None, |picture| {
                                    TaskPicture::Found(std::sync::Arc::new(picture))
                                }),
                            );
                    });
                if spawned.is_err() {
                    known.insert(name.to_string(), TaskPicture::None);
                }
                TaskPicture::Looking
            }
        }
    };
    match state {
        TaskPicture::None => None,
        TaskPicture::Looking => {
            context.request_repaint_after(std::time::Duration::from_millis(150));
            None
        }
        TaskPicture::Found(picture) => {
            let id = egui::Id::unique(("task-picture", name));
            let kept: Option<egui::TextureHandle> = context.data(|data| data.get_temp(id));
            Some(kept.unwrap_or_else(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [picture.width as usize, picture.height as usize],
                    &picture.pixels,
                );
                let texture = context.load_texture(
                    format!("task-{name}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                context.data_mut(|data| data.insert_temp(id, texture.clone()));
                texture
            }))
        }
    }
}

/// Look for a picture by its name, or read it from the file that is named.
fn look_for_picture(name: &str) -> Option<gevrek_system::icons::Picture> {
    #[cfg(target_os = "linux")]
    {
        if name.starts_with('/') {
            return gevrek_system::icons::load(std::path::Path::new(name), TASK_PICTURE);
        }
        // (By the program's name, or by its application's entry.)
        gevrek_system::icons::picture_of_app(name, TASK_PICTURE)
    }
    #[cfg(not(target_os = "linux"))]
    gevrek_system::icons::picture(name, TASK_PICTURE)
}

/// What stands on a window's button in the taskbar: its title, cut where
/// it is long; its program's name where it has no title.
pub fn task_label(task: &Task) -> String {
    const LETTERS: usize = 24;
    let words = if task.title.trim().is_empty() {
        task.class.trim()
    } else {
        task.title.trim()
    };
    if words.chars().count() <= LETTERS {
        words.to_string()
    } else {
        let start: String = words.chars().take(LETTERS - 1).collect();
        format!("{}…", start.trim_end())
    }
}

/// What the panel of one display shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct View {
    pub tabs: Vec<Tab>,
    pub layout: String,
    /// The shown workspace does not tile its windows.
    pub floating: bool,
    /// The title of the focused window on the shown workspace.
    pub title: String,
    /// The display has the keyboard's window.
    pub focused: bool,
    /// The display, as the window manager and the configuration name it.
    pub place: gevrek_model::settings::DisplayRef,
    /// The windows of the shown workspace, in the order the state has
    /// them (the taskbar's buttons).
    pub tasks: Vec<Task>,
    /// The windows of every workspace of the display, workspace by
    /// workspace (the taskbar's buttons where it is set to show all).
    pub all_tasks: Vec<Task>,
}

/// Something the window manager has to tell its user.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// What the window manager knows it by: the same for as long as the
    /// notice holds, so that it is told once.
    pub id: String,
    pub title: String,
    pub text: String,
}

/// The name a workspace is shown by: its own, unless that is only a
/// placeholder (`Workspace 4`, or none at all), then its number.
pub fn tab_label(name: &str, index: usize) -> String {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("workspace") {
        (index + 1).to_string()
    } else if let Some(number) = name
        .strip_prefix("Workspace ")
        .filter(|rest| !rest.is_empty() && rest.chars().all(|letter| letter.is_ascii_digit()))
    {
        number.to_string()
    } else {
        name.to_string()
    }
}

/// The name a display has in the window manager's state: its connector
/// where it has one (Linux), else its name (`Monitor ACM2702 (2)`).
pub fn name_of(monitor: &Value) -> Option<&str> {
    monitor["connector"]
        .as_str()
        .filter(|name| !name.is_empty())
        .or_else(|| monitor["name"].as_str())
        .filter(|name| !name.is_empty())
}

/// Read a display's part of the window manager's state. `None` when the
/// display is not in it.
pub fn view(state: &Value, output: &str) -> Option<View> {
    let (index, monitor) = state["monitors"].as_array()?.iter().enumerate().find(
        |(_, monitor)| {
            monitor["connector"].as_str() == Some(output)
                || monitor["name"].as_str() == Some(output)
        },
    )?;
    Some(view_at(monitor, Some(index)))
}

/// The keyboard layout in use, as the state tells it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyboardLayout {
    pub short: String,
    pub name: String,
    pub index: u32,
    pub count: u32,
}

/// The keyboard layout the state names, if it names one.
pub fn keyboard_layout(state: &Value) -> Option<KeyboardLayout> {
    let told = state.get("keyboard_layout")?;
    let short = told["short"].as_str()?.to_string();
    Some(KeyboardLayout {
        name: told["name"].as_str().unwrap_or(&short).to_string(),
        short,
        index: told["index"].as_u64().unwrap_or(0) as u32,
        count: told["count"].as_u64().unwrap_or(1) as u32,
    })
}

/// The command that goes to the next keyboard layout, or the one before.
pub fn switch_keyboard_layout(next: bool) -> String {
    serde_json::json!({"command": "keyboard_layout", "target": {"kind": if next { "next" } else { "previous" }}})
        .to_string()
}

/// The windows of one workspace of the state, as the taskbar's buttons.
fn tasks_of(workspace: &Value, shown: bool) -> Vec<Task> {
    workspace["detailed_windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|window| !window["is_border_window"].as_bool().unwrap_or(false))
        .filter_map(|window| {
            let said = |key: &str| window[key].as_str().unwrap_or_default();
            Some(Task {
                id: window["window_id"].as_u64()?,
                title: window["title"].as_str().unwrap_or_default().to_string(),
                class: window["class"].as_str().unwrap_or_default().to_string(),
                icon: task_icon(said("icon"), said("app_id"), said("class"), said("exe")),
                focused: window["is_focused"].as_bool().unwrap_or(false),
                shown,
            })
        })
        .collect()
}

/// What comes of a new name typed for a workspace of a display.
#[derive(Debug, Clone)]
pub enum Renaming {
    /// It is shown by that already.
    Nothing,
    /// The configuration with the new title in it, to be given to the
    /// window manager: it shows the workspace by it at once, and has it
    /// again when it is started anew.
    Apply(Box<gevrek_model::Config>),
    /// It cannot be: why, in words for the person.
    Refused(String),
}

/// Give the workspace `name` of a display the title `new`, in a copy of
/// the configuration that runs. Only what is shown of the workspace
/// changes, and only on this display: the keys and rules that name it
/// mean it as before, and the workspaces of that name on the other
/// displays keep what they are shown by. A workspace the configuration
/// does not have for the display (one opened for the moment) is written
/// down for it with the title, so that a name given to it lasts.
pub fn renaming(
    config: Option<&gevrek_model::Config>,
    place: &gevrek_model::settings::DisplayRef,
    name: &str,
    new: &str,
) -> Renaming {
    use gevrek_model::settings::Titled;
    let Some(config) = config else {
        return Renaming::Refused(
            "The configuration is not known yet. Try again in a moment.".to_string(),
        );
    };
    if place.name().is_empty() {
        return Renaming::Refused("This display has no name to write the title down for.".to_string());
    }
    let mut changed = config.clone();
    let taken = || {
        Renaming::Refused(format!(
            "Another workspace of this display is called \u{201c}{}\u{201d} already.",
            new.trim()
        ))
    };
    match changed.desktop.title_workspace_on(place, name, new) {
        Titled::Changed => Renaming::Apply(Box::new(changed)),
        Titled::Same => Renaming::Nothing,
        Titled::Taken => taken(),
        Titled::Unknown => {
            let shown = |profile: &&gevrek_model::settings::WorkspaceProfile| {
                profile.label().eq_ignore_ascii_case(new.trim())
                    || profile.name.trim().eq_ignore_ascii_case(new.trim())
            };
            if changed.desktop.workspaces_on(place).iter().any(shown) {
                return taken();
            }
            if changed.desktop.keep_workspace_on(place, name, new) {
                Renaming::Apply(Box::new(changed))
            } else {
                Renaming::Refused("This workspace could not be written down.".to_string())
            }
        }
    }
}

/// How large the box is that asks for a workspace's new name.
pub const RENAME_SIZE: (u32, u32) = (340, 132);

/// The box that asks for a workspace's new name. `Some(true)`: Enter was
/// pressed on a name; `Some(false)`: it is to be put away.
pub fn rename_box(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    old: &str,
    name: &mut String,
    problem: &str,
) -> Option<bool> {
    let mut answer = None;
    egui::Frame::new()
        .fill(palette.surface)
        .corner_radius(12)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("Rename workspace \u{201c}{old}\u{201d}"))
                    .size(14.0)
                    .color(palette.text),
            );
            ui.add_space(8.0);
            let field = ui.add(
                egui::TextEdit::singleline(name)
                    .desired_width(f32::INFINITY)
                    .hint_text("Its new name"),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                answer = Some(!name.trim().is_empty());
            }
            ui.add_space(6.0);
            let words = if problem.is_empty() {
                "Enter keeps it, Escape leaves it as it is."
            } else {
                problem
            };
            ui.label(egui::RichText::new(words).size(12.0).color(palette.muted));
        });
    answer
}

/// What the panel of this display of the state shows.
pub fn view_of(monitor: &Value) -> View {
    view_at(monitor, None)
}

/// A display of the state, as a configuration can name it: by the names
/// the window manager knows it by, its place among the displays, and
/// whether it is the primary one.
pub fn place_of(monitor: &Value, index: Option<usize>) -> gevrek_model::settings::DisplayRef {
    gevrek_model::settings::DisplayRef::of_state(monitor, index)
}

/// What the panel of this display of the state shows; `index` is the
/// display's place among the state's displays.
pub fn view_at(monitor: &Value, index: Option<usize>) -> View {
    let shown_id = &monitor["workspace"]["id"];
    let mut found = View {
        focused: monitor["is_focused"].as_bool().unwrap_or(false),
        place: place_of(monitor, index),
        ..Default::default()
    };
    let workspaces = monitor["workspaces"].as_array().into_iter().flatten();
    for (index, workspace) in workspaces.enumerate() {
        let shown = &workspace["id"] == shown_id;
        let listed = workspace["detailed_windows"].as_array().map_or(0, Vec::len);
        // Where only the number is told (a workspace that is not shown).
        let counted = workspace["total_windows"].as_u64().unwrap_or(0) as usize;
        let name = workspace["name"].as_str().unwrap_or("?").to_string();
        // (What is shown of it: its title. A window manager that tells
        // none tells the name.)
        let title = workspace["title"]
            .as_str()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(&name);
        found.tabs.push(Tab {
            label: tab_label(title, index),
            configured: workspace["configured"].as_bool().unwrap_or(false),
            name,
            shown,
            windows: listed.max(counted),
        });
        found.all_tasks.extend(tasks_of(workspace, shown));
        if shown {
            found.layout = workspace["layout_type"].as_str().unwrap_or("").to_string();
            found.floating = workspace["tiling_enabled"].as_bool() == Some(false);
            let focused_of = |list: &Value| {
                list.as_array()
                    .into_iter()
                    .flatten()
                    .find(|window| window["is_focused"].as_bool().unwrap_or(false))
                    .and_then(|window| window["title"].as_str())
                    .map(str::to_string)
            };
            found.tasks = tasks_of(workspace, true);
            found.title = focused_of(&workspace["detailed_windows"])
                .or_else(|| focused_of(&workspace["windows"]))
                .unwrap_or_default();
        }
    }
    found
}

/// What the window manager has to tell, from its state.
pub fn notices(state: &Value) -> Vec<Notice> {
    state["notices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|notice| Notice {
            id: notice["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .or(notice["title"].as_str())
                .unwrap_or("")
                .to_string(),
            title: notice["title"].as_str().unwrap_or("").to_string(),
            text: notice["text"].as_str().unwrap_or("").to_string(),
        })
        .collect()
}

/// The accent the desktop has now where it is not the configuration's
/// own (the wallpaper's colour), by the window manager's state.
pub fn accent(state: &Value) -> Option<(u8, u8, u8)> {
    let colour = u32::try_from(state["accent_rgb"].as_u64()?).ok()?;
    Some(((colour >> 16) as u8, (colour >> 8) as u8, colour as u8))
}

/// The notices that were not there the last time: each is told once when
/// it arises, however often the state is asked for while it holds. One
/// that went and comes back has arisen again. `told` is what was there
/// the last time, and is brought up to now.
pub fn arisen(notices: &[Notice], told: &mut std::collections::HashSet<String>) -> Vec<Notice> {
    let new = notices
        .iter()
        .filter(|notice| !told.contains(&notice.id))
        .cloned()
        .collect();
    *told = notices.iter().map(|notice| notice.id.clone()).collect();
    new
}

/// The set of keys that is in use, when it is not the usual one.
pub fn mode(state: &Value) -> &str {
    match state["active_binding_submap"].as_str() {
        Some(mode) if !mode.is_empty() && mode != "default" => mode,
        _ => "",
    }
}

/// A rectangle of the desktop, in the pixels of the screen: x, y, width,
/// height.
pub type Rect = (i32, i32, i32, i32);

/// Where the window manager keeps room for a panel: each display's name
/// and the rectangle of its panel. A display the window manager keeps no
/// room on (no panel in its configuration) is not among them.
///
/// This is how the window manager's names for the displays (the model's
/// name, `Monitor ACM2702 (2)`) and the system's (`DISPLAY1`) meet on
/// Windows: by the place. The panel of a display is put at the rectangle
/// the window manager gives for it, and is so on the display it means,
/// whatever either calls it.
pub fn places(state: &Value) -> Vec<(String, Rect)> {
    let whole = |value: &Value| value.as_f64().map(|number| number.round() as i32);
    displays(state)
        .into_iter()
        .filter_map(|(name, monitor)| {
            let (at, size) = (&monitor["bar_position"], &monitor["bar_size"]);
            let rect = (
                whole(&at["x"])?,
                whole(&at["y"])?,
                whole(&size["width"])?,
                whole(&size["height"])?,
            );
            (rect.2 > 0 && rect.3 > 0).then_some((name, rect))
        })
        .collect()
}

/// What the shell knows of a display to put a surface on it by itself:
/// the room windows have there, the panel's rectangle where there is one
/// (both in the pixels of the screen), and the pixels to a logical pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub work: Rect,
    pub bar: Option<Rect>,
    pub scale: f32,
    /// The display has the keyboard's window.
    pub focused: bool,
}

/// The rooms of the displays of the window manager's state. A display
/// the state gives no size for is not among them.
pub fn rooms(state: &Value) -> Vec<Room> {
    let whole = |value: &Value| value.as_f64().map(|number| number.round() as i32);
    let rect = |at: &Value, size: &Value| {
        let rect = (
            whole(&at["x"])?,
            whole(&at["y"])?,
            whole(&size["width"])?,
            whole(&size["height"])?,
        );
        (rect.2 > 0 && rect.3 > 0).then_some(rect)
    };
    state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|monitor| {
            // The room for windows where it is told, else all of the display.
            let work = rect(&monitor["work_position"], &monitor["work_size"])
                .or_else(|| rect(&monitor["position"], &monitor["size"]))?;
            Some(Room {
                work,
                bar: rect(&monitor["bar_position"], &monitor["bar_size"]),
                scale: monitor["scale"]
                    .as_f64()
                    .filter(|scale| scale.is_finite() && *scale >= 0.5)
                    .unwrap_or(1.0) as f32,
                focused: monitor["is_focused"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// The rectangle of a surface in the top right corner of a display's
/// room, `gap` from its edges, and under the panel where that is at the
/// top (the window manager keeps the panel's room, so nothing else knows
/// to keep clear of it). Never larger than the room that is left.
pub fn corner(room: &Room, size: (i32, i32), gap: i32) -> Rect {
    corner_at(
        room,
        size,
        gap,
        gevrek_model::settings::NotificationCorner::TopRight,
    )
}

/// The same in any corner of the room: under a panel at the top, over one
/// at the bottom.
pub fn corner_at(
    room: &Room,
    size: (i32, i32),
    gap: i32,
    corner: gevrek_model::settings::NotificationCorner,
) -> Rect {
    use gevrek_model::settings::NotificationCorner as Corner;
    let (x, y, width, height) = room.work;
    let (mut top, mut bottom) = (y, y + height);
    if let Some((_, bar_y, _, bar_height)) = room.bar {
        // A panel in the upper half is at the top, else at the bottom.
        if bar_y + bar_height / 2 < y + height / 2 {
            top = top.max(bar_y + bar_height);
        } else {
            bottom = bottom.min(bar_y);
        }
    }
    let wide = size.0.min(width - 2 * gap).max(1);
    let high = size.1.min(bottom - top - 2 * gap).max(1);
    let left = match corner {
        Corner::TopLeft | Corner::BottomLeft => x + gap,
        Corner::TopRight | Corner::BottomRight => x + width - gap - wide,
    };
    let up = match corner {
        Corner::TopLeft | Corner::TopRight => top + gap,
        Corner::BottomLeft | Corner::BottomRight => bottom - gap - high,
    };
    (left, up, wide, high)
}

/// The displays of the window manager's state, each with a name no other
/// has: its own (see [`name_of`]), or its number where it has none or
/// another display came with the same. Two displays are never one panel.
pub fn displays(state: &Value) -> Vec<(String, &Value)> {
    let mut found: Vec<(String, &Value)> = Vec::new();
    for (index, monitor) in state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let name = match name_of(monitor) {
            None => format!("#{index}"),
            Some(name) if found.iter().any(|(known, _)| known == name) => {
                format!("{name} #{index}")
            }
            Some(name) => name.to_string(),
        };
        found.push((name, monitor));
    }
    found
}

/// What has to happen to the panels that are open so that they are the
/// ones wanted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// Panels of displays that are gone, or that have none any more.
    pub close: Vec<String>,
    pub open: Vec<(String, Rect)>,
    /// Panels that stay, at another rectangle.
    pub put: Vec<(String, Rect)>,
}

/// Compare the panels that are open with the ones wanted, by display.
pub fn plan(open: &[(String, Rect)], wanted: &[(String, Rect)]) -> Plan {
    let mut plan = Plan::default();
    for (name, _) in open {
        if !wanted.iter().any(|(wanted, _)| wanted == name) {
            plan.close.push(name.clone());
        }
    }
    for (name, rect) in wanted {
        match open.iter().find(|(known, _)| known == name) {
            None => plan.open.push((name.clone(), *rect)),
            Some((_, now)) if now != rect => plan.put.push((name.clone(), *rect)),
            Some(_) => {}
        }
    }
    plan
}

/// Whether an event of the window manager can change what a panel shows:
/// then its state is asked for. Windows that are only moved or resized,
/// gestures on their way and keys that were pressed change nothing a
/// panel shows, and there are many of them.
pub fn follows(kind: &str) -> bool {
    kind.starts_with("workspace.")
        || kind.starts_with("layout.")
        || kind.starts_with("output.")
        || kind.starts_with("keyboard.")
        || kind.starts_with("notices.")
        || matches!(
            kind,
            "window.created"
                | "window.destroyed"
                | "window.focused"
                | "window.minimized"
                | "window.restored"
                | "window.metadata_changed"
                | "window.unmapped"
                // A command may have moved a window to another workspace,
                // or changed the keys' mode: neither has an event.
                | "dispatch.succeeded"
                | "configuration.reloaded"
                | "runtime.started"
        )
}

/// The windows the window manager's state lists, by the ids it gave them.
pub fn windows(state: &Value) -> std::collections::HashSet<u64> {
    let mut found = std::collections::HashSet::new();
    for monitor in state["monitors"].as_array().into_iter().flatten() {
        for workspace in monitor["workspaces"].as_array().into_iter().flatten() {
            for list in [&workspace["windows"], &workspace["detailed_windows"]] {
                for window in list.as_array().into_iter().flatten() {
                    if let Some(id) = window["window_id"].as_u64() {
                        found.insert(id);
                    }
                }
            }
        }
    }
    found
}

/// Whether an event is a reason to ask for the state: it is of a kind
/// that can change what a panel shows ([`follows`]), and where it tells
/// that a window went or changed, the window is one the state had.
///
/// On Windows the window manager tells of every window of the desktop
/// that closes, the ones it never had among them (a tooltip, a menu):
/// several a second on a desktop nobody touches. A panel has nothing of
/// those to forget.
pub fn concerns(kind: &str, window: Option<u64>, known: &std::collections::HashSet<u64>) -> bool {
    follows(kind)
        && match kind {
            "window.destroyed"
            | "window.minimized"
            | "window.unmapped"
            | "window.metadata_changed" => window.is_none_or(|id| known.contains(&id)),
            _ => true,
        }
}

/// Whether this system's shell can do what a module is for. One that
/// cannot is not drawn: a button that does nothing would be a lie.
///
/// Every module this version knows has what it opens on both systems
/// (on Windows the bell is for gevrek's own notifications: the system
/// keeps its own in its own centre); one it does not know is left out.
pub fn works_here(module: &PanelModule) -> bool {
    !matches!(module, PanelModule::Other(_))
}

/// The modules of a panel that are left out on this system, each once,
/// in the order they stand: to say so where the user can read it.
pub fn left_out(bar: &PanelProfile) -> Vec<PanelModule> {
    let mut found: Vec<PanelModule> = Vec::new();
    for module in bar.left.iter().chain(&bar.centre).chain(&bar.right) {
        if !works_here(module) && !found.contains(module) {
            found.push(module.clone());
        }
    }
    found
}

/// How far a wheel's event turns it, in notches: away from the user is
/// more than nothing. A touchpad sends points, forty to a notch.
pub fn notches(unit: egui::MouseWheelUnit, turned: f32) -> f32 {
    match unit {
        egui::MouseWheelUnit::Point => turned / 40.0,
        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => turned,
    }
}

/// Take the whole notches out of what a wheel has turned so far (parts of
/// one are kept for the next time): the workspaces to go on by, where
/// turning towards the user goes to the next one.
pub fn steps(turned: &mut f32) -> i32 {
    let whole = turned.trunc();
    *turned -= whole;
    -(whole as i32)
}

/// The workspace that is `steps` after the one shown (before it, when
/// negative), by its place among the tabs; round the end, as the window
/// manager's own next and previous go. `None` when that is the one shown.
pub fn wheel_to(view: &View, steps: i32) -> Option<usize> {
    let count = view.tabs.len() as i32;
    let now = view.tabs.iter().position(|tab| tab.shown)? as i32;
    let then = (now + steps).rem_euclid(count);
    (then != now).then_some(then as usize)
}

/// How long the wheel counts on from the workspace it last asked for
/// while the window manager has not yet told that it is shown.
const HEADING_HOLDS: std::time::Duration = std::time::Duration::from_millis(600);

/// What the wheel has done over one panel.
#[derive(Debug, Default)]
pub struct Wheel {
    /// Turned and not yet counted: the part of a notch.
    turned: f32,
    /// The workspace last asked for, by its place, and when. The panel
    /// hears of the switch a moment later; a wheel turned fast counts on
    /// from here, not from the workspace the panel was last told of.
    heading: Option<(usize, std::time::Instant)>,
}

impl Wheel {
    /// The wheel was turned by `turned` notches at `now`: the workspace to
    /// ask for, by its place among the tabs, if any.
    pub fn turn(&mut self, view: &View, turned: f32, now: std::time::Instant) -> Option<usize> {
        self.turned += turned;
        let steps = steps(&mut self.turned);
        let count = view.tabs.len() as i32;
        let shown = view.tabs.iter().position(|tab| tab.shown)?;
        let from = match self.heading {
            Some((index, since))
                if index < view.tabs.len()
                    && now.saturating_duration_since(since) < HEADING_HOLDS =>
            {
                index
            }
            _ => shown,
        };
        if steps == 0 {
            return None;
        }
        let then = (from as i32 + steps).rem_euclid(count) as usize;
        if then == from {
            return None;
        }
        self.heading = Some((then, now));
        Some(then)
    }
}

/// What decides where a panel's surface is: when it changes, the surface
/// is made anew.
pub fn geometry(bar: &PanelProfile) -> (PanelEdge, u32, u32, bool) {
    (
        bar.edge,
        bar.height.clamp(16, 200),
        bar.margin.min(100),
        bar.floating,
    )
}

/// How large the surface of a hint is for its words, in points.
pub fn hint_size(ui: &egui::Ui, text: &str) -> egui::Vec2 {
    let words = ui.fonts_mut(|fonts| {
        fonts.layout(
            text.to_string(),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
            HINT_WIDE,
        )
    });
    // (The words, the room around them, the line around that, and a
    // point to spare so that rounding never breaks a line.)
    (words.size() + egui::vec2(24.0, 16.0)).ceil()
}

/// A hint's words go on to a new line beyond this width.
const HINT_WIDE: f32 = 420.0;

/// Draw a hint: the words on the panel's own colours, on one line (or,
/// long ones, on several under each other).
pub fn draw_hint(ui: &mut egui::Ui, bar: &PanelProfile, text: &str) {
    let (red, green, blue) = bar.background;
    let (text_red, text_green, text_blue) = bar.text;
    let ink = egui::Color32::from_rgb(text_red, text_green, text_blue);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(red, green, blue))
                .stroke(egui::Stroke::new(1.0, ink.gamma_multiply(0.25)))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(10, 6)),
        )
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(text).size(13.0).color(ink))
                    .wrap_mode(egui::TextWrapMode::Wrap)
                    .selectable(false),
            );
        });
}

/// When the pointer has rested on something this long, its hint comes.
pub const HINT_AFTER: std::time::Duration = std::time::Duration::from_millis(450);

/// The hint of a panel, from frame to frame: which words are shown (and
/// on which surface), and which are waited for.
pub struct Hints<Surface> {
    pub shown: Option<(Surface, String)>,
    waiting: Option<(String, std::time::Instant)>,
}

impl<Surface> Default for Hints<Surface> {
    fn default() -> Self {
        Self {
            shown: None,
            waiting: None,
        }
    }
}

/// What is to be done about a panel's hint in this frame.
#[derive(Debug, PartialEq)]
pub enum HintStep<Surface> {
    Nothing,
    /// Put this one away.
    Close(Surface),
    /// Put the one shown away (if one is), and show these words.
    Open(Option<Surface>),
    /// Look again after this long.
    Wait(std::time::Duration),
}

impl<Surface: Copy> Hints<Surface> {
    /// `wanted`: the words of what the pointer rests on now.
    pub fn step(&mut self, wanted: Option<&str>, now: std::time::Instant) -> HintStep<Surface> {
        let Some(wanted) = wanted else {
            self.waiting = None;
            return match self.shown.take() {
                Some((surface, _)) => HintStep::Close(surface),
                None => HintStep::Nothing,
            };
        };
        if self
            .shown
            .as_ref()
            .is_some_and(|(_, words)| words == wanted)
        {
            return HintStep::Nothing;
        }
        // Once a hint is up, going on to the next thing shows its hint
        // at once; from nothing, the pointer has to rest first.
        let rested = match &self.waiting {
            Some((words, since)) if words == wanted => now.saturating_duration_since(*since),
            _ => {
                self.waiting = Some((wanted.to_string(), now));
                std::time::Duration::ZERO
            }
        };
        if self.shown.is_some() || rested >= HINT_AFTER {
            self.waiting = None;
            return HintStep::Open(self.shown.take().map(|(surface, _)| surface));
        }
        HintStep::Wait(HINT_AFTER - rested)
    }
}

/// The command that shows a workspace.
pub fn show_workspace(name: &str) -> String {
    serde_json::json!({"command": "switch_workspace_selector", "selector": {"kind": "name", "value": name}}).to_string()
}

/// The command that shows the workspace of a tab on the tab's display:
/// by the display's name and the workspace's, whichever display has the
/// keyboard and whatever its place among the workspaces has become since
/// the panel was drawn. None for a tab that is not there.
pub fn show_tab(view: &View, index: usize) -> Option<String> {
    use gevrek_model::command::{DispatchCommand, MonitorTarget, WorkspaceSelector};
    let tab = view.tabs.get(index)?;
    let display = view.place.name();
    let command = if display.is_empty() {
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Name(tab.name.clone()),
        }
    } else {
        DispatchCommand::FocusMonitorWorkspace {
            target: MonitorTarget::Name(display.to_string()),
            workspace: WorkspaceSelector::Name(tab.name.clone()),
        }
    };
    serde_json::to_string(&command).ok()
}

/// The panel as it is on a display: with what the configuration sets
/// for that display in place of the common settings
/// (`PanelProfile::on`); `None` where that display has no panel. The
/// display is known by several names (what the window manager calls it,
/// its connector): an entry of the configuration by any of them is its.
pub fn bar_on(
    bar: &PanelProfile,
    display: &str,
    place: &gevrek_model::settings::DisplayRef,
) -> Option<PanelProfile> {
    let own = bar
        .displays
        .iter()
        .map(|entry| entry.name.trim())
        .find(|name| !name.is_empty() && place.is_called(name));
    bar.on(own.unwrap_or(display))
}

/// The command that shows a workspace of a display by its place among
/// the display's workspaces. On the display with the keyboard that is a
/// switch; on another the display is named, or the workspace of that
/// place on the display with the keyboard would be shown.
///
/// A workspace that has a name is shown by it, on its display: its place
/// among the display's may be another by the time the command is carried
/// out (a workspace before it closed, the configuration put them in
/// another order), its name is not. One without a name by its place.
pub fn show_workspace_at(display: &str, focused: bool, index: usize, name: &str) -> String {
    use gevrek_model::command::{DispatchCommand, MonitorTarget, WorkspaceSelector};
    let command = if !name.trim().is_empty() && !display.is_empty() {
        DispatchCommand::FocusMonitorWorkspace {
            target: MonitorTarget::Name(display.to_string()),
            workspace: WorkspaceSelector::Name(name.to_string()),
        }
    } else if focused {
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Index(index),
        }
    } else {
        DispatchCommand::FocusMonitorWorkspace {
            target: MonitorTarget::Name(display.to_string()),
            workspace: WorkspaceSelector::Index(index),
        }
    };
    serde_json::to_string(&command).unwrap_or_default()
}

/// The command that gives the workspace with the keyboard a layout.
pub fn set_layout(layout: gevrek_model::layout::LayoutType) -> String {
    serde_json::to_string(&gevrek_model::command::DispatchCommand::SetLayout { layout })
        .unwrap_or_default()
}

/// A layout as the window manager's state names it.
pub fn layout_name(layout: gevrek_model::layout::LayoutType) -> String {
    serde_json::to_value(layout)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A layout's name as it is read: `MasterStack` is "Master stack".
pub fn layout_label(name: &str) -> String {
    let letters: Vec<char> = name.chars().collect();
    let mut label = String::new();
    for (index, letter) in letters.iter().enumerate() {
        // A word begins at a capital that follows a small letter.
        if index > 0 && letter.is_uppercase() && letters[index - 1].is_lowercase() {
            label.push(' ');
            label.extend(letter.to_lowercase());
        } else {
            label.push(*letter);
        }
    }
    label
}

/// The mark for what the window manager has to tell: `1 notice`.
pub fn notices_label(count: usize) -> String {
    format!("{count} notice{}", if count == 1 { "" } else { "s" })
}

/// What a panel is drawn from.
pub struct Shown<'a> {
    pub bar: &'a PanelProfile,
    pub view: &'a View,
    /// The colour of what is active where the panel has none of its own:
    /// the colour around the focused window.
    pub accent: egui::Color32,
    /// Notifications are kept from showing ("do not disturb").
    pub quiet: bool,
    /// How many notices the window manager has; their mark is there for
    /// as long as there are any, whatever stands on the panel.
    pub notices: usize,
    /// The keys' mode when it is not the usual one, else nothing.
    pub mode: &'a str,
    /// A film of a display is being taken.
    pub recording: bool,
    /// A display or a window is being shared with an application.
    pub sharing: bool,
    /// The display, to tell one panel's memory from another's.
    pub output: &'a str,
    /// The icons of programs that run in the background (the tray).
    pub tray: &'a [crate::tray::Item],
    /// The sound, the battery and the network, as far as they are known.
    pub system: &'a SystemView,
    /// The keyboard layout in use, where the window manager tells it.
    pub keyboard: Option<&'a KeyboardLayout>,
}

/// What the panel shows of the machine. Each is `None` where there is
/// nothing to say (no sound server, no battery, nobody to ask about the
/// network): its module then shows nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SystemView {
    /// How loud, in percent of full, and whether it is muted.
    pub volume: Option<(u32, bool)>,
    pub battery: Option<gevrek_system::status::Battery>,
    pub network: Option<gevrek_system::status::Network>,
    /// How bright the machine's own display is, in percent of the most.
    pub brightness: Option<u32>,
}

impl SystemView {
    /// What there is to say now.
    pub fn read() -> Self {
        Self {
            volume: gevrek_system::sound::level(gevrek_system::sound::Device::Speaker)
                .map(|level| (level.percent, level.muted)),
            battery: gevrek_system::status::battery(),
            network: gevrek_system::status::network(),
            brightness: gevrek_system::brightness::read().map(|now| now.percent()),
        }
    }
}

/// What the sound's module asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundAsk {
    /// Louder (or quieter) by so many percent of full.
    Step(i32),
    Mute,
}

/// The sign for how loud it is.
/// A module's sign from the brand's own icons, with the words beside it
/// (under it on a panel that stands upright); `or` is the icon font's
/// sign, for where the brand's cannot be drawn. `pressed`: it is a
/// button; else it only says something.
#[allow(clippy::too_many_arguments)]
fn signed(
    ui: &mut egui::Ui,
    icon: gevrek_ui::brand_icons::BrandIcon,
    or: &str,
    size: f32,
    colour: egui::Color32,
    words: Option<&str>,
    upright: bool,
    pressed: bool,
) -> egui::Response {
    let text = |said: String| egui::RichText::new(said).size(13.0).color(colour);
    let Some(image) = gevrek_ui::brand_icons::image(ui.ctx(), icon, size, colour) else {
        let joined = if upright { "\n" } else { " " };
        let said = match words {
            Some(words) => text(format!("{or}{joined}{words}")),
            None => egui::RichText::new(or).size(size).color(colour),
        };
        return if pressed {
            ui.add(egui::Button::new(said).frame(false))
        } else {
            ui.label(said)
        };
    };
    match (words, pressed, upright) {
        (None, true, _) => ui.add(egui::Button::image(image).frame(false)),
        (None, false, _) => ui.add(image),
        (Some(words), true, false) => {
            ui.add(egui::Button::image_and_text(image, text(words.to_string())).frame(false))
        }
        (Some(words), _, _) => {
            let sense = if pressed {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            };
            let both = |ui: &mut egui::Ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 1.0);
                ui.add(image);
                ui.add(egui::Label::new(text(words.to_string())).selectable(false));
            };
            let inner = if upright {
                ui.vertical_centered(both).response
            } else {
                ui.horizontal(both).response
            };
            ui.interact(
                inner.rect,
                ui.scope_id().with(("signed", icon.name())),
                sense,
            )
        }
    }
}

/// The brand's sign for the sound, where it has one for how it is (it
/// has none for "muted" and "no sound": those stay the icon font's).
fn sound_sign(percent: u32, muted: bool) -> Option<gevrek_ui::brand_icons::BrandIcon> {
    (!muted && percent > 0).then_some(gevrek_ui::brand_icons::BrandIcon::Volume)
}

pub fn sound_icon(percent: u32, muted: bool) -> &'static str {
    match (muted, percent) {
        (true, _) => icons::SPEAKER_X,
        (_, 0) => icons::SPEAKER_NONE,
        (_, 1..=49) => icons::SPEAKER_LOW,
        _ => icons::SPEAKER_HIGH,
    }
}

/// The sign for how full the battery is.
pub fn battery_icon(battery: gevrek_system::status::Battery) -> &'static str {
    match (battery.charging, battery.percent) {
        (true, _) => icons::BATTERY_CHARGING,
        (_, 0..=9) => icons::BATTERY_EMPTY,
        (_, 10..=34) => icons::BATTERY_LOW,
        (_, 35..=64) => icons::BATTERY_MEDIUM,
        (_, 65..=89) => icons::BATTERY_HIGH,
        _ => icons::BATTERY_FULL,
    }
}

/// What a frame of a panel asks its program to do.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Asked {
    /// Show this workspace (its place among the tabs).
    pub show: Option<usize>,
    /// Go to this window (a button of the taskbar).
    pub go: Option<u64>,
    /// Give this workspace (its place among the tabs) another name: its
    /// tab was right-clicked.
    pub rename: Option<usize>,
    /// The taskbar's own button was pressed: show the windows of every
    /// workspace (true), or of the shown one only.
    pub all_tasks: Option<bool>,
    /// The layout was pressed: open, or put away, the menu of layouts
    /// next to this rectangle.
    pub layout_menu: Option<egui::Rect>,
    /// The notices' mark was pressed: show them next to this rectangle.
    pub notices: Option<egui::Rect>,
    /// A button that opens something of the shell's was pressed.
    pub pressed: Option<PanelModule>,
    /// An icon of the tray was pressed: which, whether its menu is asked
    /// for (the other button, or an icon that is only a menu), and the
    /// rectangle a menu is shown next to.
    pub tray: Option<(String, bool, egui::Rect)>,
    /// The icon of the tray the pointer is over (the wheel is its then).
    pub over_tray: Option<String>,
    /// The wheel was turned over an icon of the tray: which, and by how
    /// many notches (away from the user is more than nothing).
    pub tray_wheel: Option<(String, i32)>,
    /// The middle button was pressed on an icon of the tray.
    pub tray_middle: Option<String>,
    /// The sound's module was pressed, or the wheel turned over it.
    pub sound: Option<SoundAsk>,
    /// The pointer is over the sound's module (the wheel is its then).
    pub over_sound: bool,
    /// The recording's mark was pressed: end the recording.
    pub end_recording: bool,
    /// The sharing's mark was pressed: end the sharing.
    pub end_sharing: bool,
    /// The sound's module was pressed with the other button: open, or
    /// put away, the menu of devices next to this rectangle.
    pub sound_menu: Option<egui::Rect>,
    /// The wheel turned over the brightness: brighter (or darker) by so
    /// many percent of the most.
    pub brightness: Option<i32>,
    /// The pointer is over the brightness (the wheel is its then).
    pub over_brightness: bool,
    /// The keyboard's layout was pressed: go to the next one (true) or
    /// the one before.
    pub keyboard: Option<bool>,
    /// The parts of the panel's surface that are the panel: what the
    /// pointer reaches (the rest lets clicks through to what is under).
    pub reach: Vec<egui::Rect>,
    /// The pointer rests on something that has a word to say about
    /// itself: where it is, and the word.
    pub hint: Option<(egui::Rect, String)>,
}

fn hint_id() -> egui::Id {
    egui::Id::unique("gevrek panel hint")
}

/// A word about what the pointer rests on. Not the toolkit's own: that
/// one is drawn inside the panel's surface, which is as low as the panel
/// and cuts it off. This one is noted, and shown by the shell on a
/// surface of its own under the panel (or over it, at the bottom).
trait Hinted {
    fn hint(self, text: impl Into<String>) -> Self;
}

impl Hinted for egui::Response {
    fn hint(self, text: impl Into<String>) -> Self {
        if self.hovered() || self.contains_pointer() {
            let noted = Some((self.rect, text.into()));
            self.ctx.data_mut(|data| data.insert_temp(hint_id(), noted));
        }
        self
    }
}

/// The part of a panel's surface the panel is drawn on: so long, at the
/// start, in the middle or at the end of the surface (from the left; down
/// a side, from the top).
pub fn strip_of(whole: egui::Rect, length: f32, align: PanelAlign, upright: bool) -> egui::Rect {
    let most = if upright {
        whole.height()
    } else {
        whole.width()
    };
    let length = length.min(most).max(0.0);
    let (from, middle, to) = if upright {
        (whole.top(), whole.center().y, whole.bottom())
    } else {
        (whole.left(), whole.center().x, whole.right())
    };
    let start = match align {
        PanelAlign::Start => from,
        PanelAlign::Centre => middle - length / 2.0,
        PanelAlign::End => to - length,
    };
    if upright {
        egui::Rect::from_min_size(
            egui::pos2(whole.left(), start),
            egui::vec2(whole.width(), length),
        )
    } else {
        egui::Rect::from_min_size(
            egui::pos2(start, whole.top()),
            egui::vec2(length, whole.height()),
        )
    }
}

/// The colours the modules are drawn in.
struct Ink {
    text: egui::Color32,
    accent: egui::Color32,
    /// The panel goes down a side: what it shows stands under each
    /// other and is kept narrow (a word is a sign, or a few letters).
    upright: bool,
}

/// Draw a panel. `wheel` is what the wheel has turned over this panel and
/// not yet been counted for: with it, the wheel anywhere over the panel
/// goes through the workspaces.
pub fn draw(ui: &mut egui::Ui, shown: &Shown, wheel: Option<&mut Wheel>) -> Asked {
    let context = ui.ctx().clone();
    let bar = shown.bar;
    let rgb = |(red, green, blue): (u8, u8, u8)| egui::Color32::from_rgb(red, green, blue);
    let ink = Ink {
        text: rgb(bar.text),
        accent: bar.accent.map_or(shown.accent, rgb),
        upright: bar.edge.is_upright(),
    };
    let (red, green, blue) = bar.background;
    let background = egui::Color32::from_rgba_unmultiplied(red, green, blue, bar.opacity);
    let mut asked = Asked::default();
    // What the wheel turned in this frame: over the sound's module it
    // makes louder and quieter, anywhere else on the panel it goes through
    // the workspaces (decided below, when the modules have their places).
    let turned: f32 = context.input(|input| {
        input
            .events
            .iter()
            .map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => notches(*unit, delta.y),
                _ => 0.0,
            })
            .sum()
    });
    let padding = bar.padding.min(60) as f32;
    let spacing = bar.spacing.min(40) as f32;
    let radius = bar.corner_radius.min(100) as u8;
    // As wide as what it shows: by how wide that was the last time.
    let fitted = bar.width == 0 && bar.fit == PanelFit::Content;
    let natural_id = egui::Id::unique(("panel-natural", shown.output));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            // The surface reaches from side to side (down a side: from
            // top to bottom); the panel is the part of it that is drawn
            // (and that the pointer reaches).
            let whole = ui.max_rect();
            let upright = bar.edge.is_upright();
            let long = |rect: egui::Rect| {
                if upright {
                    rect.height()
                } else {
                    rect.width()
                }
            };
            // (Along the panel, across it.)
            let along = |long: f32, across: f32| {
                if upright {
                    egui::vec2(across, long)
                } else {
                    egui::vec2(long, across)
                }
            };
            let natural: Option<f32> = context.data(|data| data.get_temp(natural_id));
            let length = if bar.width > 0 {
                bar.width as f32
            } else if fitted {
                natural.unwrap_or(long(whole))
            } else {
                long(whole)
            }
            .min(long(whole))
            .max(1.0);
            let strip = strip_of(whole, length, bar.align, upright);
            if bar.style == PanelStyle::Bar {
                ui.painter().rect_filled(strip, radius, background);
            }
            // The pieces of an island panel are painted behind what
            // stands on them, once that has its place.
            let islands: Vec<egui::layers::ShapeIdx> = (0..3)
                .map(|_| ui.painter().add(egui::Shape::Noop))
                .collect();
            let inner = strip.shrink2(along(padding.min(long(strip) / 2.0), 0.0));
            let forward = if upright {
                egui::Layout::top_down(egui::Align::Center)
            } else {
                egui::Layout::left_to_right(egui::Align::Center)
            };
            let backward = if upright {
                egui::Layout::bottom_up(egui::Align::Center)
            } else {
                egui::Layout::right_to_left(egui::Align::Center)
            };
            // Where the three groups ended up.
            let mut groups: [Option<egui::Rect>; 3] = [None; 3];
            let group = |ui: &mut egui::Ui,
                         modules: &[PanelModule],
                         marks: bool,
                         reversed: bool,
                         asked: &mut Asked|
             -> Option<egui::Rect> {
                if modules.is_empty() && !marks {
                    return None;
                }
                let rect = ui
                    .scope(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(spacing, spacing);
                        if reversed {
                            // The last one stands at the edge.
                            for module in modules.iter().rev() {
                                draw_module(ui, module, shown, &ink, asked);
                            }
                            if marks {
                                draw_marks(ui, shown, &ink, asked);
                            }
                        } else {
                            if marks {
                                draw_marks(ui, shown, &ink, asked);
                            }
                            for module in modules {
                                draw_module(ui, module, shown, &ink, asked);
                            }
                        }
                    })
                    .response
                    .rect;
                (long(rect) > 0.5).then_some(rect)
            };
            if fitted {
                // One run: the left, the middle, the right, with room
                // between the groups; how long it came to be is how long
                // the panel is from the next frame on.
                let between = if bar.style == PanelStyle::Islands {
                    padding * 2.0 + spacing
                } else {
                    spacing * 2.0
                };
                let builder = egui::UiBuilder::new().max_rect(inner).layout(forward);
                ui.scope_builder(builder, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    groups[0] = group(ui, &bar.left, false, false, &mut asked);
                    if groups[0].is_some() && !bar.centre.is_empty() {
                        ui.add_space(between);
                    }
                    groups[1] = group(ui, &bar.centre, false, false, &mut asked);
                    if groups[0].is_some() || groups[1].is_some() {
                        ui.add_space(between);
                    }
                    groups[2] = group(ui, &bar.right, true, false, &mut asked);
                });
                let used = groups
                    .iter()
                    .flatten()
                    .fold(None, |all: Option<egui::Rect>, one| {
                        Some(all.map_or(*one, |all| all.union(*one)))
                    })
                    .map_or(0.0, long)
                    + padding * 2.0;
                if natural.is_none_or(|natural| (used - natural).abs() > 0.5) {
                    context.data_mut(|data| data.insert_temp(natural_id, used));
                    context.request_repaint();
                }
            } else {
                let builder = egui::UiBuilder::new().max_rect(inner).layout(forward);
                ui.scope_builder(builder, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    groups[0] = group(ui, &bar.left, false, false, &mut asked);
                    ui.with_layout(backward, |ui| {
                        groups[2] = group(ui, &bar.right, true, true, &mut asked);
                    });
                });
                if !bar.centre.is_empty() {
                    // In the middle of the panel whatever stands at its
                    // ends: placed by how long it was the last time.
                    let id = egui::Id::unique(("panel-centre", shown.output));
                    let before: f32 = context.data(|data| data.get_temp(id)).unwrap_or(0.0);
                    let across = if upright {
                        strip.width()
                    } else {
                        strip.height()
                    };
                    let place = egui::Rect::from_center_size(
                        strip.center(),
                        along(before.max(1.0), across),
                    );
                    let builder = egui::UiBuilder::new().max_rect(place).layout(forward);
                    ui.scope_builder(builder, |ui| {
                        groups[1] = group(ui, &bar.centre, false, false, &mut asked);
                    });
                    let used = groups[1].map_or(0.0, long);
                    if (used - before).abs() > 0.5 {
                        context.data_mut(|data| data.insert_temp(id, used));
                        context.request_repaint();
                    }
                }
            }
            // What the pointer reaches: the strip, or the pieces.
            asked.reach = match bar.style {
                PanelStyle::Bar => vec![strip],
                PanelStyle::Islands | PanelStyle::Bare => groups
                    .iter()
                    .flatten()
                    .map(|rect| {
                        if upright {
                            egui::Rect::from_x_y_ranges(
                                strip.x_range(),
                                (rect.top() - padding)..=(rect.bottom() + padding),
                            )
                        } else {
                            egui::Rect::from_x_y_ranges(
                                (rect.left() - padding)..=(rect.right() + padding),
                                strip.y_range(),
                            )
                        }
                    })
                    .collect(),
            };
            if bar.style == PanelStyle::Islands {
                for (slot, piece) in islands.iter().zip(&asked.reach) {
                    ui.painter()
                        .set(*slot, egui::Shape::rect_filled(*piece, radius, background));
                }
            }
        });
    asked.hint = context
        .data_mut(|data| data.remove_temp::<Option<(egui::Rect, String)>>(hint_id()))
        .flatten();
    if turned != 0.0 {
        if let Some(key) = asked.over_tray.clone() {
            // The wheel over an icon of the tray is its program's.
            let notches = turned.abs().ceil().max(1.0) as i32;
            asked.tray_wheel = Some((key, if turned > 0.0 { notches } else { -notches }));
        } else if asked.over_sound {
            // Away from the user: louder. A notch is five percent.
            let notches = turned.abs().ceil().max(1.0) as i32;
            asked.sound = Some(SoundAsk::Step(if turned > 0.0 { 5 } else { -5 } * notches));
        } else if asked.over_brightness {
            // Away from the user: brighter. A notch is five percent.
            let notches = turned.abs().ceil().max(1.0) as i32;
            asked.brightness = Some(if turned > 0.0 { 5 } else { -5 } * notches);
        } else if let Some(wheel) = wheel {
            asked.show = wheel.turn(shown.view, turned, std::time::Instant::now());
        }
    }
    asked
}

/// The icons of the tray. `from_right`: the layout runs from the right
/// (the first to come still stands first).
fn draw_tray(ui: &mut egui::Ui, shown: &Shown, ink: &Ink, asked: &mut Asked, from_right: bool) {
    let mut one = |item: &crate::tray::Item| {
        let (pressed, over, middle) = draw_tray_icon(ui, item, ink);
        if let Some((menu, anchor)) = pressed {
            asked.tray = Some((item.key.clone(), menu, anchor));
        }
        if over {
            asked.over_tray = Some(item.key.clone());
        }
        if middle {
            asked.tray_middle = Some(item.key.clone());
        }
    };
    if from_right {
        shown.tray.iter().rev().for_each(&mut one);
    } else {
        shown.tray.iter().for_each(&mut one);
    }
}

/// What is on a panel without being one of its modules, for as long as
/// there is something to say: the window manager's notices, and the keys'
/// mode when it is not the usual one.
fn draw_marks(ui: &mut egui::Ui, shown: &Shown, ink: &Ink, asked: &mut Asked) {
    // The tray, on a panel that gives it no place of its own.
    if !shown.bar.shows(PanelModule::Tray) {
        draw_tray(ui, shown, ink, asked, true);
    }
    // A film is being taken: said for as long as it is, and a press on
    // it ends it.
    if shown.recording {
        let red = egui::Color32::from_rgb(220, 60, 60);
        let words = if ink.upright { "" } else { "  recording" };
        let label = egui::RichText::new(format!("{}{words}", icons::RECORD))
            .size(12.5)
            .color(theme::readable(egui::Color32::WHITE, red, 4.5));
        let button = egui::Button::new(label).fill(red).corner_radius(10);
        if ui
            .add(button)
            .hint("The screen is being recorded. A press ends it.")
            .clicked()
        {
            asked.end_recording = true;
        }
    }
    // The screen is shared with an application: said for as long as it
    // is, and a press on it ends it.
    if shown.sharing {
        let red = egui::Color32::from_rgb(220, 60, 60);
        let words = if ink.upright { "" } else { "  sharing" };
        let label = egui::RichText::new(format!("{}{words}", icons::SCREENCAST))
            .size(12.5)
            .color(theme::readable(egui::Color32::WHITE, red, 4.5));
        let button = egui::Button::new(label).fill(red).corner_radius(10);
        if ui
            .add(button)
            .hint("The screen is being shared with an application. A press ends it.")
            .clicked()
        {
            asked.end_sharing = true;
        }
    }
    if shown.notices > 0 {
        let warning = egui::Color32::from_rgb(240, 180, 40);
        let label = egui::RichText::new(format!(
            "{}  {}",
            icons::WARNING,
            notices_label(shown.notices)
        ))
        .size(12.5)
        .color(theme::readable(egui::Color32::BLACK, warning, 4.5));
        let button = egui::Button::new(label).fill(warning).corner_radius(10);
        let response = ui.add(button);
        if response.clicked() {
            asked.notices = Some(response.rect);
        }
    }
    if !shown.mode.is_empty() {
        let label = egui::RichText::new(format!("keys: {}", shown.mode))
            .size(12.5)
            .color(theme::readable(egui::Color32::BLACK, ink.accent, 4.5));
        // A sign, not a button: nothing happens when it is pressed.
        egui::Frame::new()
            .fill(ink.accent)
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| ui.label(label));
    }
}

/// One icon of the tray. When it was pressed: whether its menu is asked
/// for, and where it stands.
/// An icon of the tray. What a press on it asks for (its menu or not,
/// and where a menu goes), whether the pointer is over it, and whether
/// the middle button was pressed on it.
fn draw_tray_icon(
    ui: &mut egui::Ui,
    item: &crate::tray::Item,
    ink: &Ink,
) -> (Option<(bool, egui::Rect)>, bool, bool) {
    let side = (ui.max_rect().height() - 8.0).clamp(12.0, 20.0);
    let response = match &item.picture {
        Some(picture) => {
            // The picture is given to the drawing once, and again when
            // the program shows another.
            let id = egui::Id::unique(("tray-icon", &item.key));
            let kept: Option<(u64, egui::TextureHandle)> = ui.ctx().data(|data| data.get_temp(id));
            let texture = match kept {
                Some((stamp, texture)) if stamp == item.stamp => texture,
                _ => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [picture.width as usize, picture.height as usize],
                        &picture.pixels,
                    );
                    let texture = ui.ctx().load_texture(
                        format!("tray-{}", item.key),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(id, (item.stamp, texture.clone())));
                    texture
                }
            };
            let image = egui::Image::new((texture.id(), egui::vec2(side, side)))
                .sense(egui::Sense::click());
            ui.add(image)
        }
        None => {
            let label = egui::RichText::new(item.letter())
                .size(12.5)
                .color(ink.text);
            let button = egui::Button::new(label)
                .fill(ink.text.gamma_multiply(0.10))
                .corner_radius(10)
                .min_size(egui::vec2(side, side));
            ui.add(button)
        }
    };
    let response = if item.hint.is_empty() {
        response
    } else {
        response.hint(&item.hint)
    };
    // Its program wants to be looked at: a dot at the icon's corner.
    if item.attention {
        ui.painter().circle_filled(
            response.rect.right_top() + egui::vec2(-2.5, 2.5),
            3.5,
            egui::Color32::from_rgb(240, 180, 40),
        );
    }
    let pressed = if response.clicked() {
        Some((item.menu_only, response.rect))
    } else if response.secondary_clicked() {
        Some((true, response.rect))
    } else {
        None
    };
    (
        pressed,
        response.hovered(),
        response.clicked_by(egui::PointerButton::Middle),
    )
}

/// How high a line of a tray's menu is, and a separator in it, in points.
const TRAY_LINE: f32 = 28.0;
const TRAY_SEPARATOR: f32 = 9.0;

/// The line of a menu the keys are on, moved by the arrow keys among the
/// `count` lines of which `live` says whether one can be chosen; and
/// whether Enter (or the space key) chose it now. The menu remembers the
/// line between two frames; `fresh` forgets it (another menu is shown in
/// the same place).
pub fn menu_keys(
    context: &egui::Context,
    count: usize,
    live: impl Fn(usize) -> bool,
    fresh: bool,
) -> (Option<usize>, bool) {
    let id = egui::Id::unique("the line of a menu the keys are on");
    let mut on: Option<usize> = if fresh {
        None
    } else {
        context
            .data(|data| data.get_temp::<Option<usize>>(id))
            .flatten()
    };
    on = on.filter(|index| *index < count && live(*index));
    let pressed = |key| context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
    let step = |from: Option<usize>, forward: bool| -> Option<usize> {
        // The next line that can be chosen, round the ends.
        let start = match (from, forward) {
            (Some(index), true) => index + 1,
            (Some(index), false) => index + count - 1,
            (None, true) => 0,
            (None, false) => count.saturating_sub(1),
        };
        (0..count)
            .map(|offset| {
                if forward {
                    (start + offset) % count
                } else {
                    (start + count - offset % count) % count
                }
            })
            .find(|index| live(*index))
    };
    if count > 0 {
        if pressed(egui::Key::ArrowDown) {
            on = step(on, true);
        }
        if pressed(egui::Key::ArrowUp) {
            on = step(on, false);
        }
        if pressed(egui::Key::Home) {
            on = step(None, true);
        }
        if pressed(egui::Key::End) {
            on = step(None, false);
        }
    }
    let chosen = on.is_some() && (pressed(egui::Key::Enter) || pressed(egui::Key::Space));
    context.data_mut(|data| data.insert_temp(id, on));
    (on, chosen)
}

/// Whether the Escape key asks a menu to go.
pub fn menu_escape(context: &egui::Context) -> bool {
    context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

/// The mark of the line of a menu the keys are on: an edge around it.
fn menu_mark(ui: &egui::Ui, line: egui::Rect, palette: &theme::Palette) {
    ui.painter().rect_stroke(
        line,
        6,
        egui::Stroke::new(1.5, palette.accent),
        egui::StrokeKind::Inside,
    );
}

/// How large one menu of a tray's icon is, in points: `back` when it is
/// a submenu, which has a line that leads back above its own.
fn tray_lines_size(entries: &[crate::tray::Entry], back: bool) -> egui::Vec2 {
    let marked = entries.iter().any(|entry| entry.toggle.is_some());
    let pictured = entries.iter().any(|entry| entry.picture.is_some());
    // About how wide the longest line is written, with what stands
    // before and after it.
    let widest = entries
        .iter()
        .map(|entry| {
            7.5 * entry.label.chars().count() as f32
                + if entry.keys.is_empty() {
                    0.0
                } else {
                    18.0 + 7.0 * entry.keys.chars().count() as f32
                }
                + if entry.submenu { 18.0 } else { 0.0 }
        })
        .fold(0.0, f32::max);
    // Each line is two points apart from the next.
    let height: f32 = entries
        .iter()
        .map(|entry| {
            2.0 + if entry.separator {
                TRAY_SEPARATOR
            } else {
                TRAY_LINE
            }
        })
        .sum::<f32>()
        + if back { TRAY_LINE + 2.0 } else { 0.0 };
    egui::vec2(
        40.0 + widest + if marked { 22.0 } else { 0.0 } + if pictured { 22.0 } else { 0.0 },
        12.0 + height,
    )
}

/// How large the menu of a tray's icon is, in points: as large as the
/// largest of the menu and its submenus, each of which is shown in the
/// same place.
pub fn tray_menu_size(menu: &crate::tray::Node) -> egui::Vec2 {
    fn largest(menu: &crate::tray::Node, back: bool, size: &mut egui::Vec2) {
        *size = size.max(tray_lines_size(&crate::tray::lines(menu), back));
        for child in menu.children.iter().filter(|child| child.visible) {
            if !child.children.is_empty() {
                largest(child, true, size);
            }
        }
    }
    let mut size = egui::Vec2::ZERO;
    largest(menu, false, &mut size);
    egui::vec2(size.x.clamp(160.0, 420.0), size.y.clamp(40.0, 600.0))
}

/// What was done in the menu of a tray's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenuAsk {
    /// A line was pressed: its number.
    Press(i32),
    /// A submenu was opened: its number.
    Open(i32),
}

/// The menu of a tray's icon: the lines its program gives, with their
/// pictures, marks and keys. A line with a submenu opens it in the menu's
/// place (`path`: the submenus gone down into), under a line that leads
/// back. What was done, if anything.
pub fn tray_menu(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    menu: &crate::tray::Node,
    path: &mut Vec<i32>,
) -> Option<TrayMenuAsk> {
    use crate::tray::Toggle;
    // (A submenu that is gone since: as far as the path still holds.)
    let (shown, held) = menu.at(path);
    path.truncate(held);
    let entries = crate::tray::lines(shown);
    let heading = (!path.is_empty()).then(|| crate::tray::plain(&shown.label));
    let mut asked = None;
    let mut back = ui
        .ctx()
        .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft))
        && !path.is_empty();
    // The keys: the arrows go from line to line, Enter chooses, the
    // right arrow opens a submenu (the left one leads back).
    let level = egui::Id::unique("the menu of a tray's icon that was shown last");
    let fresh = ui.ctx().data(|data| data.get_temp::<i32>(level)) != Some(shown.id);
    ui.ctx().data_mut(|data| data.insert_temp(level, shown.id));
    let (on, chose) = menu_keys(
        ui.ctx(),
        entries.len(),
        |index| !entries[index].separator && entries[index].enabled,
        fresh,
    );
    let right = ui
        .ctx()
        .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight));
    if let Some(entry) = on.and_then(|index| entries.get(index)) {
        if entry.submenu && (chose || right) {
            asked = Some(TrayMenuAsk::Open(entry.id));
        } else if chose {
            asked = Some(TrayMenuAsk::Press(entry.id));
        }
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let font = egui::FontId::proportional(13.0);
            // In a submenu: its name, and a press leads back.
            if let Some(heading) = &heading {
                let (line, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), TRAY_LINE),
                    egui::Sense::click(),
                );
                if response.hovered() {
                    ui.painter()
                        .rect_filled(line, 6, palette.text.gamma_multiply(0.10));
                }
                ui.painter().text(
                    egui::pos2(line.left() + 10.0, line.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{}  {heading}", icons::CARET_LEFT),
                    font.clone(),
                    palette.muted,
                );
                if response.clicked() {
                    back = true;
                }
            }
            // Room for the marks and the pictures only where a line has one.
            let marked = entries.iter().any(|entry| entry.toggle.is_some());
            let pictured = entries.iter().any(|entry| entry.picture.is_some());
            for (index, entry) in entries.iter().enumerate() {
                if entry.separator {
                    let (line, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), TRAY_SEPARATOR),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(
                        line.shrink2(egui::vec2(4.0, (TRAY_SEPARATOR - 1.0) / 2.0)),
                        0,
                        palette.muted.gamma_multiply(0.35),
                    );
                    continue;
                }
                // A line its program has switched off is read, not pressed.
                let live = entry.enabled;
                let (line, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), TRAY_LINE),
                    if live {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    },
                );
                if live && response.hovered() {
                    ui.painter()
                        .rect_filled(line, 6, palette.text.gamma_multiply(0.10));
                }
                if on == Some(index) {
                    menu_mark(ui, line, palette);
                }
                let ink = if live { palette.text } else { palette.muted };
                let mut left = line.left() + 10.0;
                if marked {
                    let mark = match entry.toggle {
                        Some((Toggle::Check, true)) => icons::CHECK,
                        Some((Toggle::Radio, true)) => "•",
                        _ => "",
                    };
                    ui.painter().text(
                        egui::pos2(left + 7.0, line.center().y),
                        egui::Align2::CENTER_CENTER,
                        mark,
                        font.clone(),
                        ink,
                    );
                    left += 22.0;
                }
                if pictured {
                    if let Some(picture) = &entry.picture {
                        // Given to the drawing once for as long as the
                        // menu is shown (a menu's pictures do not change
                        // under the pointer).
                        let id = egui::Id::unique(("tray-line", shown.id, entry.id));
                        let kept: Option<egui::TextureHandle> =
                            ui.ctx().data(|data| data.get_temp(id));
                        let texture = kept.unwrap_or_else(|| {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [picture.width as usize, picture.height as usize],
                                &picture.pixels,
                            );
                            let texture = ui.ctx().load_texture(
                                format!("tray-line-{}-{}", shown.id, entry.id),
                                image,
                                egui::TextureOptions::LINEAR,
                            );
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(id, texture.clone()));
                            texture
                        });
                        let place = egui::Rect::from_center_size(
                            egui::pos2(left + 8.0, line.center().y),
                            egui::vec2(16.0, 16.0),
                        );
                        let whole =
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        let tint = if live {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_white_alpha(110)
                        };
                        ui.painter().image(texture.id(), place, whole, tint);
                    }
                    left += 22.0;
                }
                ui.painter().text(
                    egui::pos2(left, line.center().y),
                    egui::Align2::LEFT_CENTER,
                    &entry.label,
                    font.clone(),
                    ink,
                );
                // At the right: that it leads on, or its keys.
                if entry.submenu {
                    ui.painter().text(
                        egui::pos2(line.right() - 10.0, line.center().y),
                        egui::Align2::RIGHT_CENTER,
                        icons::CARET_RIGHT,
                        font.clone(),
                        ink,
                    );
                } else if !entry.keys.is_empty() {
                    ui.painter().text(
                        egui::pos2(line.right() - 10.0, line.center().y),
                        egui::Align2::RIGHT_CENTER,
                        &entry.keys,
                        egui::FontId::proportional(12.0),
                        palette.muted,
                    );
                }
                if response.clicked() {
                    asked = Some(if entry.submenu {
                        TrayMenuAsk::Open(entry.id)
                    } else {
                        TrayMenuAsk::Press(entry.id)
                    });
                }
            }
        });
    if back {
        path.pop();
        ui.ctx().request_repaint();
        return None;
    }
    if let Some(TrayMenuAsk::Open(id)) = asked {
        path.push(id);
        ui.ctx().request_repaint();
    }
    asked
}

/// One module, wherever it stands.
fn draw_module(
    ui: &mut egui::Ui,
    module: &PanelModule,
    shown: &Shown,
    ink: &Ink,
    asked: &mut Asked,
) {
    if !works_here(module) {
        return;
    }
    let (bar, view, text, accent) = (shown.bar, shown.view, ink.text, ink.accent);
    // A low panel has low buttons.
    let tall = (ui.max_rect().height() - 6.0).clamp(12.0, 22.0);
    let upright = ink.upright;
    // (Down a side: a sign over its number, not beside it.)
    let joined = if upright { "\n" } else { " " };
    match module {
        PanelModule::Launcher => {
            // gevrek's mark is the launcher's button: the one for a dark
            // ground where the panel's words are light, and the other way
            // round. (Where the picture cannot be read: the glass.)
            let on_dark = theme::luminance(text) > 0.5;
            let side = (ui.max_rect().height() - 10.0).clamp(12.0, 20.0);
            let button = match gevrek_ui::brand::mark(ui.ctx(), on_dark, side) {
                Some(mark) => ui.add(
                    egui::Image::new((mark.id(), egui::vec2(side, side)))
                        .sense(egui::Sense::click()),
                ),
                None => signed(
                    ui,
                    BrandIcon::Launcher,
                    icons::MAGNIFYING_GLASS,
                    15.0,
                    text,
                    None,
                    upright,
                    true,
                ),
            };
            if button.hint("Launcher").clicked() {
                asked.pressed = Some(PanelModule::Launcher);
            }
        }
        PanelModule::Workspaces => {
            for (index, tab) in view.tabs.iter().enumerate() {
                // The one that is shown stands out; one with
                // windows reads stronger than an empty one.
                let (fill, ink) = if tab.shown {
                    (accent, theme::readable(egui::Color32::BLACK, accent, 4.5))
                } else if tab.windows > 0 {
                    (egui::Color32::TRANSPARENT, text)
                } else {
                    (egui::Color32::TRANSPARENT, text.gamma_multiply(0.45))
                };
                let label = egui::RichText::new(&tab.label).size(13.0).color(ink);
                let button = egui::Button::new(label)
                    .fill(fill)
                    .corner_radius(10)
                    .min_size(egui::vec2(30.0, tall));
                let response = ui
                    .add(button)
                    .hint(format!("Workspace {}. Right-click to rename.", tab.name));
                if response.clicked() && !tab.shown {
                    asked.show = Some(index);
                }
                if response.secondary_clicked() {
                    asked.rename = Some(index);
                }
            }
        }
        PanelModule::Layout if !view.layout.is_empty() => {
            // The layout, and a press on it the menu of them.
            let said = if view.floating {
                "Floating".to_string()
            } else {
                layout_label(&view.layout)
            };
            // (Down a side: its first letters, and the whole word as a hint.)
            let whole = said.clone();
            let said = if upright {
                said.chars().take(3).collect()
            } else {
                said
            };
            let label = egui::RichText::new(said).size(12.5).color(text);
            let button = egui::Button::new(label)
                .fill(text.gamma_multiply(0.10))
                .corner_radius(10);
            let response = ui.add(button);
            let response = if upright {
                response.hint(whole)
            } else {
                response
            };
            if response.clicked() {
                asked.layout_menu = Some(response.rect);
            }
        }
        // (A title is as long as it is: not on a panel down a side.)
        PanelModule::Title if !view.title.is_empty() && !upright => {
            let title: String = view.title.chars().take(200).collect();
            let words = egui::RichText::new(title)
                .size(13.0)
                .color(text.gamma_multiply(if view.focused { 1.0 } else { 0.6 }));
            if bar.title_width == 0 {
                ui.label(words);
            } else {
                // A place of one width, whatever it says: a title that
                // changes (a count, a progress) moves nothing after it,
                // and one that is too long ends in "…".
                let wide = (bar.title_width as f32).min(ui.available_width().max(20.0));
                ui.allocate_ui_with_layout(
                    egui::vec2(wide, tall),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(wide);
                        ui.set_max_width(wide);
                        ui.add(egui::Label::new(words).truncate().selectable(false));
                    },
                );
            }
        }
        PanelModule::Clock => {
            let now = jiff::Zoned::now();
            let format = if bar.clock_format.trim().is_empty() {
                "%H:%M"
            } else {
                bar.clock_format.as_str()
            };
            let said = clock_text(format, &now);
            if upright {
                // Its parts under each other.
                for part in said.split_whitespace() {
                    ui.label(egui::RichText::new(part).size(12.0).color(text));
                }
            } else {
                ui.label(egui::RichText::new(said).size(13.0).color(text));
            }
            // Drawn again when the time it shows changes.
            let wait = if format.contains("%S") || format.contains("%T") {
                1000 - u64::from(now.millisecond().unsigned_abs()).min(999)
            } else {
                (60 - u64::from(now.second().unsigned_abs()).min(59)) * 1000
            };
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(wait.max(50)));
        }
        PanelModule::Stats => {
            let windows = view
                .tabs
                .iter()
                .find(|tab| tab.shown)
                .map_or(0, |tab| tab.windows);
            let said = if upright {
                windows.to_string()
            } else if windows == 1 {
                "1 window".to_string()
            } else {
                format!("{windows} windows")
            };
            ui.label(
                egui::RichText::new(said)
                    .size(12.5)
                    .color(text.gamma_multiply(0.7)),
            );
        }
        PanelModule::Notifications => {
            let (icon, hint) = if shown.quiet {
                (icons::BELL_SLASH, "Notifications (do not disturb)")
            } else {
                (icons::BELL, "Notifications")
            };
            // (The brand has one bell: while nothing is to disturb, it
            // is the icon font's bell with the stroke through it.)
            let pressed = if shown.quiet {
                let label = egui::RichText::new(icon).size(15.0).color(text);
                ui.add(egui::Button::new(label).frame(false))
            } else {
                signed(
                    ui,
                    BrandIcon::Notifications,
                    icon,
                    15.0,
                    text,
                    None,
                    upright,
                    true,
                )
            };
            if pressed.hint(hint).clicked() {
                asked.pressed = Some(PanelModule::Notifications);
            }
        }
        PanelModule::Power => {
            if signed(
                ui,
                BrandIcon::Power,
                icons::POWER,
                15.0,
                text,
                None,
                upright,
                true,
            )
            .hint("Power")
            .clicked()
            {
                asked.pressed = Some(PanelModule::Power);
            }
        }
        PanelModule::Tray => {
            let from_right = ui.layout().main_dir() == egui::Direction::RightToLeft;
            draw_tray(ui, shown, ink, asked, from_right);
        }
        PanelModule::Sound => {
            if let Some((percent, muted)) = shown.system.volume {
                let said = format!("{}{joined}{percent}%", sound_icon(percent, muted));
                let colour = if muted {
                    text.gamma_multiply(0.55)
                } else {
                    text
                };
                let label = egui::RichText::new(said).size(13.0).color(colour);
                let hint = if muted {
                    "Sound: muted. A press lets it be heard; the wheel makes it louder and quieter; right-click chooses the device."
                } else {
                    "Sound. A press mutes it; the wheel makes it louder and quieter; right-click chooses the device."
                };
                let button = match sound_sign(percent, muted) {
                    Some(sign) => signed(
                        ui,
                        sign,
                        sound_icon(percent, muted),
                        15.0,
                        colour,
                        Some(&format!("{percent}%")),
                        upright,
                        true,
                    ),
                    None => ui.add(egui::Button::new(label).frame(false)),
                }
                .hint(hint);
                if button.clicked() {
                    asked.sound = Some(SoundAsk::Mute);
                }
                // The other button: which device to play on.
                if button.secondary_clicked() {
                    asked.sound_menu = Some(button.rect);
                }
                if button.contains_pointer() {
                    asked.over_sound = true;
                }
            }
        }
        PanelModule::Brightness => {
            if let Some(percent) = shown.system.brightness {
                let label = egui::RichText::new(format!("{}{joined}{percent}%", icons::SUN))
                    .size(13.0)
                    .color(text);
                let response = ui
                    .label(label)
                    .hint("Brightness. The wheel makes it brighter and darker.");
                // (The wheel over it is its own, as over the sound.)
                if response.contains_pointer() {
                    asked.over_brightness = true;
                }
            }
        }
        PanelModule::Battery => {
            if let Some(battery) = shown.system.battery {
                let said = format!("{}{joined}{}%", battery_icon(battery), battery.percent);
                let low = !battery.charging && battery.percent < 15;
                let colour = if low {
                    egui::Color32::from_rgb(240, 110, 90)
                } else {
                    text
                };
                // (The brand's battery, but while it is charged: for
                // that the pack has no sign.)
                if battery.charging {
                    ui.label(egui::RichText::new(said).size(13.0).color(colour))
                } else {
                    signed(
                        ui,
                        BrandIcon::Battery,
                        battery_icon(battery),
                        15.0,
                        colour,
                        Some(&format!("{}%", battery.percent)),
                        upright,
                        false,
                    )
                }
                .hint(if battery.charging {
                    "Battery, on the mains"
                } else {
                    "Battery"
                });
            }
        }
        PanelModule::Network => {
            if let Some(network) = &shown.system.network {
                let (icon, hint) = match (network.connected, network.wireless) {
                    (true, true) => (icons::WIFI_HIGH, "Wireless network"),
                    (true, false) => (icons::PLUGS_CONNECTED, "Network by cable"),
                    (false, _) => (icons::WIFI_SLASH, "No network"),
                };
                let colour = if network.connected {
                    text
                } else {
                    text.gamma_multiply(0.55)
                };
                let hint = if network.name.is_empty() {
                    hint.to_string()
                } else {
                    format!("{hint}: {}", network.name)
                };
                // A press opens the control centre, where the network is
                // named with the rest of what is set often.
                // (The brand's sign is the wireless one: a cable and
                // "none" stay the icon font's.)
                let pressed = if network.connected && network.wireless {
                    signed(
                        ui,
                        BrandIcon::Network,
                        icon,
                        15.0,
                        colour,
                        None,
                        upright,
                        true,
                    )
                } else {
                    let label = egui::RichText::new(icon).size(15.0).color(colour);
                    ui.add(egui::Button::new(label).frame(false))
                };
                if pressed
                    .hint(format!("{hint}. A press opens the control centre."))
                    .clicked()
                {
                    asked.pressed = Some(PanelModule::Network);
                }
            }
        }
        PanelModule::Taskbar => {
            // Whose windows it has: every workspace's, or the shown
            // one's. Its own small button says which, and changes it.
            let all = bar.tasks_of_all_workspaces;
            let tasks = if all { &view.all_tasks } else { &view.tasks };
            let sign = if all { icons::STACK } else { icons::SQUARE };
            let scope = egui::Button::new(
                egui::RichText::new(sign)
                    .size(14.0)
                    .color(if all { accent } else { text.gamma_multiply(0.6) }),
            )
            .fill(egui::Color32::TRANSPARENT)
            .corner_radius(8)
            .min_size(egui::vec2(22.0, tall));
            if ui
                .add(scope)
                .hint(if all {
                    "The windows of every workspace. A press shows this workspace's only."
                } else {
                    "The windows of this workspace. A press shows every workspace's."
                })
                .clicked()
            {
                asked.all_tasks = Some(!all);
            }
            // As wide as the setting says, or narrower when the buttons
            // of all the windows would not fit beside each other.
            let task_wide = if bar.task_width == 0 || tasks.is_empty() {
                0.0
            } else {
                let count = tasks.len() as f32;
                let room =
                    (ui.available_width() - ui.spacing().item_spacing.x * (count - 1.0)) / count;
                (bar.task_width as f32).min(room.max(36.0))
            };
            for task in tasks {
                // The one with the keyboard stands out, as the shown
                // workspace does among the tabs, more quietly.
                let has_keys = task.focused && task.shown && view.focused;
                // (One on a workspace that is not shown reads fainter.)
                let text = if task.shown { text } else { text.gamma_multiply(0.6) };
                let (fill, ink) = if has_keys {
                    (accent.gamma_multiply(0.28), text)
                } else {
                    (text.gamma_multiply(0.07), text.gamma_multiply(0.75))
                };
                // (Down a side: its first letter; the hint has the title.)
                let said = if upright {
                    task_label(task)
                        .chars()
                        .next()
                        .map(|letter| letter.to_uppercase().to_string())
                        .unwrap_or_default()
                } else {
                    task_label(task)
                };
                let label = egui::RichText::new(said).size(12.5).color(ink);
                // The window's picture, where it has one: before its
                // words, or in their place down a side.
                let picture = task_picture(ui.ctx(), &task.icon);
                let small = |picture: &egui::TextureHandle| {
                    egui::Image::new((picture.id(), egui::vec2(16.0, 16.0)))
                };
                let button = match &picture {
                    Some(picture) if upright => egui::Button::image(small(picture)),
                    Some(picture) => egui::Button::image_and_text(small(picture), label),
                    None => egui::Button::new(label),
                }
                .fill(fill)
                .corner_radius(8)
                .min_size(egui::vec2(0.0, tall));
                let response = if upright || task_wide <= 0.0 {
                    ui.add(button)
                } else {
                    // Every button the same width, whatever its title
                    // says: a title that changes moves no button.
                    // (Given its place and drawn there by hand: a button
                    // put into a place of a set size stood lower than
                    // the rest of the panel and was cut off below.)
                    let (place, response) =
                        ui.allocate_exact_size(egui::vec2(task_wide, tall), egui::Sense::click());
                    let fill = match (response.hovered(), has_keys) {
                        (true, true) => accent.gamma_multiply(0.38),
                        (true, false) => text.gamma_multiply(0.14),
                        (false, _) => fill,
                    };
                    ui.painter().rect_filled(place, 8.0, fill);
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        task_label(task),
                        egui::FontId::proportional(12.5),
                        ink,
                    );
                    // (With a picture the words begin after it; without
                    // one they stand in the middle, as they did.)
                    let before = if picture.is_some() { 22.0 } else { 0.0 };
                    job.wrap = egui::text::TextWrapping::truncate_at_width(
                        (task_wide - 16.0 - before).max(8.0),
                    );
                    let words = ui.fonts_mut(|fonts| fonts.layout_job(job));
                    let at = match &picture {
                        Some(picture) => {
                            let spot = egui::Rect::from_center_size(
                                egui::pos2(place.left() + 16.0, place.center().y),
                                egui::vec2(16.0, 16.0),
                            );
                            let whole = egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0),
                            );
                            ui.painter()
                                .image(picture.id(), spot, whole, egui::Color32::WHITE);
                            egui::pos2(
                                place.left() + 8.0 + before,
                                place.center().y - words.size().y / 2.0,
                            )
                        }
                        None => place.center() - words.size() / 2.0,
                    };
                    ui.painter().galley(at, words, ink);
                    response
                }
                .hint(&task.title);
                if response.clicked() && !has_keys {
                    asked.go = Some(task.id);
                }
            }
        }
        PanelModule::Keyboard => {
            if let Some(layout) = shown.keyboard {
                // Its short word; the hint has the whole name and what a
                // press does.
                let label = egui::RichText::new(layout.short.to_uppercase())
                    .size(12.5)
                    .color(text);
                let button = egui::Button::new(label)
                    .fill(text.gamma_multiply(0.10))
                    .corner_radius(8)
                    .min_size(egui::vec2(28.0, tall));
                let hint = if layout.count > 1 {
                    format!(
                        "Keyboard: {} ({} of {}). A press goes to the next layout.",
                        layout.name,
                        layout.index + 1,
                        layout.count
                    )
                } else {
                    format!(
                        "Keyboard: {}. More layouts are named in the settings (Input).",
                        layout.name
                    )
                };
                let response = ui.add(button).hint(hint);
                if layout.count > 1 {
                    if response.clicked() {
                        asked.keyboard = Some(true);
                    } else if response.secondary_clicked() {
                        asked.keyboard = Some(false);
                    }
                }
            }
        }
        PanelModule::Spacer => {
            ui.add_space(12.0);
        }
        // Nothing to show now, or not known to this version.
        PanelModule::Layout | PanelModule::Title | PanelModule::Other(_) => {}
    }
}

/// How large the menu of devices to play on is, in points.
pub fn sound_menu_size(outputs: usize) -> egui::Vec2 {
    egui::vec2(380.0, 14.0 + 32.0 * outputs.max(1) as f32)
}

/// The menu under a panel's sound: the devices to play on, the one in
/// use marked. The one that was pressed, if one was.
pub fn sound_menu(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    outputs: &[gevrek_system::sound::Output],
) -> Option<u32> {
    let mut chosen = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if outputs.is_empty() {
                ui.label(
                    egui::RichText::new("No device to play on is known.").color(palette.muted),
                );
            }
            let (on, chose) = menu_keys(ui.ctx(), outputs.len(), |_| true, false);
            for (index, output) in outputs.iter().enumerate() {
                let name: String = output.name.chars().take(48).collect();
                let label = egui::RichText::new(name).size(13.0);
                let button = egui::Button::selectable(output.chosen, label)
                    .min_size(egui::vec2(ui.available_width(), 28.0));
                let response = ui.add(button);
                if on == Some(index) {
                    menu_mark(ui, response.rect, palette);
                }
                if (response.clicked() || (chose && on == Some(index))) && !output.chosen {
                    chosen = Some(output.id);
                }
            }
        });
    chosen
}

/// How large the menu of layouts is, in points.
pub fn layout_menu_size() -> egui::Vec2 {
    let layouts = gevrek_model::layout::LayoutType::all();
    // A line of the menu is 30 points high and two apart from the next.
    egui::vec2(220.0, 14.0 + 32.0 * layouts.len() as f32)
}

/// The menu under a panel's layout: every layout, the one in use marked.
/// The layout that was pressed, if one was.
pub fn layout_menu(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    current: &str,
) -> Option<gevrek_model::layout::LayoutType> {
    let mut chosen = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let layouts = gevrek_model::layout::LayoutType::all();
            let (on, chose) = menu_keys(ui.ctx(), layouts.len(), |_| true, false);
            for (index, layout) in layouts.into_iter().enumerate() {
                let name = layout_name(layout);
                let label = egui::RichText::new(layout_label(&name)).size(13.0);
                let button = egui::Button::selectable(name == current, label)
                    .min_size(egui::vec2(ui.available_width(), 28.0));
                let response = ui.add(button);
                if on == Some(index) {
                    menu_mark(ui, response.rect, palette);
                }
                if response.clicked() || (chose && on == Some(index)) {
                    chosen = Some(layout);
                }
            }
        });
    chosen
}

/// How large the list of notices is, in points: wide enough to read a
/// sentence in, and as high as what there is to read asks for, roughly
/// (it is scrolled when that was too little).
pub fn notices_size(notices: &[Notice]) -> egui::Vec2 {
    const WIDTH: f32 = 420.0;
    // About sixty letters to a line of this width.
    let lines: usize = notices
        .iter()
        .map(|notice| 1 + notice.text.chars().count().div_ceil(60).max(1))
        .sum();
    egui::vec2(
        WIDTH,
        (20.0 + 19.0 * lines as f32 + 10.0 * notices.len() as f32).clamp(60.0, 420.0),
    )
}

/// What the window manager has to tell, in full.
pub fn notices_list(ui: &mut egui::Ui, palette: &theme::Palette, notices: &[Notice]) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(10)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if notices.is_empty() {
                        ui.label(egui::RichText::new("Nothing to tell.").color(palette.muted));
                    }
                    for notice in notices {
                        ui.label(
                            egui::RichText::new(&notice.title)
                                .size(13.5)
                                .strong()
                                .color(palette.text),
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&notice.text)
                                    .size(13.0)
                                    .color(palette.muted),
                            )
                            .wrap(),
                        );
                        ui.add_space(8.0);
                    }
                });
        });
}

/// The time as a format of the `strftime` kind writes it. A format that
/// is not one shows as it is written.
fn clock_text(format: &str, now: &jiff::Zoned) -> String {
    // The signs that stand for several others are written out, as the
    // clock has always written them.
    let mut whole = String::with_capacity(format.len());
    let mut signs = format.chars();
    while let Some(sign) = signs.next() {
        if sign != '%' {
            whole.push(sign);
            continue;
        }
        match signs.next() {
            Some('c') => whole.push_str("%a %b %e %H:%M:%S %Y"),
            Some('x') => whole.push_str("%m/%d/%y"),
            Some('X') => whole.push_str("%H:%M:%S"),
            Some('v') => whole.push_str("%e-%b-%Y"),
            Some('+') => whole.push_str("%Y-%m-%dT%H:%M:%S%:z"),
            Some('Z') => whole.push_str("%:z"),
            Some(other) => {
                whole.push('%');
                whole.push(other);
            }
            None => whole.push('%'),
        }
    }
    let mut said = String::new();
    if std::fmt::Write::write_fmt(&mut said, format_args!("{}", now.strftime(&whole))).is_err() {
        said = format.to_string();
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a person wrote as the clock's format reads as it did.
    #[test]
    fn the_clock_writes_the_time_as_its_format_says() {
        let now = jiff::civil::date(2026, 11, 18)
            .at(9, 5, 7, 0)
            .in_tz("UTC")
            .expect("a time");
        for (format, said) in [
            ("%a %d %b  %H:%M", "Wed 18 Nov  09:05"),
            ("%H:%M", "09:05"),
            ("%H:%M:%S", "09:05:07"),
            ("%T", "09:05:07"),
            ("%R", "09:05"),
            ("%I:%M %p", "09:05 AM"),
            ("%l:%M %P", " 9:05 am"),
            ("%-H.%M", "9.05"),
            ("%A, %-d %B %Y", "Wednesday, 18 November 2026"),
            ("%e %h %y", "18 Nov 26"),
            ("%F", "2026-11-18"),
            ("%D", "11/18/26"),
            ("%d.%m.%Y", "18.11.2026"),
            ("%j", "322"),
            ("%u %w", "3 3"),
            ("%c", "Wed Nov 18 09:05:07 2026"),
            ("%x", "11/18/26"),
            ("%X", "09:05:07"),
            ("%s", "1794992707"),
            ("%v", "18-Nov-2026"),
            ("%+", "2026-11-18T09:05:07+00:00"),
            ("%Z", "+00:00"),
            ("%H:%M:%S%.3f", "09:05:07.000"),
            ("%U %W %V %G %C", "46 46 47 2026 20"),
            ("%%c", "%c"),
            ("%z", "+0000"),
            ("%:z", "+00:00"),
            ("100%%", "100%"),
            ("now", "now"),
            // What is no format shows as it is written.
            ("%!", "%!"),
        ] {
            assert_eq!(clock_text(format, &now), said, "{format}");
        }
    }

    #[test]
    fn the_taskbar_has_the_windows_of_the_shown_workspace() {
        let window = |id: u64, title: &str, focused: bool| serde_json::json!({"window_id": id, "title": title, "class": "app", "is_focused": focused});
        let monitor = serde_json::json!({
            "is_focused": true, "workspace": {"id": 2}, "workspaces": [
                {"id": 1, "name": "1", "detailed_windows": [window(9, "elsewhere", true)]},
                {"id": 2, "name": "2", "detailed_windows": [
                    window(1, "editor", false),
                    window(2, "", true),
                    {"window_id": 3, "title": "border", "is_border_window": true},
                ]},
            ]
        });
        let view = view_of(&monitor);
        let ids: Vec<u64> = view.tasks.iter().map(|task| task.id).collect();
        assert_eq!(ids, [1, 2]);
        assert!(view.tasks[1].focused && !view.tasks[0].focused);
        // A window without a title is known by its program.
        assert_eq!(task_label(&view.tasks[1]), "app");
        let long = Task {
            id: 4,
            title: "A very long title of a window that goes on".to_string(),
            class: String::new(),
            icon: String::new(),
            focused: false,
            shown: true,
        };
        let label = task_label(&long);
        // (Cut at 24 letters; a space the cut ends on is left out.)
        assert_eq!(label, "A very long title of a…");
        assert!(label.ends_with('…') && label.starts_with("A very long title"));
    }

    #[test]
    fn the_wallpapers_accent_is_read_from_the_state() {
        assert_eq!(
            accent(&serde_json::json!({"accent_rgb": 0xE63939})),
            Some((230, 57, 57))
        );
        // The configuration's own accent holds: nothing is named.
        assert_eq!(accent(&serde_json::json!({"accent_rgb": null})), None);
        assert_eq!(accent(&serde_json::json!({})), None);
    }
    use serde_json::json;

    fn state() -> Value {
        json!({"monitors": [
            {"name": "gevrek-1", "connector": "DP-1", "is_focused": true, "workspace": {"id": 2},
             "workspaces": [
                {"id": 1, "name": "1", "layout_type": "Dwindle", "detailed_windows": [{"title": "mail", "is_focused": false}]},
                {"id": 2, "name": "2", "layout_type": "MasterStack", "detailed_windows": [
                    {"title": "editor", "is_focused": false}, {"title": "terminal", "is_focused": true}]},
                {"id": 3, "name": "3", "layout_type": "Grid", "detailed_windows": []}]},
            {"name": "HEADLESS-2", "connector": "", "is_focused": false, "workspace": {"id": 9},
             "workspaces": [{"id": 9, "name": "9", "layout_type": "Columns", "detailed_windows": []}]},
        ]})
    }

    /// The state as the window manager gives it on Windows: displays by
    /// the name of their model, the panel's rectangle with each.
    fn windows_state() -> Value {
        json!({
            "active_binding_submap": "default",
            "notices": [{"id": "a", "title": "Shift and drag", "text": "FancyZones has it too."}],
            "monitors": [
                {"name": "Monitor ACM2702 (1)", "connector": "", "is_focused": true,
                 "position": {"x": 0.0, "y": 0.0}, "size": {"width": 2560, "height": 1440},
                 "bar_position": {"x": 6.0, "y": 6.0}, "bar_size": {"width": 2548, "height": 36},
                 "workspace": {"id": 1},
                 "workspaces": [
                    {"id": 1, "name": "Workspace 1", "layout_type": "BSP", "tiling_enabled": true,
                     "total_windows": 2, "detailed_windows": [],
                     "windows": [{"title": "notes", "is_focused": true}]},
                    {"id": 2, "name": "mail", "layout_type": "BSP", "tiling_enabled": true,
                     "total_windows": 3, "detailed_windows": []}]},
                {"name": "Monitor ACM2702 (2)", "connector": "", "is_focused": false,
                 "position": {"x": 2560.0, "y": 0.0}, "size": {"width": 2560, "height": 1440},
                 "bar_position": {"x": 2566.0, "y": 6.0}, "bar_size": {"width": 2548, "height": 36},
                 "workspace": {"id": 7},
                 "workspaces": [{"id": 7, "name": "", "layout_type": "Grid", "tiling_enabled": false,
                                 "total_windows": 0, "detailed_windows": []}]},
            ]
        })
    }

    #[test]
    fn a_displays_part_of_the_state_is_what_its_panel_shows() {
        let first = view(&state(), "DP-1").expect("the first display");
        let names: Vec<(&str, bool, usize)> = first
            .tabs
            .iter()
            .map(|tab| (tab.name.as_str(), tab.shown, tab.windows))
            .collect();
        assert_eq!(names, [("1", false, 1), ("2", true, 2), ("3", false, 0)]);
        assert_eq!(first.layout, "MasterStack");
        assert_eq!(first.title, "terminal");
        assert!(first.focused);
        assert!(!first.floating);
        // A display known by its name only (one without a connector).
        let second = view(&state(), "HEADLESS-2").expect("the second display");
        assert_eq!(second.tabs.len(), 1);
        assert_eq!(second.title, "");
        assert!(!second.focused);
        assert!(view(&state(), "HDMI-A-1").is_none());
    }

    #[test]
    fn a_workspace_is_shown_by_its_name_or_by_its_number() {
        assert_eq!(tab_label("Workspace", 0), "1");
        assert_eq!(tab_label("", 2), "3");
        assert_eq!(tab_label("Workspace 4", 3), "4");
        assert_eq!(tab_label("code", 1), "code");
        assert_eq!(tab_label("Workspace four", 3), "Workspace four");
        assert_eq!(tab_label("2", 1), "2");
    }

    #[test]
    fn a_notice_is_told_once_when_it_arises_and_again_when_it_comes_back() {
        let notice = |id: &str| Notice {
            id: id.to_string(),
            title: format!("about {id}"),
            text: String::new(),
        };
        let mut told = std::collections::HashSet::new();
        let first = arisen(&[notice("keys")], &mut told);
        assert_eq!(first, [notice("keys")]);
        // The state is asked for again and says the same: nothing new.
        assert!(arisen(&[notice("keys")], &mut told).is_empty());
        // Another arises beside it.
        assert_eq!(
            arisen(&[notice("keys"), notice("zones")], &mut told),
            [notice("zones")]
        );
        // The first went; later it is there again.
        assert!(arisen(&[notice("zones")], &mut told).is_empty());
        assert_eq!(
            arisen(&[notice("zones"), notice("keys")], &mut told),
            [notice("keys")]
        );
        assert!(arisen(&[], &mut told).is_empty() && told.is_empty());
    }

    #[test]
    fn the_state_on_windows_is_read_as_the_bar_before_read_it() {
        let state = windows_state();
        let first = view(&state, "Monitor ACM2702 (1)").expect("the first display");
        let tabs: Vec<(&str, usize)> = first
            .tabs
            .iter()
            .map(|tab| (tab.label.as_str(), tab.windows))
            .collect();
        // Windows are counted where they are not listed one by one.
        assert_eq!(tabs, [("1", 2), ("mail", 3)]);
        // The title, where only the short list of windows has it.
        assert_eq!(first.title, "notes");
        let second = view(&state, "Monitor ACM2702 (2)").expect("the second display");
        assert_eq!(second.tabs[0].label, "1");
        assert!(second.floating);
        assert_eq!(notices(&state).len(), 1);
        assert_eq!(notices(&state)[0].title, "Shift and drag");
        assert_eq!(notices(&state)[0].id, "a");
        assert_eq!(mode(&state), "");
        assert_eq!(mode(&json!({"active_binding_submap": "resize"})), "resize");
        assert_eq!(notices_label(1), "1 notice");
        assert_eq!(notices_label(3), "3 notices");
    }

    #[test]
    fn a_panel_is_where_the_window_manager_keeps_room_for_it_on_each_display() {
        let found = places(&windows_state());
        assert_eq!(
            found,
            [
                ("Monitor ACM2702 (1)".to_string(), (6, 6, 2548, 36)),
                ("Monitor ACM2702 (2)".to_string(), (2566, 6, 2548, 36)),
            ]
        );
        // No panel in the configuration: the state has no rectangle, and
        // no panel is wanted. (The first state has none.)
        assert!(places(&state()).is_empty());
        // Two displays that came with one name are still two panels, and
        // one without a name has one too.
        let twins = json!({"monitors": [
            {"name": "Monitor X", "bar_position": {"x": 0.0, "y": 0.0}, "bar_size": {"width": 800, "height": 30}},
            {"name": "Monitor X", "bar_position": {"x": 800.0, "y": 0.0}, "bar_size": {"width": 800, "height": 30}},
            {"name": "", "bar_position": {"x": 1600.0, "y": 0.0}, "bar_size": {"width": 800, "height": 30}},
            {"name": "Monitor Y", "bar_position": {"x": 0.0, "y": 0.0}, "bar_size": {"width": 0, "height": 30}},
        ]});
        let names: Vec<String> = places(&twins).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["Monitor X", "Monitor X #1", "#2"]);
    }

    #[test]
    fn the_panels_that_are_open_are_brought_in_line_with_the_ones_wanted() {
        let at = |name: &str, x: i32| (name.to_string(), (x, 6, 800, 36));
        // Nothing open, two wanted.
        let first = plan(&[], &[at("one", 0), at("two", 800)]);
        assert_eq!(first.open, [at("one", 0), at("two", 800)]);
        assert!(first.close.is_empty() && first.put.is_empty());
        // Nothing changed: nothing to do.
        assert_eq!(
            plan(
                &[at("one", 0), at("two", 800)],
                &[at("one", 0), at("two", 800)]
            ),
            Plan::default()
        );
        // A display went, another moved, a third came.
        let later = plan(
            &[at("one", 0), at("two", 800)],
            &[at("two", 0), at("three", 800)],
        );
        assert_eq!(later.close, ["one"]);
        assert_eq!(later.put, [at("two", 0)]);
        assert_eq!(later.open, [at("three", 800)]);
        // Switched off: all go.
        assert_eq!(plan(&[at("one", 0)], &[]).close, ["one"]);
    }

    #[test]
    fn the_panel_asks_for_the_state_after_what_changes_it_and_not_after_the_rest() {
        for kind in [
            "workspace.switched",
            "workspace.renamed",
            "layout.changed",
            "output.added",
            "window.created",
            "window.destroyed",
            "window.focused",
            "window.metadata_changed",
            "dispatch.succeeded",
            "configuration.reloaded",
        ] {
            assert!(follows(kind), "{kind}");
        }
        // These come many times a second while a window is dragged or a
        // gesture is made, and change nothing a panel shows.
        for kind in [
            "window.moved",
            "window.resized",
            "window.drag_started",
            "gesture.updated",
            "binding.triggered",
            "shell.command",
            "dispatch.failed",
            "runtime.stopping",
        ] {
            assert!(!follows(kind), "{kind}");
        }
    }

    #[test]
    fn a_window_the_state_never_had_is_no_reason_to_ask_when_it_goes() {
        let state = json!({"monitors": [{"workspaces": [
            {"windows": [{"window_id": 7}], "detailed_windows": [{"window_id": 8}]},
            {"windows": [], "detailed_windows": [{"window_id": 9}]},
        ]}]});
        let known = windows(&state);
        assert_eq!(known.len(), 3);
        // A tooltip of some program closed: not among the windows.
        assert!(!concerns("window.destroyed", Some(1234), &known));
        assert!(!concerns("window.metadata_changed", Some(1234), &known));
        assert!(!concerns("window.unmapped", Some(1234), &known));
        // One of the windows closed, or has another title.
        assert!(concerns("window.destroyed", Some(8), &known));
        assert!(concerns("window.metadata_changed", Some(7), &known));
        // Not said which window: asked, to be sure.
        assert!(concerns("window.destroyed", None, &known));
        // A window that is new cannot be known yet, and the focus may have
        // left the windows the state has.
        assert!(concerns("window.created", Some(1234), &known));
        assert!(concerns("window.focused", Some(1234), &known));
        assert!(concerns("workspace.switched", None, &known));
        // What changes nothing on a panel stays so.
        assert!(!concerns("window.moved", Some(8), &known));
    }

    #[test]
    fn a_panel_is_drawn_again_only_when_what_it_shows_changed() {
        // What a panel shows is compared as a whole: a state that says the
        // same again (after a window was only moved) is no reason to draw.
        let before = view(&windows_state(), "Monitor ACM2702 (1)");
        let mut state = windows_state();
        state["monitors"][0]["workspaces"][0]["windows"][0]["position"] = json!({"x": 5.0});
        assert_eq!(view(&state, "Monitor ACM2702 (1)"), before);
        state["monitors"][0]["workspaces"][0]["windows"][0]["title"] = json!("other");
        assert_ne!(view(&state, "Monitor ACM2702 (1)"), before);
    }

    #[test]
    fn a_wheel_turned_fast_counts_on_from_where_it_was_heading() {
        // Three workspaces, the second shown; the panel is not told of
        // anything while the wheel turns.
        let view = view(&state(), "DP-1").expect("the first output");
        assert_eq!(view.tabs.iter().position(|tab| tab.shown), Some(1));
        let start = std::time::Instant::now();
        let after = |ms| start + std::time::Duration::from_millis(ms);
        let mut wheel = Wheel::default();
        assert_eq!(wheel.turn(&view, -1.0, after(0)), Some(2));
        assert_eq!(wheel.turn(&view, -1.0, after(30)), Some(0), "not 2 again");
        // Back to the one that is still shown here: asked for, since the
        // window manager is on its way elsewhere.
        assert_eq!(wheel.turn(&view, -1.0, after(60)), Some(1));
        // Half a notch is nothing yet.
        assert_eq!(wheel.turn(&view, -0.5, after(90)), None);
        // Long after, with the panel still showing the second (the switch
        // did not happen): counted from what is shown.
        assert_eq!(wheel.turn(&view, 1.5, after(5000)), Some(0));
        // One workspace: nowhere to go.
        let lone = super::view(&state(), "HEADLESS-2").expect("the second display");
        assert_eq!(Wheel::default().turn(&lone, 1.0, after(0)), None);
    }

    #[test]
    fn the_wheel_goes_through_the_workspaces_a_notch_at_a_time() {
        let first = view(&state(), "DP-1").expect("the first display");
        // The second of three is shown.
        assert_eq!(wheel_to(&first, 1), Some(2));
        assert_eq!(wheel_to(&first, -1), Some(0));
        // Round the end, as the window manager's own next goes.
        assert_eq!(wheel_to(&first, 2), Some(0));
        assert_eq!(wheel_to(&first, -2), Some(2));
        assert_eq!(wheel_to(&first, 3), None, "once round is where it was");
        assert_eq!(wheel_to(&first, 0), None);
        let lone = view(&state(), "HEADLESS-2").expect("the second display");
        assert_eq!(wheel_to(&lone, 1), None);
        assert_eq!(wheel_to(&View::default(), 1), None);

        // A notch towards the user is the next workspace.
        let mut turned = -1.0;
        assert_eq!(steps(&mut turned), 1);
        assert_eq!(turned, 0.0);
        // Parts of a notch (a touchpad) add up.
        let mut turned = 0.0;
        let mut went = 0;
        for _ in 0..5 {
            turned += notches(egui::MouseWheelUnit::Point, 10.0);
            went += steps(&mut turned);
        }
        assert_eq!(went, -1);
        assert_eq!(notches(egui::MouseWheelUnit::Line, 2.0), 2.0);
    }

    #[test]
    fn a_module_this_system_cannot_do_is_left_out_and_named() {
        assert!(works_here(&PanelModule::Workspaces));
        assert!(works_here(&PanelModule::Layout));
        assert!(works_here(&PanelModule::Title));
        assert!(works_here(&PanelModule::Clock));
        assert!(works_here(&PanelModule::Stats));
        assert!(works_here(&PanelModule::Spacer));
        assert!(!works_here(&PanelModule::Other("weather".to_string())));
        let mut bar = PanelProfile::default();
        bar.left.insert(0, PanelModule::Launcher);
        bar.right.push(PanelModule::Power);
        bar.right.push(PanelModule::Power);
        bar.centre.push(PanelModule::Other("weather".to_string()));
        let out = left_out(&bar);
        assert!(out.contains(&PanelModule::Other("weather".to_string())));
        // The launcher is there on every system.
        assert!(works_here(&PanelModule::Launcher));
        assert!(!out.contains(&PanelModule::Launcher));
        // The notifications and the power menu are the shell's own
        // surfaces, on every system.
        assert!(works_here(&PanelModule::Notifications));
        assert!(works_here(&PanelModule::Power));
        assert_eq!(out.len(), 1);
        // One that is not known is named once, however often it stands.
        bar.left.push(PanelModule::Other("weather".to_string()));
        assert_eq!(left_out(&bar).len(), 1);
    }

    #[test]
    fn a_surface_in_the_corner_keeps_clear_of_a_panel_at_the_top() {
        let found = rooms(&windows_state());
        assert_eq!(found.len(), 2);
        // No room for windows is told there: all of the display counts.
        assert_eq!(found[0].work, (0, 0, 2560, 1440));
        assert_eq!(found[0].bar, Some((6, 6, 2548, 36)));
        assert_eq!(found[0].scale, 1.0);
        assert!(found[0].focused && !found[1].focused);
        // Under the panel, at the right.
        assert_eq!(corner(&found[0], (380, 100), 12), (2168, 54, 380, 100));
        assert_eq!(corner(&found[1], (380, 100), 12), (4728, 54, 380, 100));
        // The room for windows where the state tells it (a taskbar at the
        // bottom), a panel at the bottom, and a scale.
        let state = json!({"monitors": [{
            "position": {"x": 0.0, "y": 0.0}, "size": {"width": 1920, "height": 1080},
            "work_position": {"x": 0.0, "y": 0.0}, "work_size": {"width": 1920, "height": 1032},
            "bar_position": {"x": 0.0, "y": 996.0}, "bar_size": {"width": 1920, "height": 36},
            "scale": 1.5,
        }]});
        let room = &rooms(&state)[0];
        assert_eq!(room.work, (0, 0, 1920, 1032));
        assert_eq!(room.scale, 1.5);
        assert_eq!(corner(room, (570, 150), 18), (1332, 18, 570, 150));
        // No panel: at the top of the room. Never more than there is.
        let bare = Room {
            work: (0, 0, 800, 600),
            bar: None,
            scale: 1.0,
            focused: true,
        };
        assert_eq!(corner(&bare, (380, 100), 12), (408, 12, 380, 100));
        // Any corner of the room.
        use gevrek_model::settings::NotificationCorner as Corner;
        assert_eq!(
            corner_at(&bare, (380, 100), 12, Corner::TopLeft),
            (12, 12, 380, 100)
        );
        assert_eq!(
            corner_at(&bare, (380, 100), 12, Corner::BottomRight),
            (408, 488, 380, 100)
        );
        assert_eq!(
            corner_at(&bare, (380, 100), 12, Corner::BottomLeft),
            (12, 488, 380, 100)
        );
        // Over a panel at the bottom, under one at the top.
        let low = Room {
            bar: Some((6, 558, 788, 36)),
            ..bare.clone()
        };
        assert_eq!(
            corner_at(&low, (380, 100), 12, Corner::BottomRight),
            (408, 446, 380, 100)
        );
        assert_eq!(
            corner_at(&low, (380, 100), 12, Corner::TopRight),
            (408, 12, 380, 100)
        );
        assert_eq!(corner(&bare, (380, 5000), 12), (408, 12, 380, 576));
        assert_eq!(corner(&bare, (5000, 100), 12).2, 776);
        assert!(rooms(&json!({"monitors": [{"name": "x"}]})).is_empty());
    }

    #[test]
    fn a_tab_names_its_workspace_and_its_display_whatever_it_is_shown_by() {
        use gevrek_model::command::{DispatchCommand, MonitorTarget, WorkspaceSelector};
        let state = json!({"monitors": [
            {"name": "Left", "connector": "DP-1", "is_primary": true, "workspace": {"id": 4}, "workspaces": [
                {"id": 4, "name": "1", "title": "Mail", "configured": true},
                {"id": 5, "name": "2", "title": "2", "configured": true},
                {"id": 9, "name": "10", "title": "10"}
            ]},
            {"name": "Right", "connector": "DP-2", "workspace": {"id": 6}, "workspaces": [
                {"id": 6, "name": "1", "title": "1", "configured": true},
                // (A window manager that tells no title: the name.)
                {"id": 7, "name": "Workspace 2"}
            ]}
        ]});
        let left = view(&state, "DP-1").expect("the left display");
        let right = view(&state, "DP-2").expect("the right display");
        let labels = |view: &View| -> Vec<String> {
            view.tabs.iter().map(|tab| tab.label.clone()).collect()
        };
        assert_eq!(labels(&left), ["Mail", "2", "10"]);
        assert_eq!(labels(&right), ["1", "2"]);
        assert_eq!(left.tabs[0].name, "1");
        assert!(left.tabs[0].configured && !left.tabs[2].configured);
        assert_eq!((left.place.name(), left.place.index, left.place.primary), ("DP-1", Some(0), true));
        assert!(right.place.is_called("right") && right.place.index == Some(1));

        // Shown by the display's name and the workspace's, not by a place
        // among the tabs that may be another by now.
        let shown: DispatchCommand =
            serde_json::from_str(&show_tab(&left, 0).expect("a tab")).expect("a command");
        assert_eq!(
            shown,
            DispatchCommand::FocusMonitorWorkspace {
                target: MonitorTarget::Name("DP-1".to_string()),
                workspace: WorkspaceSelector::Name("1".to_string()),
            }
        );
        // A tab that is not there asks for nothing (and makes nothing).
        assert!(show_tab(&left, 3).is_none());
    }

    #[test]
    fn a_new_name_for_a_tab_is_a_title_on_its_display_alone() {
        use gevrek_model::settings::DisplayRef;
        let config = gevrek_model::examples::starting_configuration();
        let (left, right) = (DisplayRef::named("DP-1"), DisplayRef::named("DP-2"));
        let label = |config: &gevrek_model::Config, display: &DisplayRef, name: &str| {
            config
                .desktop
                .workspace_on(display, name)
                .map(|profile| profile.label().to_string())
        };
        let Renaming::Apply(first) = renaming(Some(&config), &left, "1", " Mail ") else {
            panic!("a title is written down");
        };
        assert_eq!(label(&first, &left, "1").as_deref(), Some("Mail"));
        assert_eq!(label(&first, &right, "1").as_deref(), Some("1"));
        // As often as one likes, each on what the one before left.
        let Renaming::Apply(second) = renaming(Some(&first), &left, "1", "Post") else {
            panic!("a second title is written down");
        };
        assert_eq!(label(&second, &left, "1").as_deref(), Some("Post"));
        assert_eq!(second.desktop.workspaces.len(), first.desktop.workspaces.len());
        assert!(second.desktop.validate().is_empty());
        // The keys are what they were.
        assert_eq!(
            serde_json::to_value(&second.desktop.bindings).unwrap(),
            serde_json::to_value(&config.desktop.bindings).unwrap()
        );

        assert!(matches!(renaming(Some(&second), &left, "1", "Post"), Renaming::Nothing));
        // What another workspace of the display is called: said, not done.
        assert!(matches!(renaming(Some(&second), &left, "2", "post"), Renaming::Refused(_)));
        assert!(matches!(renaming(Some(&second), &left, "2", "3"), Renaming::Refused(_)));
        // On the other display the title is free.
        assert!(matches!(renaming(Some(&second), &right, "2", "Post"), Renaming::Apply(_)));
        assert!(matches!(renaming(None, &left, "1", "x"), Renaming::Refused(_)));

        // A workspace of the moment is written down for its display, so
        // that the name it was given lasts.
        let Renaming::Apply(kept) = renaming(Some(&second), &left, "12", "Music") else {
            panic!("a workspace of the moment is kept");
        };
        assert_eq!(label(&kept, &left, "12").as_deref(), Some("Music"));
        assert_eq!(label(&kept, &right, "12"), None);
        assert!(kept.desktop.validate().is_empty());
        assert!(matches!(renaming(Some(&second), &left, "12", "Post"), Renaming::Refused(_)));
    }

    #[test]
    fn a_display_has_the_panel_the_configuration_gives_it() {
        use gevrek_model::settings::{DisplayRef, PanelOnDisplay};
        let mut bar = PanelProfile::default();
        let common = bar.left.clone();
        bar.displays = vec![PanelOnDisplay {
            name: "DP-2".to_string(),
            left: Some(vec![PanelModule::Clock]),
            ..PanelOnDisplay::default()
        }];
        // A display the window manager calls one thing and whose
        // connector is another: the entry by its connector is its.
        let second = DisplayRef {
            names: vec!["Monitor B".to_string(), "DP-2".to_string()],
            index: Some(1),
            primary: false,
        };
        let own = bar_on(&bar, "Monitor B", &second).expect("a panel");
        assert_eq!(own.left, vec![PanelModule::Clock]);
        assert_eq!(own.right, bar.right);
        // Another display has the common panel.
        let first = DisplayRef::named("Monitor A");
        assert_eq!(bar_on(&bar, "Monitor A", &first).expect("a panel").left, common);
        // Switched off for that display: none there.
        bar.displays[0].enabled = false;
        assert!(bar_on(&bar, "Monitor B", &second).is_none());
        assert!(bar_on(&bar, "Monitor A", &first).is_some());
    }

    #[test]
    fn a_workspace_of_a_display_is_shown_by_a_command_the_model_reads() {
        use gevrek_model::command::{DispatchCommand, MonitorTarget, WorkspaceSelector};
        let here: DispatchCommand =
serde_json::from_str(&show_workspace_at("Monitor X", true, 2, "")).expect("a command");
        assert_eq!(
            here,
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Index(2)
            }
        );
        // On a display without the keyboard, the display is named.
        let there: DispatchCommand =
            serde_json::from_str(&show_workspace_at("Monitor X", false, 2, " ")).expect("a command");
        assert_eq!(
            there,
            DispatchCommand::FocusMonitorWorkspace {
                target: MonitorTarget::Name("Monitor X".to_string()),
                workspace: WorkspaceSelector::Index(2)
            }
        );
        // One that has a name is shown by it, on its display, wherever
        // among the display's workspaces it is by then.
        for focused in [true, false] {
            let named: DispatchCommand =
                serde_json::from_str(&show_workspace_at("Monitor X", focused, 2, "mail"))
                    .expect("a command");
            assert_eq!(
                named,
                DispatchCommand::FocusMonitorWorkspace {
                    target: MonitorTarget::Name("Monitor X".to_string()),
                    workspace: WorkspaceSelector::Name("mail".to_string())
                }
            );
        }
    }

    #[test]
    fn a_layout_is_named_as_the_state_names_it_and_read_in_words() {
        use gevrek_model::layout::LayoutType;
        assert_eq!(layout_name(LayoutType::MasterStack), "MasterStack");
        assert_eq!(layout_label("MasterStack"), "Master stack");
        assert_eq!(
            layout_label("UltrawideVerticalStack"),
            "Ultrawide vertical stack"
        );
        assert_eq!(layout_label("BSP"), "BSP");
        let command: gevrek_model::command::DispatchCommand =
            serde_json::from_str(&set_layout(LayoutType::Grid)).unwrap();
        assert!(matches!(
            command,
            gevrek_model::command::DispatchCommand::SetLayout {
                layout: LayoutType::Grid
            }
        ));
    }

    #[test]
    fn the_command_of_a_tab_is_one_the_model_reads() {
        let command: gevrek_model::command::DispatchCommand =
            serde_json::from_str(&show_workspace("mail")).expect("a command");
        assert!(!matches!(
            command,
            gevrek_model::command::DispatchCommand::Unknown(_)
        ));
    }

    #[test]
    fn a_hint_comes_when_the_pointer_has_rested_and_goes_with_it() {
        let start = std::time::Instant::now();
        let at = |ms| start + std::time::Duration::from_millis(ms);
        let mut hints: Hints<u32> = Hints::default();
        assert_eq!(hints.step(None, start), HintStep::Nothing);
        assert_eq!(hints.step(Some("Power"), start), HintStep::Wait(HINT_AFTER));
        assert!(matches!(
            hints.step(Some("Power"), at(200)),
            HintStep::Wait(_)
        ));
        assert_eq!(hints.step(Some("Power"), at(460)), HintStep::Open(None));
        hints.shown = Some((7, "Power".to_string()));
        assert_eq!(hints.step(Some("Power"), at(900)), HintStep::Nothing);
        // On to the next thing: its hint at once, the old one away.
        assert_eq!(
            hints.step(Some("Launcher"), at(950)),
            HintStep::Open(Some(7))
        );
        hints.shown = Some((8, "Launcher".to_string()));
        // The pointer left: away, and the next one waits again.
        assert_eq!(hints.step(None, at(1000)), HintStep::Close(8));
        assert_eq!(hints.step(None, at(1010)), HintStep::Nothing);
        assert!(matches!(
            hints.step(Some("Launcher"), at(1020)),
            HintStep::Wait(_)
        ));
        // Moving on before it came starts the wait again.
        assert!(matches!(
            hints.step(Some("Power"), at(1400)),
            HintStep::Wait(_)
        ));
        assert!(matches!(
            hints.step(Some("Power"), at(1700)),
            HintStep::Wait(_)
        ));
        assert_eq!(hints.step(Some("Power"), at(1860)), HintStep::Open(None));
    }

    #[test]
    fn a_panel_narrower_than_its_display_stands_where_it_is_told() {
        let whole = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 36.0));
        let strip = strip_of(whole, 400.0, PanelAlign::Start, false);
        assert_eq!((strip.left(), strip.right()), (0.0, 400.0));
        let strip = strip_of(whole, 400.0, PanelAlign::Centre, false);
        assert_eq!((strip.left(), strip.right()), (300.0, 700.0));
        let strip = strip_of(whole, 400.0, PanelAlign::End, false);
        assert_eq!((strip.left(), strip.right()), (600.0, 1000.0));
        // Never wider than the display.
        assert_eq!(strip_of(whole, 5000.0, PanelAlign::Centre, false), whole);
        // Down a side: from the top.
        let side = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(40.0, 1000.0));
        let strip = strip_of(side, 300.0, PanelAlign::End, true);
        assert_eq!(
            (strip.top(), strip.bottom(), strip.width()),
            (700.0, 1000.0, 40.0)
        );
    }

    #[test]
    fn the_surface_follows_the_bars_edge_height_and_margin() {
        let mut bar = PanelProfile::default();
        let before = geometry(&bar);
        bar.right.clear();
        assert_eq!(
            geometry(&bar),
            before,
            "what it shows does not move the surface"
        );
        bar.height += 6;
        assert_ne!(geometry(&bar), before);
    }

    #[test]
    fn a_panel_is_drawn_with_what_it_shows_and_asks_for_nothing_by_itself() {
        // Drawn without a screen: every module's code runs.
        let bar = PanelProfile {
            left: vec![
                PanelModule::Launcher,
                PanelModule::Workspaces,
                PanelModule::Layout,
                PanelModule::Spacer,
                PanelModule::Stats,
            ],
            centre: vec![PanelModule::Title],
            right: vec![
                PanelModule::Notifications,
                PanelModule::Clock,
                PanelModule::Power,
                PanelModule::Sound,
                PanelModule::Battery,
                PanelModule::Network,
                PanelModule::Other("weather".to_string()),
            ],
            ..Default::default()
        };
        let view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        let context = egui::Context::default();
        let mut wheel = Wheel::default();
        // (With the sound, a battery and a network to show: the modules
        // for them are drawn too when the panel has them.)
        let system = SystemView {
            volume: Some((40, false)),
            battery: Some(gevrek_system::status::Battery {
                percent: 12,
                charging: false,
            }),
            network: Some(gevrek_system::status::Network {
                connected: true,
                wireless: true,
                name: "Home".to_string(),
            }),
            brightness: Some(60),
        };
        let mut frame = |events: Vec<egui::Event>, notices: usize, mode: &str| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 36.0),
                )),
                events,
                ..Default::default()
            };
            let mut asked = Asked::default();
            let mut output = context.run_ui(input, |ui| {
                asked = draw(
                    ui,
                    &Shown {
                        bar: &bar,
                        view: &view,
                        accent: egui::Color32::from_rgb(64, 224, 208),
                        quiet: false,
                        notices,
                        mode,
                        recording: false,
                        sharing: false,
                        output: "Monitor ACM2702 (1)",
                        tray: &[],
                        system: &system,
                        keyboard: None,
                    },
                    Some(&mut wheel),
                );
            });
            // No screen takes the pictures egui made.
            output.textures_delta.clear();
            asked
        };
        // Twice: the middle is placed by how wide it was the first time.
        frame(Vec::new(), 2, "resize");
        // (It says which part of its surface it is: all of it, as a strip.)
        let mut quiet = frame(Vec::new(), 2, "resize");
        assert_eq!(quiet.reach.len(), 1);
        quiet.reach.clear();
        assert_eq!(quiet, Asked::default());

        // A notch of the wheel towards the user over it: the next workspace.
        let notch = egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        let asked = frame(vec![notch], 0, "");
        assert_eq!(asked.show, Some(1));
    }

    #[test]
    fn the_signs_for_the_sound_and_the_battery_follow_their_state() {
        use gevrek_system::status::Battery;
        assert_eq!(sound_icon(80, true), icons::SPEAKER_X);
        assert_eq!(sound_icon(0, false), icons::SPEAKER_NONE);
        assert_eq!(sound_icon(30, false), icons::SPEAKER_LOW);
        assert_eq!(sound_icon(80, false), icons::SPEAKER_HIGH);
        let at = |percent, charging| battery_icon(Battery { percent, charging });
        assert_eq!(at(5, false), icons::BATTERY_EMPTY);
        assert_eq!(at(50, false), icons::BATTERY_MEDIUM);
        assert_eq!(at(95, false), icons::BATTERY_FULL);
        assert_eq!(at(5, true), icons::BATTERY_CHARGING);
    }
}
