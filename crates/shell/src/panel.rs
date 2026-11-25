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
    PanelAlign, PanelEdge, PanelFit, PanelModule, PanelProfile, PanelStyle, TaskGrouping,
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
    /// The person minimized it: its button is drawn apart from the
    /// others (`task_look`), and a press brings it back.
    pub minimized: bool,
    /// It floats (is not in the layout): only such a window can be kept
    /// on top.
    pub floating: bool,
    /// It is kept above the other windows (always on top): its button
    /// has a pin in its corner.
    pub on_top: bool,
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
///
/// The display is the one with that connector; only where none has it,
/// the one of that name. (Asked for both at once, the first display of
/// the state that answered to either was taken: a display named as
/// another's connector is would have had that other's panel.)
pub fn view(state: &Value, output: &str) -> Option<View> {
    let (index, monitor) = monitor_of(state, output)?;
    Some(view_at(monitor, Some(index)))
}

/// The display of the state that `output` means, and its place among
/// the displays: by its connector before any name.
pub fn monitor_of<'a>(state: &'a Value, output: &str) -> Option<(usize, &'a Value)> {
    let monitors = state["monitors"].as_array()?;
    let by = |key: &str| {
        monitors
            .iter()
            .enumerate()
            .find(|(_, monitor)| monitor[key].as_str() == Some(output))
    };
    by("connector").or_else(|| by("name"))
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
                minimized: window["is_minimized"].as_bool().unwrap_or(false),
                floating: window["is_floating"].as_bool().unwrap_or(false),
                on_top: window["is_always_on_top"].as_bool().unwrap_or(false),
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
        return Renaming::Refused(
            "This display has no name to write the title down for.".to_string(),
        );
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

/// What stands between a display's name and a panel's id in the key of
/// a panel on a display (a sign no name has).
const KEY_BETWEEN: char = '\u{1f}';

/// What one panel on one display is known by among the open panels: the
/// display and the panel's id. (For a panel whose id is not told, the
/// display alone: a window manager from before there were several.)
pub fn key(display: &str, panel: &str) -> String {
    if panel.is_empty() {
        display.to_string()
    } else {
        format!("{display}{KEY_BETWEEN}{panel}")
    }
}

/// The display of a panel's key.
pub fn key_display(key: &str) -> &str {
    key.split(KEY_BETWEEN).next().unwrap_or(key)
}

/// The panel's id of a panel's key (nothing: the first there is).
pub fn key_panel(key: &str) -> &str {
    key.split_once(KEY_BETWEEN).map_or("", |(_, panel)| panel)
}

/// The rectangle a state gives by where it is and how large, where it
/// gives one that is a rectangle.
fn rect_of(at: &Value, size: &Value) -> Option<Rect> {
    let whole = |value: &Value| value.as_f64().map(|number| number.round() as i32);
    let rect = (
        whole(&at["x"])?,
        whole(&at["y"])?,
        whole(&size["width"])?,
        whole(&size["height"])?,
    );
    (rect.2 > 0 && rect.3 > 0).then_some(rect)
}

/// Where the panels of one display of the state belong: each one's id
/// (nothing where the state tells of one panel without saying which) and
/// its rectangle.
fn panels_of(monitor: &Value) -> Vec<(String, Rect)> {
    let told: Vec<(String, Rect)> = monitor["panels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|panel| {
            Some((
                panel["id"].as_str().unwrap_or_default().to_string(),
                rect_of(&panel["position"], &panel["size"])?,
            ))
        })
        .collect();
    if !told.is_empty() {
        return told;
    }
    rect_of(&monitor["bar_position"], &monitor["bar_size"])
        .map(|rect| (String::new(), rect))
        .into_iter()
        .collect()
}

/// Where the window manager keeps room for the panels: the key of each
/// panel on each display ([`key`]) and its rectangle. A display the
/// window manager keeps no room on (no panel in its configuration) has
/// none among them.
///
/// This is how the window manager's names for the displays (the model's
/// name, `Monitor ACM2702 (2)`) and the system's (`DISPLAY1`) meet on
/// Windows: by the place. The panel of a display is put at the rectangle
/// the window manager gives for it, and is so on the display it means,
/// whatever either calls it.
pub fn places(state: &Value) -> Vec<(String, Rect)> {
    displays(state)
        .into_iter()
        .flat_map(|(name, monitor)| {
            panels_of(monitor)
                .into_iter()
                .map(move |(panel, rect)| (key(&name, &panel), rect))
        })
        .collect()
}

/// What the shell knows of a display to put a surface on it by itself:
/// the room windows have there, the panel's rectangle where there is one
/// (both in the pixels of the screen), and the pixels to a logical pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub work: Rect,
    /// The panels of the display, each where it is.
    pub bars: Vec<Rect>,
    pub scale: f32,
    /// The display has the keyboard's window.
    pub focused: bool,
}

/// The rooms of the displays of the window manager's state. A display
/// the state gives no size for is not among them.
pub fn rooms(state: &Value) -> Vec<Room> {
    let rect = rect_of;
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
                bars: panels_of(monitor).into_iter().map(|(_, at)| at).collect(),
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
    let (mut from, mut to) = (x, x + width);
    for (bar_x, bar_y, bar_width, bar_height) in room.bars.iter().copied() {
        if bar_width >= bar_height {
            // A panel in the upper half is at the top, else at the bottom.
            if bar_y + bar_height / 2 < y + height / 2 {
                top = top.max(bar_y + bar_height);
            } else {
                bottom = bottom.min(bar_y);
            }
        } else if bar_x + bar_width / 2 < x + width / 2 {
            // One down a side: at the left, or at the right. (It was
            // taken for one at the top, and nothing had room under it.)
            from = from.max(bar_x + bar_width);
        } else {
            to = to.min(bar_x);
        }
    }
    let wide = size.0.min(to - from - 2 * gap).max(1);
    let high = size.1.min(bottom - top - 2 * gap).max(1);
    let left = match corner {
        Corner::TopLeft | Corner::BottomLeft => from + gap,
        Corner::TopRight | Corner::BottomRight => to - gap - wide,
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
                // A window that was let go of may be on another display
                // now, or at another place among its workspace's: once
                // for a drag, not many times a second as while it moves.
                // (Left out, the panel of the display a window was
                // dragged from listed it until something else happened.)
                | "window.drag_ended"
                // And however else a window came to another workspace or
                // display: the window manager tells it from what it
                // publishes.
                | "window.workspace_changed"
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
pub fn left_out(bars: &[PanelProfile]) -> Vec<PanelModule> {
    let mut found: Vec<PanelModule> = Vec::new();
    for bar in bars.iter().filter(|bar| bar.enabled) {
        for module in bar.left.iter().chain(&bar.centre).chain(&bar.right) {
            if !works_here(module) && !found.contains(module) {
                found.push(module.clone());
            }
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
        // (One that is out of sight at times stands at the display's
        // very edge: the pointer comes there to bring it back.)
        if bar.hide == gevrek_model::settings::PanelHide::Never {
            bar.margin.min(100)
        } else {
            0
        },
        // Over the windows: no room is kept for it.
        !bar.keeps_room(),
    )
}

/// How long a panel that is to be out of sight stays after the pointer
/// left it (or the window that covers it came).
pub const HIDE_AFTER: std::time::Duration = std::time::Duration::from_millis(700);

/// How much of a panel that is out of sight is left at the display's
/// edge for the pointer to come to, in points.
pub const HIDDEN_STRIP: f32 = 3.0;

/// Whether a panel that is out of sight at times is in sight now.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hiding {
    /// Since when nothing has held it in sight.
    away_since: Option<std::time::Instant>,
    /// It is out of sight.
    pub hidden: bool,
}

impl Hiding {
    /// One look: whether the panel is in sight, and when to look again
    /// where that depends on the clock. `covered`: a window lies where it
    /// stands. `held`: the pointer is over it, or a menu of its is open.
    pub fn step(
        &mut self,
        hide: gevrek_model::settings::PanelHide,
        covered: bool,
        held: bool,
        now: std::time::Instant,
    ) -> (bool, Option<std::time::Duration>) {
        use gevrek_model::settings::PanelHide;
        let to_hide = match hide {
            PanelHide::Never => false,
            PanelHide::Always => true,
            PanelHide::Covered => covered,
        };
        if !to_hide || held {
            self.away_since = None;
            self.hidden = false;
            return (true, None);
        }
        if self.hidden {
            return (false, None);
        }
        let since = *self.away_since.get_or_insert(now);
        let gone = now.saturating_duration_since(since);
        if gone >= HIDE_AFTER {
            self.hidden = true;
            (false, None)
        } else {
            (true, Some(HIDE_AFTER - gone))
        }
    }
}

/// The part of a hidden panel's surface that stays for the pointer to
/// come to: a strip at the surface's outer edge (the display's).
pub fn hidden_reach(whole: egui::Rect, edge: PanelEdge) -> egui::Rect {
    match edge {
        PanelEdge::Top => egui::Rect::from_min_max(
            whole.min,
            egui::pos2(whole.right(), whole.top() + HIDDEN_STRIP),
        ),
        PanelEdge::Bottom => egui::Rect::from_min_max(
            egui::pos2(whole.left(), whole.bottom() - HIDDEN_STRIP),
            whole.max,
        ),
        PanelEdge::Left => egui::Rect::from_min_max(
            whole.min,
            egui::pos2(whole.left() + HIDDEN_STRIP, whole.bottom()),
        ),
        PanelEdge::Right => egui::Rect::from_min_max(
            egui::pos2(whole.right() - HIDDEN_STRIP, whole.top()),
            whole.max,
        ),
    }
}

/// The same for a panel that is a window placed at `at` (the pixels of
/// the screen): the strip of the screen its window is while it is out of
/// sight, `strip` pixels thick at the panel's outer edge.
pub fn hidden_place(at: Rect, edge: PanelEdge, strip: i32) -> Rect {
    let (x, y, width, height) = at;
    let strip = strip.max(1);
    match edge {
        PanelEdge::Top => (x, y, width, strip.min(height)),
        PanelEdge::Bottom => (x, y + (height - strip).max(0), width, strip.min(height)),
        PanelEdge::Left => (x, y, strip.min(width), height),
        PanelEdge::Right => (x + (width - strip).max(0), y, strip.min(width), height),
    }
}

/// Whether a window lies where a panel stands: one of the workspace its
/// display shows, that is there to be seen, over the rectangle `at` (the
/// pixels of the screen).
pub fn covered(state: &Value, display: &str, at: Rect) -> bool {
    let Some((_, monitor)) = displays(state)
        .into_iter()
        .find(|(name, _)| name == display)
        .map(|(name, monitor)| (name, monitor))
        .or_else(|| monitor_of(state, display).map(|(_, monitor)| (display.to_string(), monitor)))
    else {
        return false;
    };
    let (x, y, width, height) = at;
    monitor["workspace"]["detailed_windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|window| !window["is_border_window"].as_bool().unwrap_or(false))
        .filter(|window| !window["is_minimized"].as_bool().unwrap_or(false))
        .filter_map(|window| rect_of(&window["position"], &window["size"]))
        .any(|(left, top, wide, high)| {
            left < x + width && x < left + wide && top < y + height && y < top + high
        })
}

/// Nothing at all on a panel's surface: the panel is out of sight.
pub fn draw_nothing(ui: &mut egui::Ui) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |_| {});
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
/// The command that brings a window to the middle of this panel's
/// display, floating, or puts it back as it was.
pub fn centre_window(view: &View, window: u64) -> Option<String> {
    use gevrek_model::command::{DispatchCommand, MonitorTarget, WindowSelector};
    let display = view.place.name();
    let command = DispatchCommand::ToggleWindowCentered {
        window: WindowSelector::Handle(window as i64),
        target: if display.is_empty() {
            MonitorTarget::Primary
        } else {
            MonitorTarget::Name(display.to_string())
        },
    };
    serde_json::to_string(&command).ok()
}

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

/// The panel of this id as the configuration has it, where it is on
/// this display (`PanelProfile::is_on`); with no id, the first panel
/// that is on it. `None` where that panel is not on that display. The
/// display is known by several names (what the window manager calls it,
/// its connector): a panel that names it by any of them is on it.
pub fn bar_on<'a>(
    bars: &'a [PanelProfile],
    panel: &str,
    display: &str,
    place: &gevrek_model::settings::DisplayRef,
) -> Option<&'a PanelProfile> {
    let mut place = place.clone();
    if !place.is_called(display) && !display.trim().is_empty() {
        place.names.push(display.trim().to_string());
    }
    bars.iter()
        .filter(|bar| panel.is_empty() || bar.is(panel))
        .find(|bar| bar.is_on(&place))
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
    /// What the later modules show (the machine's load, what is playing,
    /// the panel's own words), as far as it is known.
    pub extras: &'a Extras,
}

/// What a panel's later modules show. Each is `None` where there is
/// nothing to say: its module then shows nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extras {
    /// How busy the machine is.
    pub load: Option<gevrek_system::status::Load>,
    /// What is playing, in one line, and whether it plays now.
    pub playing: Option<(String, bool)>,
    /// What this panel's program wrote (its `custom`), where it has one
    /// and it wrote something.
    pub custom: Option<String>,
}

/// What the module of what is playing asks of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAsk {
    PlayPause,
    Next,
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
        Self::read_with(None)
    }

    /// What there is to say now; the network as `kept` has it where one
    /// is given (asking for it costs the start of a program on Linux, so
    /// whoever asks often asks for it only when it may be another).
    pub fn read_with(kept: Option<&SystemView>) -> Self {
        Self {
            volume: gevrek_system::sound::level(gevrek_system::sound::Device::Speaker)
                .map(|level| (level.percent, level.muted)),
            battery: gevrek_system::status::battery(),
            network: match kept {
                Some(kept) => kept.network.clone(),
                None => gevrek_system::status::network(),
            },
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
    /// The other button was pressed on a window of the taskbar: the
    /// menu of what can be done with it, next to this rectangle.
    pub task_actions: Option<(egui::Rect, u64)>,
    /// A button of the taskbar that stands for several windows was
    /// pressed (the windows of one program, or those there is no room
    /// for): list these next to this rectangle.
    pub task_menu: Option<(egui::Rect, Vec<u64>)>,
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
    /// The other button was pressed on the panel itself, where nothing
    /// stands: the panel's own menu, at this place.
    pub panel_menu: Option<egui::Rect>,
    /// What is playing was pressed: pause or go on, or the next piece.
    pub media: Option<MediaAsk>,
    /// The panel's own words were pressed (its `custom`, which has a
    /// program to start then).
    pub custom_pressed: bool,
    /// The middle button was pressed on a window's button of the
    /// taskbar: that window is to be closed.
    pub close_window: Option<u64>,
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
#[derive(Clone, Copy)]
struct Ink {
    text: egui::Color32,
    accent: egui::Color32,
    /// The panel goes down a side: what it shows stands under each
    /// other and is kept narrow.
    upright: bool,
    /// How words are written there: turned a quarter (a word is as long
    /// as it is), upright and short, or not at all.
    side: gevrek_model::settings::SideText,
    /// How long a module that takes what room there is (the taskbar)
    /// may be: as long as its group may be, less what else stands in
    /// the group.
    room: f32,
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
        side: bar.side_text,
        room: f32::INFINITY,
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
            // The panel itself, under what stands on it: the other button
            // on it, where nothing stands, asks for the panel's own menu.
            // (Before the modules: each of them is over it, and hears its
            // own presses.)
            let ground = ui.interact(
                strip,
                egui::Id::unique(("panel-ground", shown.output)),
                egui::Sense::click(),
            );
            if ground.secondary_clicked() {
                let at = ground
                    .interact_pointer_pos()
                    .unwrap_or_else(|| strip.center());
                asked.panel_menu = Some(egui::Rect::from_center_size(at, egui::vec2(2.0, 2.0)));
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
                         ink: &Ink,
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
                                draw_module(ui, module, shown, ink, asked);
                            }
                            if marks {
                                draw_marks(ui, shown, ink, asked);
                            }
                        } else {
                            if marks {
                                draw_marks(ui, shown, ink, asked);
                            }
                            for module in modules {
                                draw_module(ui, module, shown, ink, asked);
                            }
                        }
                    })
                    .response
                    .rect;
                (long(rect) > 0.5).then_some(rect)
            };
            // What stands in a group beside its taskbar is as long as it
            // was the last time: the taskbar has the rest of the room
            // its group may take, and no more of it.
            let beside_id =
                |slot: usize| egui::Id::unique(("panel-beside-tasks", shown.output, slot));
            let with_room = |slot: usize, most: f32| {
                let beside: f32 = context
                    .data(|data| data.get_temp(beside_id(slot)))
                    .unwrap_or(0.0);
                Ink {
                    room: (most - beside).max(0.0),
                    ..ink
                }
            };
            // (`all`: how long the group came to be, its taskbar with it.
            // Nothing is noted for a group without one.)
            let note_tasks = |slot: usize, all: f32| {
                let Some(tasks) = context.data_mut(|data| data.remove_temp::<f32>(task_long_id()))
                else {
                    return;
                };
                let beside = (all - tasks).max(0.0);
                let before: f32 = context
                    .data(|data| data.get_temp(beside_id(slot)))
                    .unwrap_or(0.0);
                if (beside - before).abs() > 0.5 {
                    context.data_mut(|data| data.insert_temp(beside_id(slot), beside));
                    context.request_repaint();
                }
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
                // (The run is not longer than the display: a taskbar in
                // it has what the rest of the run leaves of that.)
                let ink = with_room(3, long(whole) - padding * 2.0);
                let builder = egui::UiBuilder::new().max_rect(inner).layout(forward);
                ui.scope_builder(builder, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    groups[0] = group(ui, &bar.left, false, false, &ink, &mut asked);
                    if groups[0].is_some() && !bar.centre.is_empty() {
                        ui.add_space(between);
                    }
                    groups[1] = group(ui, &bar.centre, false, false, &ink, &mut asked);
                    if groups[0].is_some() || groups[1].is_some() {
                        ui.add_space(between);
                    }
                    groups[2] = group(ui, &bar.right, true, false, &ink, &mut asked);
                });
                let run = groups
                    .iter()
                    .flatten()
                    .fold(None, |all: Option<egui::Rect>, one| {
                        Some(all.map_or(*one, |all| all.union(*one)))
                    })
                    .map_or(0.0, long);
                note_tasks(3, run);
                let used = run + padding * 2.0;
                if natural.is_none_or(|natural| (used - natural).abs() > 0.5) {
                    context.data_mut(|data| data.insert_temp(natural_id, used));
                    context.request_repaint();
                }
            } else {
                // The three groups are as long as they were the last
                // time: each may reach up to where its neighbours stand
                // (the middle one stays in the middle), and no further.
                let end_id = egui::Id::unique(("panel-end", shown.output));
                let end_before: f32 = context.data(|data| data.get_temp(end_id)).unwrap_or(0.0);
                let centre_id = egui::Id::unique(("panel-centre", shown.output));
                let centre_before: f32 = if bar.centre.is_empty() {
                    0.0
                } else {
                    context.data(|data| data.get_temp(centre_id)).unwrap_or(0.0)
                };
                // (How long the start is matters to a taskbar that
                // stands elsewhere, and is kept only for one.)
                let start_id = egui::Id::unique(("panel-start", shown.output));
                let tasks_elsewhere = bar
                    .centre
                    .iter()
                    .chain(&bar.right)
                    .any(|module| *module == PanelModule::Taskbar);
                // (And to a title that runs down a side in the middle,
                // which is as long as the room the ends leave it.)
                let start_matters = tasks_elsewhere
                    || (upright
                        && bar
                            .centre
                            .iter()
                            .any(|module| *module == PanelModule::Title));
                let start_before: f32 = if start_matters {
                    context.data(|data| data.get_temp(start_id)).unwrap_or(0.0)
                } else {
                    0.0
                };
                let (first, middle, last) = if upright {
                    (inner.top(), strip.center().y, inner.bottom())
                } else {
                    (inner.left(), strip.center().x, inner.right())
                };
                // (A group, with the room between it and the next.)
                let kept = |long: f32| if long > 0.0 { long + spacing } else { 0.0 };
                let half = kept(centre_before / 2.0);
                let start_most = (last - kept(end_before) - first).min(if half > 0.0 {
                    middle - half - first
                } else {
                    f32::INFINITY
                });
                let end_most = (last - first - kept(start_before)).min(if half > 0.0 {
                    last - middle - half
                } else {
                    f32::INFINITY
                });
                let centre_most = 2.0
                    * (middle - first - kept(start_before)).min(last - kept(end_before) - middle);
                let start_ink = with_room(0, start_most);
                let end_ink = with_room(2, end_most);
                let builder = egui::UiBuilder::new().max_rect(inner).layout(forward);
                ui.scope_builder(builder, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    groups[0] = group(ui, &bar.left, false, false, &start_ink, &mut asked);
                    note_tasks(0, groups[0].map_or(0.0, long));
                    if !upright {
                        ui.with_layout(backward, |ui| {
                            groups[2] = group(ui, &bar.right, true, true, &end_ink, &mut asked);
                            note_tasks(2, groups[2].map_or(0.0, long));
                        });
                    }
                });
                let start_now = groups[0].map_or(0.0, long);
                if start_matters && (start_now - start_before).abs() > 0.5 {
                    context.data_mut(|data| data.insert_temp(start_id, start_now));
                    context.request_repaint();
                }
                if upright {
                    // Down a side it is laid out from the top as the rest
                    // is (from the bottom up, what a module shows under
                    // each other came out the wrong way round), and put
                    // at the end by how long it was the last time.
                    let place = egui::Rect::from_min_max(
                        egui::pos2(inner.left(), inner.bottom() - end_before.max(1.0)),
                        inner.right_bottom(),
                    );
                    let builder = egui::UiBuilder::new().max_rect(place).layout(forward);
                    ui.scope_builder(builder, |ui| {
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        groups[2] = group(ui, &bar.right, true, false, &end_ink, &mut asked);
                        note_tasks(2, groups[2].map_or(0.0, long));
                    });
                }
                let end_now = groups[2].map_or(0.0, long);
                if (end_now - end_before).abs() > 0.5 {
                    context.data_mut(|data| data.insert_temp(end_id, end_now));
                    context.request_repaint();
                }
                if !bar.centre.is_empty() {
                    // In the middle of the panel whatever stands at its
                    // ends: placed by how long it was the last time.
                    let (id, before) = (centre_id, centre_before);
                    let centre_ink = with_room(1, centre_most);
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
                        groups[1] = group(ui, &bar.centre, false, false, &centre_ink, &mut asked);
                        note_tasks(1, groups[1].map_or(0.0, long));
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
        let words = if ink.turned() { "recording" } else { words };
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
        let words = if ink.turned() { "sharing" } else { words };
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

/// What a cell of the panel shows before its words.
#[derive(Clone, Copy)]
enum Sign<'a> {
    None,
    /// A sign of the icon font.
    Font(&'a str),
    /// One of the brand's own, and the icon font's for where that cannot
    /// be drawn.
    Brand(BrandIcon, &'a str),
}

/// How a cell is drawn.
struct Cell<'a> {
    sign: Sign<'a>,
    /// Its words: beside the sign, or under it (smaller) down a side.
    words: Option<&'a str>,
    colour: egui::Color32,
    /// A ground it has always (one that is pressed lights up besides).
    fill: Option<egui::Color32>,
    /// It is a button.
    pressed: bool,
}

impl Ink {
    /// Words are written turned a quarter here: down a side, and wanted so.
    fn turned(&self) -> bool {
        self.upright && self.side == gevrek_model::settings::SideText::Rotated
    }

    /// Words are kept short here (a number without its sign, a few
    /// letters of a name): down a side, where they are not turned.
    fn short(&self) -> bool {
        self.upright && self.side != gevrek_model::settings::SideText::Rotated
    }

    /// No words at all here: down a side, the signs alone.
    fn signs_only(&self) -> bool {
        self.upright && self.side == gevrek_model::settings::SideText::Signs
    }
}

/// The size of the words on a panel down a side where they are turned.
const TURNED_SIZE: f32 = 12.0;

/// Paint words turned a quarter, read from the top down: in the column
/// whose middle is `centre_x`, from `top` on. (They take as much of the
/// panel's length as they are wide, and as much of its thickness as
/// they are high.)
fn paint_turned(
    painter: &egui::Painter,
    words: std::sync::Arc<egui::Galley>,
    centre_x: f32,
    top: f32,
    colour: egui::Color32,
) {
    // (Turned about its upper left corner, which is the upper right one
    // of the place the turned words take.)
    let at = egui::pos2(centre_x + words.size().y / 2.0, top);
    painter.add(
        egui::epaint::TextShape::new(at, words, colour).with_angle(std::f32::consts::FRAC_PI_2),
    );
}

/// How thick the modules of a panel are across it, by the panel's own
/// thickness: a low panel has low modules.
fn across_of(ui: &egui::Ui, upright: bool) -> f32 {
    if upright {
        (ui.max_rect().width() - 8.0).clamp(16.0, 64.0)
    } else {
        (ui.max_rect().height() - 10.0).clamp(14.0, 30.0)
    }
}

/// One cell of the panel: a sign, words, or both, on a rounded ground
/// that lights under the pointer where it is a button. Along a panel
/// that lies, the words stand beside the sign; down a side they stand
/// under it, smaller, and the cell is as wide as the panel.
fn cell(ui: &mut egui::Ui, ink: &Ink, cell: &Cell) -> egui::Response {
    let across = across_of(ui, ink.upright);
    let turned = ink.turned();
    let font = egui::FontId::proportional(if turned {
        TURNED_SIZE
    } else if ink.upright {
        10.5
    } else {
        12.5
    });
    let has_sign = !matches!(cell.sign, Sign::None);
    let words = cell
        .words
        .filter(|words| !words.is_empty())
        // (The signs alone: no words beside a sign. Words that have no
        // sign are all the cell says, and stay.)
        .filter(|_| !(ink.signs_only() && has_sign))
        .map(|words| {
            ui.painter()
                .layout_no_wrap(words.to_string(), font, cell.colour)
        });
    let size = if turned {
        // The sign, and under it the words, turned: as long as they are.
        let high = 5.0
            + if has_sign { 17.0 } else { 0.0 }
            + words.as_ref().map_or(0.0, |words| {
                words.size().x + if has_sign { 3.0 } else { 0.0 }
            })
            + 5.0;
        egui::vec2(across, high.max(22.0))
    } else if ink.upright {
        let high = 5.0
            + if has_sign { 17.0 } else { 0.0 }
            + words.as_ref().map_or(0.0, |words| words.size().y + 1.0)
            + 5.0;
        egui::vec2(across, high.max(22.0))
    } else {
        let wide = match (&words, has_sign) {
            (Some(words), true) => 8.0 + 16.0 + 5.0 + words.size().x + 8.0,
            (Some(words), false) => 9.0 + words.size().x + 9.0,
            (None, _) => across.max(26.0),
        };
        egui::vec2(wide, across)
    };
    let sense = if cell.pressed {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let lit = cell.pressed && response.hovered();
    if let Some(fill) = cell.fill {
        ui.painter().rect_filled(rect, 8.0, fill);
    }
    if lit {
        ui.painter()
            .rect_filled(rect, 8.0, ink.text.gamma_multiply(0.10));
    }
    let sign_at = if ink.upright {
        egui::pos2(
            rect.center().x,
            if words.is_some() {
                rect.top() + 5.0 + 8.5
            } else {
                rect.center().y
            },
        )
    } else if words.is_some() {
        egui::pos2(rect.left() + 8.0 + 8.0, rect.center().y)
    } else {
        rect.center()
    };
    let font_sign = |ui: &egui::Ui, sign: &str| {
        ui.painter().text(
            sign_at,
            egui::Align2::CENTER_CENTER,
            sign,
            egui::FontId::proportional(16.0),
            cell.colour,
        );
    };
    match cell.sign {
        Sign::None => {}
        Sign::Font(sign) => font_sign(ui, sign),
        Sign::Brand(icon, or) => {
            match gevrek_ui::brand_icons::image(ui.ctx(), icon, 15.0, cell.colour) {
                Some(image) => image.paint_at(
                    ui,
                    egui::Rect::from_center_size(sign_at, egui::Vec2::splat(15.0)),
                ),
                None => font_sign(ui, or),
            }
        }
    }
    if let (Some(words), true) = (&words, turned) {
        let top = rect.top() + 5.0 + if has_sign { 17.0 + 3.0 } else { 0.0 };
        // (In the middle of what room there is, where the cell is longer
        // than its words: one that is 22 high at the least.)
        let room = rect.bottom() - 5.0 - top;
        let top = top + ((room - words.size().x) / 2.0).max(0.0);
        paint_turned(
            ui.painter(),
            words.clone(),
            rect.center().x,
            top,
            cell.colour,
        );
        return response;
    }
    if let Some(words) = words {
        let at = if ink.upright {
            egui::pos2(
                rect.center().x - words.size().x / 2.0,
                if has_sign {
                    rect.top() + 5.0 + 17.0 + 1.0
                } else {
                    rect.center().y - words.size().y / 2.0
                },
            )
        } else {
            egui::pos2(
                rect.left() + if has_sign { 8.0 + 16.0 + 5.0 } else { 9.0 },
                rect.center().y - words.size().y / 2.0,
            )
        };
        ui.painter().galley(at, words, cell.colour);
    }
    response
}

/// A name in as few letters as a narrow place has room for: a number as
/// it is, else its first two letters.
fn short_name(name: &str) -> String {
    let name = name.trim();
    if name.chars().all(|letter| letter.is_ascii_digit()) {
        return name.chars().take(3).collect();
    }
    let mut letters = name.chars().filter(|letter| !letter.is_whitespace());
    match letters.next() {
        Some(first) => first
            .to_uppercase()
            .chain(letters.next().into_iter().flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
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
    let upright = ink.upright;
    let across = across_of(ui, upright);
    // (Along the panel, across it.)
    let along = |long: f32| {
        if upright {
            egui::vec2(across, long)
        } else {
            egui::vec2(long, across)
        }
    };
    let quiet = text.gamma_multiply(0.55);
    // A cell that only says something, and one that is pressed.
    fn says<'a>(sign: Sign<'a>, words: Option<&'a str>, colour: egui::Color32) -> Cell<'a> {
        Cell {
            sign,
            words,
            colour,
            fill: None,
            pressed: false,
        }
    }
    match module {
        PanelModule::Launcher => {
            // gevrek's mark is the launcher's button: the one for a dark
            // ground where the panel's words are light, and the other way
            // round. (Where the picture cannot be read: the glass.)
            let on_dark = theme::luminance(text) > 0.5;
            let side = (across - 4.0).clamp(12.0, 22.0);
            let button = match gevrek_ui::brand::mark(ui.ctx(), on_dark, side) {
                Some(mark) => {
                    let (rect, response) =
                        ui.allocate_exact_size(along(across.max(26.0)), egui::Sense::click());
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, 8.0, text.gamma_multiply(0.10));
                    }
                    ui.painter().image(
                        mark.id(),
                        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side)),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    response
                }
                None => cell(
                    ui,
                    ink,
                    &Cell {
                        pressed: true,
                        ..says(
                            Sign::Brand(BrandIcon::Launcher, icons::MAGNIFYING_GLASS),
                            None,
                            text,
                        )
                    },
                ),
            };
            if button.hint("Launcher").clicked() {
                asked.pressed = Some(PanelModule::Launcher);
            }
        }
        PanelModule::Workspaces => {
            // The workspaces stand together on one ground, as the parts
            // of one thing; the one that is shown is the accent's, one
            // with windows reads stronger than an empty one and has a
            // dot to say so.
            let ground = ui.painter().add(egui::Shape::Noop);
            let before = ui.spacing().item_spacing;
            let all = ui
                .scope(|ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::splat(2.0);
                    for (index, tab) in view.tabs.iter().enumerate() {
                        let said = if ink.short() {
                            short_name(&tab.label)
                        } else {
                            tab.label.clone()
                        };
                        let colour = if tab.shown {
                            theme::readable(egui::Color32::BLACK, accent, 4.5)
                        } else if tab.windows > 0 {
                            text
                        } else {
                            text.gamma_multiply(0.45)
                        };
                        let words = ui.painter().layout_no_wrap(
                            said,
                            egui::FontId::proportional(12.5),
                            colour,
                        );
                        let size = if ink.turned() {
                            // As long as its name, which is turned.
                            egui::vec2(across, (words.size().x + 16.0).max(26.0))
                        } else if upright {
                            egui::vec2(across, 26.0)
                        } else {
                            egui::vec2((words.size().x + 18.0).max(28.0), across)
                        };
                        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                        if tab.shown {
                            ui.painter().rect_filled(rect, 7.0, accent);
                        } else if response.hovered() {
                            ui.painter()
                                .rect_filled(rect, 7.0, text.gamma_multiply(0.12));
                        }
                        if ink.turned() {
                            paint_turned(
                                ui.painter(),
                                words.clone(),
                                rect.center().x,
                                rect.center().y - words.size().x / 2.0,
                                colour,
                            );
                        } else {
                            ui.painter()
                                .galley(rect.center() - words.size() / 2.0, words, colour);
                        }
                        if tab.windows > 0 && !tab.shown {
                            // (Its dot: under the name, or beside it where
                            // the name runs down.)
                            let dot = if ink.turned() {
                                egui::pos2(rect.left() + 3.0, rect.center().y)
                            } else {
                                egui::pos2(rect.center().x, rect.bottom() - 3.0)
                            };
                            ui.painter()
                                .circle_filled(dot, 1.5, text.gamma_multiply(0.6));
                        }
                        let response = response
                            .hint(format!("Workspace {}. Right-click to rename.", tab.name));
                        if response.clicked() && !tab.shown {
                            asked.show = Some(index);
                        }
                        if response.secondary_clicked() {
                            asked.rename = Some(index);
                        }
                    }
                })
                .response
                .rect;
            ui.spacing_mut().item_spacing = before;
            if !view.tabs.is_empty() {
                ui.painter().set(
                    ground,
                    egui::Shape::rect_filled(all, 8.0, text.gamma_multiply(0.06)),
                );
            }
        }
        PanelModule::Layout if !view.layout.is_empty() => {
            // The layout, and a press on it the menu of them. (Down a
            // side: its sign, and the word as a hint.)
            let said = if view.floating {
                "Floating".to_string()
            } else {
                layout_label(&view.layout)
            };
            let response = cell(
                ui,
                ink,
                &Cell {
                    sign: Sign::Font(icons::SQUARES_FOUR),
                    words: (!ink.short()).then_some(said.as_str()),
                    colour: text,
                    fill: Some(text.gamma_multiply(0.07)),
                    pressed: true,
                },
            )
            .hint(format!("Layout: {said}. A press chooses another."));
            if response.clicked() {
                asked.layout_menu = Some(response.rect);
            }
        }
        // Down a side a title runs down it, turned, in a place as long as
        // `title_width` (as long as it is where that is 0, but never more
        // than a third of a screen): one that is longer ends in "…".
        PanelModule::Title if !view.title.is_empty() && ink.turned() => {
            let colour = text.gamma_multiply(if view.focused { 0.95 } else { 0.55 });
            // (Not by the room its group has just now: the group is placed
            // by how long it was the last time, which is nothing before
            // the title was ever drawn. By the display: never more than
            // two fifths of how high the panel is.)
            let most = if bar.title_width == 0 {
                360.0
            } else {
                bar.title_width as f32
            }
            .min(ui.ctx().content_rect().height() * 0.4)
            // (And by what its group may take: in the middle of a panel,
            // what the ends leave it.)
            .min(ink.room);
            if most < 48.0 {
                return;
            }
            let title: String = view.title.chars().take(200).collect();
            let mut job = egui::text::LayoutJob::simple_singleline(
                title,
                egui::FontId::proportional(TURNED_SIZE),
                colour,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(most - 8.0);
            let words = ui.painter().layout_job(job);
            let long = if bar.title_width == 0 {
                words.size().x + 8.0
            } else {
                most
            }
            .min(most);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(across, long), egui::Sense::hover());
            paint_turned(
                ui.painter(),
                words,
                rect.center().x,
                rect.top() + 4.0,
                colour,
            );
        }
        // (Not where words are short or left out: a title is as long as
        // it is.)
        PanelModule::Title if !view.title.is_empty() && !upright => {
            let title: String = view.title.chars().take(200).collect();
            let words = egui::RichText::new(title)
                .size(13.0)
                .color(text.gamma_multiply(if view.focused { 0.95 } else { 0.55 }));
            if bar.title_width == 0 {
                ui.label(words);
            } else {
                // A place of one width, whatever it says: a title that
                // changes (a count, a progress) moves nothing after it,
                // and one that is too long ends in "…".
                let wide = (bar.title_width as f32).min(ui.available_width().max(20.0));
                // (Where there is room for a few letters only, a title
                // would be its three dots and nothing else: left out.)
                if wide < 48.0 {
                    return;
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(wide, across),
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
            if ink.turned() {
                // One line, turned, as along the top.
                cell(ui, ink, &says(Sign::None, Some(said.trim()), text));
            } else if upright {
                // Its parts under each other, in their order; the time
                // itself (the part with a colon) larger, its hours over
                // its minutes.
                let parts: Vec<&str> = said.split_whitespace().collect();
                let lines: Vec<(String, f32, egui::Color32)> = parts
                    .iter()
                    .flat_map(|part| {
                        if part.contains(':') {
                            part.split(':')
                                .map(|piece| (piece.to_string(), 14.5, text))
                                .collect::<Vec<_>>()
                        } else {
                            vec![(part.to_string(), 10.5, text.gamma_multiply(0.7))]
                        }
                    })
                    .collect();
                let laid: Vec<_> = lines
                    .into_iter()
                    .map(|(line, size, colour)| {
                        (
                            ui.painter().layout_no_wrap(
                                line,
                                egui::FontId::proportional(size),
                                colour,
                            ),
                            colour,
                        )
                    })
                    .collect();
                let high: f32 = laid.iter().map(|(line, _)| line.size().y).sum::<f32>() + 8.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(across, high), egui::Sense::hover());
                let mut y = rect.top() + 4.0;
                for (line, colour) in laid {
                    let x = rect.center().x - line.size().x / 2.0;
                    let down = line.size().y;
                    ui.painter().galley(egui::pos2(x, y), line, colour);
                    y += down;
                }
            } else {
                cell(ui, ink, &says(Sign::None, Some(&said), text));
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
            let count = windows.to_string();
            cell(
                ui,
                ink,
                &says(
                    Sign::Font(icons::APP_WINDOW),
                    Some(&count),
                    text.gamma_multiply(0.7),
                ),
            )
            .hint(if windows == 1 {
                "1 window on this workspace".to_string()
            } else {
                format!("{windows} windows on this workspace")
            });
        }
        PanelModule::Notifications => {
            // (The brand has one bell: while nothing is to disturb, it
            // is the icon font's bell with the stroke through it.)
            let (sign, hint) = if shown.quiet {
                (
                    Sign::Font(icons::BELL_SLASH),
                    "Notifications (do not disturb)",
                )
            } else {
                (
                    Sign::Brand(BrandIcon::Notifications, icons::BELL),
                    "Notifications",
                )
            };
            let pressed = cell(
                ui,
                ink,
                &Cell {
                    pressed: true,
                    ..says(sign, None, text)
                },
            );
            if pressed.hint(hint).clicked() {
                asked.pressed = Some(PanelModule::Notifications);
            }
        }
        PanelModule::Power => {
            let pressed = cell(
                ui,
                ink,
                &Cell {
                    pressed: true,
                    ..says(Sign::Brand(BrandIcon::Power, icons::POWER), None, text)
                },
            );
            if pressed.hint("Power").clicked() {
                asked.pressed = Some(PanelModule::Power);
            }
        }
        PanelModule::ControlCentre => {
            let pressed = cell(
                ui,
                ink,
                &Cell {
                    pressed: true,
                    ..says(Sign::Font(icons::SLIDERS_HORIZONTAL), None, text)
                },
            );
            if pressed.hint("Control centre").clicked() {
                asked.pressed = Some(PanelModule::ControlCentre);
            }
        }
        PanelModule::Tray => {
            let from_right = ui.layout().main_dir() == egui::Direction::RightToLeft;
            draw_tray(ui, shown, ink, asked, from_right);
        }
        PanelModule::Sound => {
            if let Some((percent, muted)) = shown.system.volume {
                // (Upright down a side there is no room for the sign
                // of percent.)
                let said = if ink.short() {
                    percent.to_string()
                } else {
                    format!("{percent}%")
                };
                let colour = if muted { quiet } else { text };
                let hint = if muted {
                    "Sound: muted. A press lets it be heard; the wheel makes it louder and quieter; right-click chooses the device."
                } else {
                    "Sound. A press mutes it; the wheel makes it louder and quieter; right-click chooses the device."
                };
                // (The brand's sign where it has one for how it is: it
                // has none for "muted" and "no sound".)
                let sign = match sound_sign(percent, muted) {
                    Some(sign) => Sign::Brand(sign, sound_icon(percent, muted)),
                    None => Sign::Font(sound_icon(percent, muted)),
                };
                let button = cell(
                    ui,
                    ink,
                    &Cell {
                        pressed: true,
                        ..says(sign, Some(&said), colour)
                    },
                )
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
                let said = if ink.short() {
                    percent.to_string()
                } else {
                    format!("{percent}%")
                };
                let response = cell(ui, ink, &says(Sign::Font(icons::SUN), Some(&said), text))
                    .hint("Brightness. The wheel makes it brighter and darker.");
                // (The wheel over it is its own, as over the sound.)
                if response.contains_pointer() {
                    asked.over_brightness = true;
                }
            }
        }
        PanelModule::Battery => {
            if let Some(battery) = shown.system.battery {
                let said = if ink.short() {
                    battery.percent.to_string()
                } else {
                    format!("{}%", battery.percent)
                };
                let low = !battery.charging && battery.percent < 15;
                let colour = if low {
                    egui::Color32::from_rgb(240, 110, 90)
                } else {
                    text
                };
                // (The brand's battery, but while it is charged: for
                // that the pack has no sign.)
                let sign = if battery.charging {
                    Sign::Font(battery_icon(battery))
                } else {
                    Sign::Brand(BrandIcon::Battery, battery_icon(battery))
                };
                cell(ui, ink, &says(sign, Some(&said), colour)).hint(if battery.charging {
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
                let colour = if network.connected { text } else { quiet };
                let hint = if network.name.is_empty() {
                    hint.to_string()
                } else {
                    format!("{hint}: {}", network.name)
                };
                // A press opens the control centre, where the network is
                // named with the rest of what is set often.
                // (The brand's sign is the wireless one: a cable and
                // "none" stay the icon font's.)
                let sign = if network.connected && network.wireless {
                    Sign::Brand(BrandIcon::Network, icon)
                } else {
                    Sign::Font(icon)
                };
                let pressed = cell(
                    ui,
                    ink,
                    &Cell {
                        pressed: true,
                        ..says(sign, None, colour)
                    },
                );
                if pressed
                    .hint(format!("{hint}. A press opens the control centre."))
                    .clicked()
                {
                    asked.pressed = Some(PanelModule::Network);
                }
            }
        }
        PanelModule::Taskbar => draw_tasks(ui, shown, ink, asked),
        PanelModule::Keyboard => {
            if let Some(layout) = shown.keyboard {
                // Its short word; the hint has the whole name and what a
                // press does.
                let short = layout.short.to_uppercase();
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
                let response = cell(
                    ui,
                    ink,
                    &Cell {
                        sign: Sign::None,
                        words: Some(&short),
                        colour: text,
                        fill: Some(text.gamma_multiply(0.07)),
                        pressed: layout.count > 1,
                    },
                )
                .hint(hint);
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
        PanelModule::Separator => {
            // A thin line across the panel, shorter than it is thick.
            let (rect, _) = ui.allocate_exact_size(along(7.0), egui::Sense::hover());
            let stroke = egui::Stroke::new(1.0, text.gamma_multiply(0.22));
            let reach = (across * 0.32).max(4.0);
            if upright {
                ui.painter().hline(
                    (rect.center().x - reach)..=(rect.center().x + reach),
                    rect.center().y,
                    stroke,
                );
            } else {
                ui.painter().vline(
                    rect.center().x,
                    (rect.center().y - reach)..=(rect.center().y + reach),
                    stroke,
                );
            }
        }
        PanelModule::Load => {
            if let Some(load) = shown.extras.load {
                // The processor, and how full the memory is. (Upright and
                // short: the processor alone.)
                let said = if ink.short() {
                    load.processor.to_string()
                } else {
                    format!("{}%  {}%", load.processor, load.memory)
                };
                let colour = if load.processor >= 90 || load.memory >= 90 {
                    egui::Color32::from_rgb(240, 110, 90)
                } else {
                    text.gamma_multiply(0.8)
                };
                cell(ui, ink, &says(Sign::Font(icons::CPU), Some(&said), colour)).hint(format!(
                    "The processor is {} % busy; the memory is {} % full.",
                    load.processor, load.memory
                ));
            }
        }
        PanelModule::Media => {
            if let Some((line, playing)) = &shown.extras.playing {
                // What is on, cut to what a panel has room for; a press
                // pauses it or lets it go on, the other button is the
                // next piece.
                let most = if upright { 28 } else { 36 };
                let said: String = if line.chars().count() > most {
                    line.chars()
                        .take(most - 1)
                        .chain(std::iter::once('…'))
                        .collect()
                } else {
                    line.clone()
                };
                let button = cell(
                    ui,
                    ink,
                    &Cell {
                        sign: Sign::Font(if *playing { icons::PAUSE } else { icons::PLAY }),
                        words: (!ink.short()).then_some(said.as_str()),
                        colour: if *playing { text } else { quiet },
                        fill: None,
                        pressed: true,
                    },
                )
                .hint(format!(
                    "{line}. A press {}; right-click is the next piece.",
                    if *playing {
                        "pauses it"
                    } else {
                        "lets it go on"
                    }
                ));
                if button.clicked() {
                    asked.media = Some(MediaAsk::PlayPause);
                }
                if button.secondary_clicked() {
                    asked.media = Some(MediaAsk::Next);
                }
            }
        }
        PanelModule::Custom => {
            // What its program wrote, else its own words; nothing where
            // there is neither.
            let said = shown
                .extras
                .custom
                .as_deref()
                .unwrap_or(bar.custom.text.as_str())
                .trim();
            if !said.is_empty() {
                let starts = !bar.custom.on_press.is_empty();
                let button = cell(
                    ui,
                    ink,
                    &Cell {
                        pressed: starts,
                        ..says(Sign::None, Some(said), text)
                    },
                );
                if starts && button.clicked() {
                    asked.custom_pressed = true;
                }
            }
        }
        // Nothing to show now, or not known to this version.
        PanelModule::Layout | PanelModule::Title | PanelModule::Other(_) => {}
    }
}

/// A button of the taskbar is not shorter than this along the panel: a
/// window's picture, or a letter of its name.
pub const TASK_LEAST: f32 = 28.0;

/// A button narrower than this has no room for words beside its picture:
/// the picture alone. (One without a picture: a letter of its name.)
const TASK_WORDS: f32 = 72.0;
const TASK_WORDS_ALONE: f32 = 48.0;

/// How long the button is that stands for the windows there is no room
/// for.
pub const TASK_MORE: f32 = 30.0;

/// Where the taskbar notes how long it came to be in this frame, for the
/// group it stands in.
fn task_long_id() -> egui::Id {
    egui::Id::unique("gevrek panel taskbar long")
}

/// How long each of `count` buttons of the taskbar is, and how many of
/// them stand, in a room of `room` with `gap` between two buttons.
///
/// All of them stand while each has [`TASK_LEAST`] at least: `wanted`
/// long, or shorter where that is more than its share of the room. Where
/// not even that is there, as many of the shortest as there is room for
/// beside the button that has the rest ([`TASK_MORE`]).
pub fn task_fit(room: f32, gap: f32, count: usize, wanted: f32) -> (f32, usize) {
    if count == 0 {
        return (TASK_LEAST, 0);
    }
    let each = (room - gap * (count as f32 - 1.0)) / count as f32;
    if each >= TASK_LEAST {
        return (each.min(wanted.max(TASK_LEAST)), count);
    }
    let fits = ((room - TASK_MORE) / (TASK_LEAST + gap)).floor().max(0.0) as usize;
    (TASK_LEAST, fits.min(count - 1))
}

/// What the windows of one program are known together by: what their
/// picture is looked for by, else what the program calls its windows.
/// None for a window that says neither (it stands alone).
fn task_program(task: &Task) -> Option<&str> {
    [task.icon.as_str(), task.class.as_str()]
        .into_iter()
        .map(str::trim)
        .find(|said| !said.is_empty())
}

/// The buttons of the taskbar: the windows each stands for, in the order
/// the windows have (a program's button where its first window is).
/// `all_fit`: there is room for a button for every window.
pub fn task_buttons(tasks: &[Task], grouping: TaskGrouping, all_fit: bool) -> Vec<Vec<&Task>> {
    let together = match grouping {
        TaskGrouping::Never => false,
        TaskGrouping::WhenFull => !all_fit,
        TaskGrouping::Always => true,
    };
    let mut buttons: Vec<Vec<&Task>> = Vec::new();
    for task in tasks {
        let program = task_program(task).filter(|_| together);
        let place = program.and_then(|program| {
            buttons
                .iter()
                .position(|button| task_program(button[0]) == Some(program))
        });
        match place {
            Some(place) => buttons[place].push(task),
            None => buttons.push(vec![task]),
        }
    }
    buttons
}

/// The windows of these ids as the view has them, in the order asked.
pub fn tasks_of_ids(view: &View, ids: &[u64]) -> Vec<Task> {
    ids.iter()
        .filter_map(|id| {
            view.all_tasks
                .iter()
                .chain(&view.tasks)
                .find(|task| task.id == *id)
                .cloned()
        })
        .collect()
}

/// How strong the words and the line around a minimized window's button
/// are, of the panel's text: fainter than any other button's, and still
/// above seven to one in the high-contrast palettes (white on black, and
/// black on white).
const MINIMIZED_INK: f32 = 0.7;

/// How a window's button in the taskbar is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TaskLook {
    fill: egui::Color32,
    /// The line around it, where it has one.
    outline: Option<egui::Color32>,
    /// The accent's line at its edge: it has the keyboard.
    keys_line: bool,
    /// What its picture is drawn through (white: as it is).
    picture: egui::Color32,
}

/// The look of a window's button: `text` is the colour its words have,
/// `hovered` that the pointer is over it, `has_keys` that its window has
/// the keyboard, `minimized` that every window it stands for was
/// minimized by the person.
///
/// A minimized window's button is hollow: a line around it in place of a
/// fill, its picture faded, and no line of the accent's (nothing of it is
/// to be seen). That it is hollow says it as well as its colour does, so
/// it reads in a high-contrast palette too.
fn task_look(
    text: egui::Color32,
    accent: egui::Color32,
    hovered: bool,
    has_keys: bool,
    minimized: bool,
) -> TaskLook {
    if minimized {
        return TaskLook {
            fill: if hovered {
                text.gamma_multiply(0.10)
            } else {
                egui::Color32::TRANSPARENT
            },
            outline: Some(text.gamma_multiply(MINIMIZED_INK)),
            keys_line: false,
            picture: egui::Color32::WHITE.gamma_multiply(0.5),
        };
    }
    let fill = match (hovered, has_keys) {
        (true, true) => accent.gamma_multiply(0.36),
        (false, true) => accent.gamma_multiply(0.24),
        (true, false) => text.gamma_multiply(0.14),
        (false, false) => text.gamma_multiply(0.06),
    };
    TaskLook {
        fill,
        outline: None,
        keys_line: has_keys,
        picture: egui::Color32::WHITE,
    }
}

/// The command that gives a window the keyboard. One the person minimized
/// is brought back by it first, to its own workspace, on its display (the
/// window manager does that for a window it is asked to focus).
pub fn go_to_window(id: u64) -> String {
    serde_json::json!({"command": "focus_window", "selector": {"kind": "handle", "value": id}})
        .to_string()
}

/// The command that closes a window (the middle button on its button of
/// the taskbar).
pub fn close_window(id: u64) -> String {
    serde_json::json!({"command": "close_window", "selector": {"kind": "handle", "value": id}})
        .to_string()
}

/// What a panel's later modules show on this panel: what all panels
/// show of it, and what this panel's own program wrote.
pub fn extras_of(
    load: Option<gevrek_system::status::Load>,
    playing: Option<(String, bool)>,
    custom: &std::collections::HashMap<String, String>,
    panel: &str,
) -> Extras {
    Extras {
        load,
        playing,
        custom: custom.get(panel).cloned(),
    }
}

/// The taskbar: its own button (whose windows it has), and a button for
/// each window. It is never longer than the room it has (`ink.room`):
/// the buttons get shorter, down to a window's picture alone; then the
/// windows of one program stand under one button (where the settings let
/// them); and the windows there is still no room for are behind a button
/// that lists them.
fn draw_tasks(ui: &mut egui::Ui, shown: &Shown, ink: &Ink, asked: &mut Asked) {
    let (bar, view, accent) = (shown.bar, shown.view, ink.accent);
    let upright = ink.upright;
    let across = across_of(ui, upright);
    // (Along the panel, across it.)
    let along = |long: f32| {
        if upright {
            egui::vec2(across, long)
        } else {
            egui::vec2(long, across)
        }
    };
    let quiet = ink.text.gamma_multiply(0.55);
    // Whose windows it has: every workspace's, or the shown one's. Its
    // own small button says which, and changes it.
    let all = bar.tasks_of_all_workspaces;
    let tasks = if all { &view.all_tasks } else { &view.tasks };
    let gap = if upright {
        ui.spacing().item_spacing.y
    } else {
        ui.spacing().item_spacing.x
    };
    // The room the windows' buttons have: the taskbar's, less its own
    // button (as long as `cell` makes one that has a sign and no words).
    let own = if upright { 27.0 } else { across.max(26.0) };
    let room = (ink.room - own - gap).max(0.0);
    // Along a panel that lies: as wide as the setting says, or narrower
    // when the buttons would not fit beside each other. Down a side: a
    // square each.
    let natural = !upright && bar.task_width == 0;
    let wanted = if upright {
        TASK_LEAST
    } else if natural {
        f32::INFINITY
    } else {
        bar.task_width as f32
    };
    let all_fit = task_fit(room, gap, tasks.len(), wanted).1 == tasks.len();
    let buttons = task_buttons(tasks, bar.task_grouping, all_fit);
    let (each, fits) = task_fit(room, gap, buttons.len(), wanted);

    // What each button says, and how long it is.
    struct Button<'a> {
        windows: &'a [&'a Task],
        /// The window it shows: the one with the keyboard among its own.
        chosen: &'a Task,
        picture: Option<egui::TextureHandle>,
        has_keys: bool,
        /// Every window it stands for is minimized.
        minimized: bool,
        colour: egui::Color32,
        long: f32,
    }
    let font = egui::FontId::proportional(12.5);
    let mut drawn: Vec<Button> = buttons
        .iter()
        .take(fits)
        .map(|windows| {
            let chosen = windows
                .iter()
                .find(|task| task.focused && task.shown)
                .or_else(|| windows.iter().find(|task| task.focused))
                .or_else(|| windows.iter().find(|task| !task.minimized))
                .copied()
                .unwrap_or(windows[0]);
            let minimized = windows.iter().all(|task| task.minimized);
            // The one with the keyboard stands out, as the shown
            // workspace does among the tabs, more quietly.
            let has_keys = view.focused && windows.iter().any(|task| task.focused && task.shown);
            // (One on a workspace that is not shown reads fainter.)
            let text = if windows.iter().any(|task| task.shown) {
                ink.text
            } else {
                ink.text.gamma_multiply(0.6)
            };
            Button {
                windows: windows.as_slice(),
                chosen,
                picture: task_picture(ui.ctx(), &chosen.icon),
                has_keys,
                minimized,
                colour: if has_keys {
                    text
                } else if minimized {
                    text.gamma_multiply(MINIMIZED_INK)
                } else {
                    text.gamma_multiply(0.78)
                },
                long: each,
            }
        })
        .collect();
    // (The room a button's picture takes before its words, and the count
    // of its windows after them.)
    let before = |button: &Button| if button.picture.is_some() { 22.0 } else { 0.0 };
    // (A window kept on top has a pin there, too.)
    let on_top = |button: &Button| button.windows.iter().any(|task| task.on_top);
    let after = |button: &Button| {
        let count = if button.windows.len() > 1 { 16.0 } else { 0.0 };
        count + if on_top(button) { 14.0 } else { 0.0 }
    };
    if natural {
        // Each as wide as its words, while they all have room so; else
        // none wider than its share.
        let wide: Vec<f32> = drawn
            .iter()
            .map(|button| {
                let words = ui.painter().layout_no_wrap(
                    task_label(button.chosen),
                    font.clone(),
                    button.colour,
                );
                words.size().x + 18.0 + before(button) + after(button)
            })
            .collect();
        let together = wide.iter().sum::<f32>() + gap * (wide.len() as f32 - 1.0).max(0.0);
        for (button, wide) in drawn.iter_mut().zip(wide) {
            button.long = if together <= room {
                wide.max(TASK_LEAST)
            } else {
                wide.clamp(TASK_LEAST, each)
            };
        }
    }
    // The windows there is no room for.
    let hidden: Vec<&Task> = buttons.iter().skip(fits).flatten().copied().collect();

    // In the order they stand along the panel (where the panel's end is
    // laid out from its edge inwards, the last comes first).
    enum Piece {
        Own,
        Window(usize),
        More,
    }
    let mut pieces = vec![Piece::Own];
    pieces.extend((0..drawn.len()).map(Piece::Window));
    if !hidden.is_empty() {
        pieces.push(Piece::More);
    }
    if matches!(
        ui.layout().main_dir(),
        egui::Direction::RightToLeft | egui::Direction::BottomUp
    ) {
        pieces.reverse();
    }
    let mut reached: Option<egui::Rect> = None;
    let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    for piece in pieces {
        let place = match piece {
            Piece::Own => {
                let scope = cell(
                    ui,
                    ink,
                    &Cell {
                        sign: Sign::Font(if all { icons::STACK } else { icons::SQUARE }),
                        words: None,
                        colour: if all { accent } else { quiet },
                        fill: None,
                        pressed: true,
                    },
                )
                .hint(if all {
                    "The windows of every workspace. A press shows this workspace's only."
                } else {
                    "The windows of this workspace. A press shows every workspace's."
                });
                if scope.clicked() {
                    asked.all_tasks = Some(!all);
                }
                scope.rect
            }
            Piece::Window(index) => {
                let button = &drawn[index];
                let many = button.windows.len() > 1;
                // (No room for words: its picture alone, or a letter.)
                let narrow = upright
                    || button.long
                        < if button.picture.is_some() {
                            TASK_WORDS
                        } else {
                            TASK_WORDS_ALONE
                        };
                let label = task_label(button.chosen);
                let said = if !narrow {
                    label
                } else if button.picture.is_none() {
                    label
                        .chars()
                        .next()
                        .map(|letter| letter.to_uppercase().to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let mut job =
                    egui::text::LayoutJob::simple_singleline(said, font.clone(), button.colour);
                if !narrow {
                    // A button is as wide as it is, whatever its title
                    // says: a title that changes moves no button.
                    job.wrap = egui::text::TextWrapping::truncate_at_width(
                        (button.long - 18.0 - before(button) - after(button)).max(8.0),
                    );
                }
                let words = ui.fonts_mut(|fonts| fonts.layout_job(job));
                let (place, response) =
                    ui.allocate_exact_size(along(button.long), egui::Sense::click());
                let look = task_look(
                    ink.text,
                    accent,
                    response.hovered(),
                    button.has_keys,
                    button.minimized,
                );
                ui.painter().rect_filled(place, 8.0, look.fill);
                if let Some(outline) = look.outline {
                    ui.painter().rect_stroke(
                        place,
                        8.0,
                        egui::Stroke::new(1.0, outline),
                        egui::StrokeKind::Inside,
                    );
                }
                if look.keys_line {
                    // A line of the accent's at its edge says it again.
                    let line = if upright {
                        egui::Rect::from_min_max(
                            egui::pos2(place.left(), place.top() + 7.0),
                            egui::pos2(place.left() + 2.0, place.bottom() - 7.0),
                        )
                    } else {
                        let in_by = (place.width() / 2.0 - 4.0).min(10.0);
                        egui::Rect::from_min_max(
                            egui::pos2(place.left() + in_by, place.bottom() - 2.0),
                            egui::pos2(place.right() - in_by, place.bottom()),
                        )
                    };
                    ui.painter().rect_filled(line, 1.0, accent);
                }
                match (&button.picture, narrow) {
                    (Some(picture), true) => {
                        let side = if upright { 18.0 } else { 16.0 };
                        ui.painter().image(
                            picture.id(),
                            egui::Rect::from_center_size(place.center(), egui::Vec2::splat(side)),
                            whole,
                            look.picture,
                        );
                    }
                    (Some(picture), false) => {
                        ui.painter().image(
                            picture.id(),
                            egui::Rect::from_center_size(
                                egui::pos2(place.left() + 17.0, place.center().y),
                                egui::Vec2::splat(16.0),
                            ),
                            whole,
                            look.picture,
                        );
                        ui.painter().galley(
                            egui::pos2(
                                place.left() + 9.0 + before(button),
                                place.center().y - words.size().y / 2.0,
                            ),
                            words,
                            button.colour,
                        );
                    }
                    (None, true) => {
                        ui.painter().galley(
                            place.center() - words.size() / 2.0,
                            words,
                            button.colour,
                        );
                    }
                    (None, false) => {
                        // (In the middle of what the count leaves.)
                        let middle = place.center() - egui::vec2(after(button) / 2.0, 0.0);
                        ui.painter()
                            .galley(middle - words.size() / 2.0, words, button.colour);
                    }
                }
                if many {
                    // How many windows it stands for: after its words,
                    // or small in its corner where it has none.
                    let count = button.windows.len().to_string();
                    if narrow {
                        ui.painter().text(
                            place.right_bottom() - egui::vec2(2.0, 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            count,
                            egui::FontId::proportional(9.0),
                            ink.text,
                        );
                    } else {
                        ui.painter().text(
                            egui::pos2(place.right() - 8.0, place.center().y),
                            egui::Align2::RIGHT_CENTER,
                            count,
                            egui::FontId::proportional(11.0),
                            quiet,
                        );
                    }
                }
                if on_top(button) {
                    // Kept on top: a pin of the accent's, before the count
                    // where there are words, else small in its upper
                    // corner (the count has the lower one).
                    let (at, size) = if narrow {
                        (place.right_top() + egui::vec2(-5.0, 5.0), 9.0)
                    } else {
                        let count = if many { 16.0 } else { 0.0 };
                        (
                            egui::pos2(place.right() - 13.0 - count, place.center().y),
                            11.0,
                        )
                    };
                    ui.painter().text(
                        at,
                        egui::Align2::CENTER_CENTER,
                        icons::PUSH_PIN,
                        egui::FontId::proportional(size),
                        accent,
                    );
                }
                if many {
                    let response = response.hint(format!(
                        "{} windows: {}. A press lists them.",
                        button.windows.len(),
                        task_label(button.chosen)
                    ));
                    // The windows it stands for, to choose one from.
                    if response.clicked() {
                        let ids = button.windows.iter().map(|task| task.id).collect();
                        asked.task_menu = Some((response.rect, ids));
                    }
                } else {
                    let response = if button.minimized {
                        response.hint(format!(
                            "{} (minimized). A press brings it back.",
                            button.chosen.title
                        ))
                    } else if button.chosen.on_top {
                        response.hint(format!("{} (always on top)", button.chosen.title))
                    } else {
                        response.hint(&button.chosen.title)
                    };
                    if response.clicked() && !button.has_keys {
                        asked.go = Some(button.chosen.id);
                    }
                    // The middle button closes the window, as on a tab.
                    if response.middle_clicked() {
                        asked.close_window = Some(button.chosen.id);
                    }
                    // The other button: the menu of what can be done with
                    // the window (`task_actions`).
                    if response.secondary_clicked() {
                        asked.task_actions = Some((response.rect, button.chosen.id));
                    }
                }
                place
            }
            Piece::More => {
                // Those there is no room for are said to be there, and a
                // press lists them. (It is the accent's where the window
                // with the keyboard is among them.)
                let (place, response) =
                    ui.allocate_exact_size(along(TASK_MORE), egui::Sense::click());
                let has_keys = view.focused && hidden.iter().any(|task| task.focused && task.shown);
                let fill = match (response.hovered(), has_keys) {
                    (true, true) => accent.gamma_multiply(0.36),
                    (false, true) => accent.gamma_multiply(0.24),
                    (true, false) => ink.text.gamma_multiply(0.14),
                    (false, false) => ink.text.gamma_multiply(0.06),
                };
                ui.painter().rect_filled(place, 8.0, fill);
                ui.painter().text(
                    place.center(),
                    egui::Align2::CENTER_CENTER,
                    if hidden.len() > 99 {
                        "+".to_string()
                    } else {
                        format!("+{}", hidden.len())
                    },
                    egui::FontId::proportional(if hidden.len() > 9 { 10.5 } else { 12.0 }),
                    if has_keys { ink.text } else { quiet },
                );
                let response = response.hint(format!(
                    "{} more windows. A press lists them.",
                    hidden.len()
                ));
                if response.clicked() {
                    let ids = hidden.iter().map(|task| task.id).collect();
                    asked.task_menu = Some((response.rect, ids));
                }
                place
            }
        };
        reached = Some(reached.map_or(place, |reached| reached.union(place)));
    }
    // How long it came to be: its group gives the rest of its room to
    // what else stands in it.
    let long = reached.map_or(0.0, |reached| {
        if upright {
            reached.height()
        } else {
            reached.width()
        }
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(task_long_id(), long));
}

/// How many windows the list of them shows at once (more are scrolled).
const TASK_LINES: usize = 12;

/// How large the list of windows under a button of the taskbar is, in
/// points.
pub fn task_menu_size(count: usize) -> egui::Vec2 {
    egui::vec2(340.0, 14.0 + 32.0 * count.clamp(1, TASK_LINES) as f32)
}

/// The list under a button of the taskbar that stands for several
/// windows: each with its picture and its title, the one with the
/// keyboard marked. The window that was pressed, if one was.
pub fn task_menu(ui: &mut egui::Ui, palette: &theme::Palette, tasks: &[Task]) -> Option<u64> {
    let mut chosen = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            let (on, chose) = menu_keys(ui.ctx(), tasks.len(), |_| true, false);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (index, task) in tasks.iter().enumerate() {
                        let words = if task.title.trim().is_empty() {
                            task.class.trim()
                        } else {
                            task.title.trim()
                        };
                        let name: String = words.chars().take(42).collect();
                        // (A minimized one in the quiet colour, its
                        // picture faded, and said to be: as its button.)
                        let label = if task.minimized {
                            egui::RichText::new(format!("{name} (minimized)"))
                                .size(13.0)
                                .color(palette.muted)
                        } else {
                            egui::RichText::new(name).size(13.0)
                        };
                        let marked = task.focused && task.shown;
                        let faded = if task.minimized {
                            egui::Color32::WHITE.gamma_multiply(0.5)
                        } else {
                            egui::Color32::WHITE
                        };
                        let button = match task_picture(ui.ctx(), &task.icon) {
                            Some(picture) => egui::Button::image_and_text(
                                egui::Image::new((picture.id(), egui::Vec2::splat(16.0)))
                                    .tint(faded),
                                label,
                            )
                            .selected(marked),
                            None => egui::Button::selectable(marked, label),
                        }
                        .min_size(egui::vec2(ui.available_width(), 28.0));
                        let response = ui.add(button);
                        if on == Some(index) {
                            menu_mark(ui, response.rect, palette);
                            response.scroll_to_me(None);
                        }
                        if response.clicked() || (chose && on == Some(index)) {
                            chosen = Some(task.id);
                        }
                    }
                });
        });
    chosen
}

/// What can be done with a window of the taskbar: the menu the other
/// button opens on its button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAction {
    /// To the middle of this display, floating; chosen again for the
    /// window, back as it was.
    Centre,
    /// Kept above the other windows, or not: a floating window only, one
    /// on each workspace (the window manager lets the one before go).
    OnTop,
}

impl TaskAction {
    /// The lines of the menu, in order.
    pub const ALL: [TaskAction; 2] = [TaskAction::Centre, TaskAction::OnTop];

    /// Whether it can be chosen for this window.
    pub fn live(self, task: &Task) -> bool {
        match self {
            TaskAction::Centre => true,
            TaskAction::OnTop => task.floating || task.on_top,
        }
    }

    /// What its line says for this window.
    pub fn words(self, task: &Task) -> &'static str {
        match self {
            TaskAction::Centre => "Centre on this display, or put back",
            TaskAction::OnTop if self.live(task) => "Always on top",
            TaskAction::OnTop => "Always on top (floating windows only)",
        }
    }

    /// The command that does it for this window, on this panel's display.
    pub fn command(self, view: &View, task: &Task) -> Option<String> {
        use gevrek_model::command::{DispatchCommand, WindowSelector};
        match self {
            TaskAction::Centre => centre_window(view, task.id),
            TaskAction::OnTop => serde_json::to_string(&DispatchCommand::SetWindowPinned {
                selector: WindowSelector::Handle(task.id as i64),
                enabled: !task.on_top,
            })
            .ok(),
        }
    }
}

/// How large the menu of a window of the taskbar is, in points.
pub fn task_actions_size() -> egui::Vec2 {
    egui::vec2(
        320.0,
        12.0 + (TRAY_LINE + 2.0) * TaskAction::ALL.len() as f32,
    )
}

/// The menu of a window of the taskbar (the other button on its button):
/// a line for each [`TaskAction`], the one that is on marked. A line that
/// cannot be chosen for this window is read, not pressed. What was
/// chosen, if anything was.
pub fn task_actions(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    task: &Task,
) -> Option<TaskAction> {
    let mut chosen = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let actions = TaskAction::ALL;
            let (on, chose) = menu_keys(
                ui.ctx(),
                actions.len(),
                |index| actions[index].live(task),
                false,
            );
            let font = egui::FontId::proportional(13.0);
            for (index, action) in actions.into_iter().enumerate() {
                let live = action.live(task);
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
                // (A check where it is on, as a menu's switch has.)
                let (sign, marked) = match action {
                    TaskAction::Centre => (icons::ARROWS_IN, false),
                    TaskAction::OnTop => (icons::PUSH_PIN, task.on_top),
                };
                if marked {
                    ui.painter().text(
                        egui::pos2(line.left() + 17.0, line.center().y),
                        egui::Align2::CENTER_CENTER,
                        icons::CHECK,
                        font.clone(),
                        palette.accent,
                    );
                }
                ui.painter().text(
                    egui::pos2(line.left() + 39.0, line.center().y),
                    egui::Align2::CENTER_CENTER,
                    sign,
                    font.clone(),
                    ink,
                );
                ui.painter().text(
                    egui::pos2(line.left() + 56.0, line.center().y),
                    egui::Align2::LEFT_CENTER,
                    action.words(task),
                    font.clone(),
                    ink,
                );
                if live && (response.clicked() || (chose && on == Some(index))) {
                    chosen = Some(action);
                }
            }
        });
    chosen
}

/// What a panel's own menu can be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAsk {
    /// Open the settings at the panels.
    Settings,
    /// Another panel, made as this model.
    Add(gevrek_model::settings::PanelModel),
    /// This panel at another edge of its displays.
    Move(PanelEdge),
    /// This panel out of sight until the pointer comes to its edge, or
    /// always there.
    Hide(bool),
    /// This panel taken away.
    Remove,
}

/// The lines of a panel's own menu, in their order: what each asks, its
/// sign and its words, and whether it is on already (marked, and not
/// to be chosen again).
fn panel_lines(bar: &PanelProfile, panels: usize) -> Vec<(PanelAsk, &'static str, String, bool)> {
    use gevrek_model::settings::{PanelHide, PanelModel};
    let mut lines = vec![(
        PanelAsk::Settings,
        icons::SLIDERS_HORIZONTAL,
        "Panel settings".to_string(),
        false,
    )];
    for (edge, sign, words) in [
        (PanelEdge::Top, icons::ARROW_UP, "Move to the top"),
        (PanelEdge::Bottom, icons::ARROW_DOWN, "Move to the bottom"),
        (PanelEdge::Left, icons::ARROW_LEFT, "Move to the left side"),
        (
            PanelEdge::Right,
            icons::ARROW_RIGHT,
            "Move to the right side",
        ),
    ] {
        lines.push((
            PanelAsk::Move(edge),
            sign,
            words.to_string(),
            bar.edge == edge,
        ));
    }
    let hidden = bar.hide == PanelHide::Always;
    lines.push((
        PanelAsk::Hide(!hidden),
        icons::EYE_SLASH,
        "Hide until the pointer comes to its edge".to_string(),
        hidden,
    ));
    for model in PanelModel::ALL {
        lines.push((
            PanelAsk::Add(model),
            icons::PLUS,
            format!("Add: {}", model.label().to_lowercase()),
            false,
        ));
    }
    lines.push((
        PanelAsk::Remove,
        icons::TRASH,
        if panels > 1 {
            "Remove this panel".to_string()
        } else {
            "Remove this panel (the only one)".to_string()
        },
        false,
    ));
    lines
}

/// How large a panel's own menu is, in points.
pub fn panel_menu_size(bar: &PanelProfile, panels: usize) -> egui::Vec2 {
    egui::vec2(
        340.0,
        12.0 + (TRAY_LINE + 2.0) * panel_lines(bar, panels).len() as f32,
    )
}

/// A panel's own menu (the other button on the panel, where nothing
/// stands): its settings, another edge, out of sight or not, a panel
/// more, and this one taken away. `panels`: how many there are. What was
/// chosen, if anything was.
pub fn panel_menu(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    bar: &PanelProfile,
    panels: usize,
) -> Option<PanelAsk> {
    let mut chosen = None;
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(palette.surface)
                .inner_margin(egui::Margin::same(6)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let lines = panel_lines(bar, panels);
            // (The edge it is at is read, not pressed.)
            let live = |index: usize| !matches!(lines[index], (PanelAsk::Move(_), _, _, true));
            let (on, chose) = menu_keys(ui.ctx(), lines.len(), live, false);
            let font = egui::FontId::proportional(13.0);
            for (index, (ask, sign, words, marked)) in lines.iter().enumerate() {
                let live = live(index);
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
                if *marked {
                    ui.painter().text(
                        egui::pos2(line.left() + 17.0, line.center().y),
                        egui::Align2::CENTER_CENTER,
                        icons::CHECK,
                        font.clone(),
                        palette.accent,
                    );
                }
                ui.painter().text(
                    egui::pos2(line.left() + 39.0, line.center().y),
                    egui::Align2::CENTER_CENTER,
                    *sign,
                    font.clone(),
                    ink,
                );
                ui.painter().text(
                    egui::pos2(line.left() + 56.0, line.center().y),
                    egui::Align2::LEFT_CENTER,
                    words,
                    font.clone(),
                    ink,
                );
                if live && (response.clicked() || (chose && on == Some(index))) {
                    chosen = Some(*ask);
                }
            }
        });
    chosen
}

/// Carry out what a panel's own menu was asked, on a configuration:
/// whether it is another configuration now. (`Settings` changes none: it
/// opens the settings.)
pub fn panel_asked(config: &mut gevrek_model::config::Config, panel: &str, ask: PanelAsk) -> bool {
    use gevrek_model::settings::PanelHide;
    let shell = &mut config.desktop.shell;
    match ask {
        PanelAsk::Settings => false,
        PanelAsk::Add(model) => {
            shell.add_panel(model.panel());
            true
        }
        PanelAsk::Remove => shell.remove_panel(panel),
        PanelAsk::Move(edge) => match shell.panel_of_mut(panel) {
            Some(own) if own.edge != edge => {
                own.edge = edge;
                true
            }
            _ => false,
        },
        PanelAsk::Hide(hide) => match shell.panel_of_mut(panel) {
            Some(own) => {
                let wanted = if hide {
                    PanelHide::Always
                } else {
                    PanelHide::Never
                };
                let other = own.hide != wanted;
                own.hide = wanted;
                other
            }
            None => false,
        },
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
            minimized: false,
            floating: false,
            on_top: false,
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
            "window.drag_ended",
            "window.workspace_changed",
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
        // A window let go of, or told to be on another workspace or
        // display: asked, whichever window it is.
        assert!(concerns("window.drag_ended", Some(8), &known));
        assert!(concerns("window.workspace_changed", None, &known));
    }

    /// Two displays side by side as the window manager tells of them,
    /// each with two workspaces. The rectangles are what they are on a
    /// desktop in use, and say nothing of whose a window is: `2` floats
    /// across the edge with most of it on the second display, `3` is on a
    /// workspace that is put away (below its display), `4` is minimized,
    /// and `6` is on its way in from beside its display, over the first.
    fn two_displays() -> Value {
        let window = |id: u64, title: &str, x: i32, y: i32, focused: bool| {
            json!({"window_id": id, "title": title, "class": "app", "is_focused": focused,
                   "position": {"x": x, "y": y}, "size": {"width": 1600, "height": 900}})
        };
        json!({"monitors": [
            {"name": "Monitor GSM5B7F", "connector": "", "is_primary": true, "is_focused": true,
             "position": {"x": 0, "y": 0}, "size": {"width": 2560, "height": 1440},
             "workspace": {"id": 0, "name": "Saturn"},
             "workspaces": [
                {"id": 0, "name": "Saturn", "layout_type": "Dwindle", "total_windows": 2,
                 "detailed_windows": [window(1, "editor", 0, 40, true),
                                      window(2, "notes", 1700, 200, false)]},
                {"id": 1, "name": "Web", "layout_type": "Floating", "total_windows": 2,
                 "detailed_windows": [window(3, "browser", 300, 1500, true),
                                      window(4, "mail", -32000, -32000, false)]}]},
            {"name": "Monitor GSM5C4D", "connector": "", "is_primary": false, "is_focused": false,
             "position": {"x": 2560, "y": 0}, "size": {"width": 2560, "height": 1440},
             "workspace": {"id": 1, "name": "Web"},
             "workspaces": [
                {"id": 0, "name": "Saturn", "layout_type": "Dwindle", "total_windows": 1,
                 "detailed_windows": [window(5, "terminal", 2600, 1500, true)]},
                {"id": 1, "name": "Web", "layout_type": "Floating", "total_windows": 1,
                 "detailed_windows": [window(6, "video", 400, 40, true)]}]},
        ]})
    }

    /// What was reported: a window of one display in the taskbar of the
    /// other's panel. A panel has the windows of its own display's
    /// workspaces, as the window manager lists them, and of no other,
    /// wherever on the desktop their rectangles are.
    #[test]
    fn a_panel_has_its_own_displays_windows_and_no_others() {
        let state = two_displays();
        let ids = |tasks: &[Task]| tasks.iter().map(|task| task.id).collect::<Vec<u64>>();
        let views: Vec<(String, View)> = displays(&state)
            .into_iter()
            .enumerate()
            .map(|(index, (name, monitor))| (name, view_at(monitor, Some(index))))
            .collect();
        let (first, second) = (&views[0].1, &views[1].1);
        assert_eq!(ids(&first.tasks), [1, 2], "the shown workspace's");
        assert_eq!(ids(&first.all_tasks), [1, 2, 3, 4], "every workspace's");
        assert_eq!(ids(&second.tasks), [6]);
        assert_eq!(ids(&second.all_tasks), [5, 6]);
        // No window is on two panels, and none of the state's is on none.
        let mut all: Vec<u64> = ids(&first.all_tasks);
        all.extend(ids(&second.all_tasks));
        all.sort_unstable();
        assert_eq!(all, [1, 2, 3, 4, 5, 6]);
        // The same of the tabs, their counts and the title a panel shows.
        let tabs = |view: &View| {
            view.tabs
                .iter()
                .map(|tab| (tab.name.clone(), tab.shown, tab.windows))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            tabs(first),
            [
                ("Saturn".to_string(), true, 2),
                ("Web".to_string(), false, 2)
            ]
        );
        assert_eq!(
            tabs(second),
            [
                ("Saturn".to_string(), false, 1),
                ("Web".to_string(), true, 1)
            ]
        );
        assert_eq!(
            (first.title.as_str(), second.title.as_str()),
            ("editor", "video")
        );
        assert!(first.focused && !second.focused);
    }

    /// The menu of a window of the taskbar: the window floats and is kept
    /// on top as the state says, a tiled one cannot be kept on top, and
    /// the line asks for the other of what it is.
    #[test]
    fn the_menu_of_a_window_keeps_a_floating_one_on_top_or_lets_it_go() {
        use gevrek_model::command::{DispatchCommand, WindowSelector};
        let monitor = serde_json::json!({
            "is_focused": true, "workspace": {"id": 1}, "workspaces": [
                {"id": 1, "name": "1", "detailed_windows": [
                    {"window_id": 1, "title": "tiled", "is_floating": false},
                    {"window_id": 2, "title": "floating", "is_floating": true},
                    {"window_id": 3, "title": "pinned", "is_floating": true, "is_always_on_top": true},
                ]},
            ]
        });
        let view = view_of(&monitor);
        let [tiled, floating, pinned] = [&view.tasks[0], &view.tasks[1], &view.tasks[2]];
        assert!(!tiled.floating && floating.floating && pinned.on_top && !floating.on_top);
        assert!(TaskAction::Centre.live(tiled));
        assert!(!TaskAction::OnTop.live(tiled));
        assert!(TaskAction::OnTop.live(floating) && TaskAction::OnTop.live(pinned));
        let asked = |task: &Task| -> DispatchCommand {
            serde_json::from_str(&TaskAction::OnTop.command(&view, task).unwrap()).unwrap()
        };
        assert_eq!(
            asked(floating),
            DispatchCommand::SetWindowPinned {
                selector: WindowSelector::Handle(2),
                enabled: true
            }
        );
        assert_eq!(
            asked(pinned),
            DispatchCommand::SetWindowPinned {
                selector: WindowSelector::Handle(3),
                enabled: false
            }
        );
        let centred: DispatchCommand =
            serde_json::from_str(&TaskAction::Centre.command(&view, tiled).unwrap()).unwrap();
        assert!(matches!(
            centred,
            DispatchCommand::ToggleWindowCentered { .. }
        ));
    }

    /// A press on a panel is for that panel's display: its workspace of
    /// that name (both displays have a `Web`), and a window brought to
    /// the middle comes to the middle of that display.
    #[test]
    fn a_press_on_a_panel_is_for_its_own_display() {
        use gevrek_model::command::{DispatchCommand, MonitorTarget, WorkspaceSelector};
        let state = two_displays();
        for (name, monitor) in displays(&state) {
            let view = view_of(monitor);
            let web = view.tabs.iter().position(|tab| tab.name == "Web").unwrap();
            let shown: DispatchCommand =
                serde_json::from_str(&show_tab(&view, web).unwrap()).unwrap();
            assert_eq!(
                shown,
                DispatchCommand::FocusMonitorWorkspace {
                    target: MonitorTarget::Name(name.clone()),
                    workspace: WorkspaceSelector::Name("Web".to_string()),
                }
            );
            let centred: DispatchCommand =
                serde_json::from_str(&centre_window(&view, 3).unwrap()).unwrap();
            assert!(
                matches!(&centred, DispatchCommand::ToggleWindowCentered {
                    target: MonitorTarget::Name(display), ..
                } if *display == name),
                "{centred:?}"
            );
            // The wheel goes round this display's workspaces.
            let next = wheel_to(&view, 1).unwrap();
            assert!(next < view.tabs.len() && !view.tabs[next].shown);
        }
    }

    /// The state after a window was dragged to the other display: it is
    /// that display's, and gone from the one it left.
    #[test]
    fn a_window_that_went_to_the_other_display_is_on_that_panel_alone() {
        let mut state = two_displays();
        let moved = state["monitors"][0]["workspaces"][0]["detailed_windows"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        state["monitors"][1]["workspaces"][1]["detailed_windows"]
            .as_array_mut()
            .unwrap()
            .push(moved);
        let ids = |index: usize| {
            view_of(&state["monitors"][index])
                .all_tasks
                .iter()
                .map(|task| task.id)
                .collect::<Vec<u64>>()
        };
        assert_eq!(ids(0), [1, 3, 4]);
        assert_eq!(ids(1), [5, 6, 2]);
    }

    /// On Linux a panel asks for its display by the connector's name. A
    /// display whose name is what another's connector is called is not
    /// that other one.
    #[test]
    fn a_display_is_found_by_its_connector_before_any_name() {
        let state = json!({"monitors": [
            {"name": "DP-2", "connector": "DP-1", "workspace": {"id": 1},
             "workspaces": [{"id": 1, "name": "one", "detailed_windows": [{"window_id": 1}]}]},
            {"name": "Second", "connector": "DP-2", "workspace": {"id": 2},
             "workspaces": [{"id": 2, "name": "two", "detailed_windows": [{"window_id": 2}]}]},
        ]});
        let ids = |output: &str| {
            view(&state, output)
                .map(|view| view.tasks.iter().map(|task| task.id).collect::<Vec<u64>>())
        };
        assert_eq!(ids("DP-1"), Some(vec![1]));
        assert_eq!(
            ids("DP-2"),
            Some(vec![2]),
            "the display on DP-2, not the one called so"
        );
        assert_eq!(ids("Second"), Some(vec![2]));
        assert_eq!(ids("HDMI-1"), None);
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
        let out = left_out(std::slice::from_ref(&bar));
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
        assert_eq!(left_out(std::slice::from_ref(&bar)).len(), 1);
    }

    #[test]
    fn a_surface_in_the_corner_keeps_clear_of_a_panel_at_the_top() {
        let found = rooms(&windows_state());
        assert_eq!(found.len(), 2);
        // No room for windows is told there: all of the display counts.
        assert_eq!(found[0].work, (0, 0, 2560, 1440));
        assert_eq!(found[0].bars, [(6, 6, 2548, 36)]);
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
            bars: Vec::new(),
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
            bars: vec![(6, 558, 788, 36)],
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
        assert_eq!(
            (left.place.name(), left.place.index, left.place.primary),
            ("DP-1", Some(0), true)
        );
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
        assert_eq!(
            second.desktop.workspaces.len(),
            first.desktop.workspaces.len()
        );
        assert!(second.desktop.validate().is_empty());
        // The keys are what they were.
        assert_eq!(
            serde_json::to_value(&second.desktop.bindings).unwrap(),
            serde_json::to_value(&config.desktop.bindings).unwrap()
        );

        assert!(matches!(
            renaming(Some(&second), &left, "1", "Post"),
            Renaming::Nothing
        ));
        // What another workspace of the display is called: said, not done.
        assert!(matches!(
            renaming(Some(&second), &left, "2", "post"),
            Renaming::Refused(_)
        ));
        assert!(matches!(
            renaming(Some(&second), &left, "2", "3"),
            Renaming::Refused(_)
        ));
        // On the other display the title is free.
        assert!(matches!(
            renaming(Some(&second), &right, "2", "Post"),
            Renaming::Apply(_)
        ));
        assert!(matches!(
            renaming(None, &left, "1", "x"),
            Renaming::Refused(_)
        ));

        // A workspace of the moment is written down for its display, so
        // that the name it was given lasts.
        let Renaming::Apply(kept) = renaming(Some(&second), &left, "12", "Music") else {
            panic!("a workspace of the moment is kept");
        };
        assert_eq!(label(&kept, &left, "12").as_deref(), Some("Music"));
        assert_eq!(label(&kept, &right, "12"), None);
        assert!(kept.desktop.validate().is_empty());
        assert!(matches!(
            renaming(Some(&second), &left, "12", "Post"),
            Renaming::Refused(_)
        ));
    }

    #[test]
    fn a_display_has_the_panels_the_configuration_gives_it() {
        use gevrek_model::settings::{DisplayRef, PanelOn};
        let common = PanelProfile {
            not_on: vec!["DP-2".to_string()],
            ..PanelProfile::default()
        };
        let own = PanelProfile {
            id: "second".to_string(),
            on: PanelOn::Only,
            only_on: vec!["dp-2".to_string()],
            left: vec![PanelModule::Clock],
            ..PanelProfile::default()
        };
        let rail = PanelProfile {
            id: "rail".to_string(),
            edge: PanelEdge::Left,
            ..PanelProfile::default()
        };
        let bars = [common, own, rail];
        // A display the window manager calls one thing and whose
        // connector is another: the panel that names its connector is its.
        let second = DisplayRef {
            names: vec!["Monitor B".to_string(), "DP-2".to_string()],
            index: Some(1),
            primary: false,
        };
        let first = DisplayRef::named("Monitor A");
        let id = |found: Option<&PanelProfile>| found.map(|bar| bar.id.clone());
        assert_eq!(
            id(bar_on(&bars, "second", "Monitor B", &second)).as_deref(),
            Some("second")
        );
        assert_eq!(id(bar_on(&bars, "main", "Monitor B", &second)), None);
        assert_eq!(
            id(bar_on(&bars, "main", "Monitor A", &first)).as_deref(),
            Some("main")
        );
        assert_eq!(id(bar_on(&bars, "second", "Monitor A", &first)), None);
        // One that is on every display is on both.
        assert!(bar_on(&bars, "RAIL", "Monitor A", &first).is_some());
        assert!(bar_on(&bars, "rail", "Monitor B", &second).is_some());
        // Not told which: the first that is on the display.
        assert_eq!(
            id(bar_on(&bars, "", "Monitor B", &second)).as_deref(),
            Some("second")
        );
        assert_eq!(
            id(bar_on(&bars, "", "Monitor A", &first)).as_deref(),
            Some("main")
        );
        // A display known by its name alone, which is the one asked by.
        assert!(bar_on(&bars, "second", "DP-2", &DisplayRef::default()).is_some());
    }

    #[test]
    fn each_panel_of_a_display_has_a_key_and_a_place_of_its_own() {
        let state = json!({"monitors": [
            {"name": "Monitor A",
             "position": {"x": 0.0, "y": 0.0}, "size": {"width": 1920, "height": 1080},
             "bar_position": {"x": 0.0, "y": 0.0}, "bar_size": {"width": 1920, "height": 30},
             "panels": [
                {"id": "main", "position": {"x": 0.0, "y": 0.0}, "size": {"width": 1920, "height": 30}},
                {"id": "rail", "position": {"x": 0.0, "y": 30.0}, "size": {"width": 40, "height": 1050}},
                {"id": "gone", "position": {"x": 0.0, "y": 0.0}, "size": {"width": 0, "height": 30}},
             ]},
            // A window manager that tells of one panel and not which.
            {"name": "Monitor B",
             "bar_position": {"x": 1920.0, "y": 0.0}, "bar_size": {"width": 1920, "height": 30}},
        ]});
        let found = places(&state);
        assert_eq!(found.len(), 3);
        assert_eq!(
            (key_display(&found[0].0), key_panel(&found[0].0), found[0].1),
            ("Monitor A", "main", (0, 0, 1920, 30))
        );
        assert_eq!(
            (key_display(&found[1].0), key_panel(&found[1].0), found[1].1),
            ("Monitor A", "rail", (0, 30, 40, 1050))
        );
        assert_eq!(found[2].0, "Monitor B");
        assert_eq!(
            (key_display(&found[2].0), key_panel(&found[2].0)),
            ("Monitor B", "")
        );
        assert_ne!(found[0].0, found[1].0);
        // A surface in a corner keeps clear of each: under the one at
        // the top, right of the one down the left side.
        let room = &rooms(&state)[0];
        assert_eq!(room.bars.len(), 2);
        use gevrek_model::settings::NotificationCorner as Corner;
        let (x, y, _, _) = corner_at(room, (300, 100), 10, Corner::TopLeft);
        assert_eq!((x, y), (50, 40));
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
            serde_json::from_str(&show_workspace_at("Monitor X", false, 2, " "))
                .expect("a command");
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
                        extras: &Extras::default(),
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

    #[test]
    fn a_name_is_made_short_for_a_narrow_place() {
        for (name, short) in [
            ("1", "1"),
            ("12", "12"),
            ("1234", "123"),
            ("Web", "We"),
            ("mail", "Ma"),
            ("saturn w", "Sa"),
            ("x", "X"),
            (" 3 ", "3"),
            ("çay", "Ça"),
            ("", ""),
        ] {
            assert_eq!(short_name(name), short, "{name}");
        }
    }

    /// Down a side, what stands at the panel's start and what stands at
    /// its end do not lie on each other, however many windows the
    /// taskbar has (they did: the taskbar went on under the clock); and
    /// what is at the end is in its order from the top.
    #[test]
    fn a_panel_goes_out_of_sight_a_while_after_nothing_holds_it() {
        use gevrek_model::settings::PanelHide;
        let start = std::time::Instant::now();
        let after = |millis: u64| start + std::time::Duration::from_millis(millis);
        // Never: always in sight, whatever lies on it.
        let mut never = Hiding::default();
        assert_eq!(
            never.step(PanelHide::Never, true, false, start),
            (true, None)
        );
        // Always: in sight for a while, then gone, and back while the
        // pointer is on it or a menu of its is open.
        let mut hiding = Hiding::default();
        let (shown, again) = hiding.step(PanelHide::Always, false, false, start);
        assert!(shown);
        assert_eq!(again, Some(HIDE_AFTER));
        let (shown, again) = hiding.step(PanelHide::Always, false, false, after(300));
        assert!(shown && again == Some(HIDE_AFTER - std::time::Duration::from_millis(300)));
        assert_eq!(
            hiding.step(PanelHide::Always, false, false, after(700)),
            (false, None)
        );
        assert!(hiding.hidden);
        assert_eq!(
            hiding.step(PanelHide::Always, false, true, after(900)),
            (true, None)
        );
        assert!(!hiding.hidden);
        // (The while begins anew when it is let go.)
        let (shown, _) = hiding.step(PanelHide::Always, false, false, after(1000));
        assert!(shown);
        let (shown, _) = hiding.step(PanelHide::Always, false, false, after(1600));
        assert!(shown);
        assert_eq!(
            hiding.step(PanelHide::Always, false, false, after(1700)),
            (false, None)
        );
        // While covered only: in sight when nothing lies on it.
        let mut covered_only = Hiding::default();
        assert_eq!(
            covered_only.step(PanelHide::Covered, false, false, start),
            (true, None)
        );
        let (shown, again) = covered_only.step(PanelHide::Covered, true, false, after(10));
        assert!(shown && again.is_some());
        assert_eq!(
            covered_only.step(PanelHide::Covered, true, false, after(800)),
            (false, None)
        );
        assert_eq!(
            covered_only.step(PanelHide::Covered, false, false, after(900)),
            (true, None)
        );
    }

    #[test]
    fn what_is_left_of_a_hidden_panel_is_a_strip_at_its_displays_edge() {
        let whole = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 36.0));
        assert_eq!(
            hidden_reach(whole, PanelEdge::Top),
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 3.0))
        );
        assert_eq!(hidden_reach(whole, PanelEdge::Bottom).top(), 33.0);
        let side = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(40.0, 900.0));
        assert_eq!(hidden_reach(side, PanelEdge::Left).width(), 3.0);
        assert_eq!(hidden_reach(side, PanelEdge::Right).left(), 37.0);
        // A window's place, in the pixels of the screen.
        assert_eq!(
            hidden_place((100, 0, 800, 36), PanelEdge::Top, 3),
            (100, 0, 800, 3)
        );
        assert_eq!(
            hidden_place((100, 1044, 800, 36), PanelEdge::Bottom, 3),
            (100, 1077, 800, 3)
        );
        assert_eq!(
            hidden_place((0, 0, 40, 900), PanelEdge::Left, 3),
            (0, 0, 3, 900)
        );
        assert_eq!(
            hidden_place((1880, 0, 40, 900), PanelEdge::Right, 3),
            (1917, 0, 3, 900)
        );
        // A panel that hides stands at the display's edge and keeps no room.
        let mut bar = PanelProfile::default();
        assert_eq!(geometry(&bar), (PanelEdge::Top, 36, 6, false));
        bar.hide = gevrek_model::settings::PanelHide::Always;
        assert_eq!(geometry(&bar), (PanelEdge::Top, 36, 0, true));
    }

    #[test]
    fn a_panel_is_covered_by_a_window_that_lies_where_it_stands() {
        let window = |x: f64, y: f64, minimized: bool, border: bool| {
            json!({
                "window_id": 1, "position": {"x": x, "y": y},
                "size": {"width": 600, "height": 400},
                "is_minimized": minimized, "is_border_window": border
            })
        };
        let state = |windows: Vec<Value>| json!({"monitors": [{"name": "Monitor A", "workspace": {"detailed_windows": windows}}]});
        let bar = (0, 0, 1920, 36);
        assert!(!covered(&state(vec![]), "Monitor A", bar));
        assert!(!covered(
            &state(vec![window(100.0, 40.0, false, false)]),
            "Monitor A",
            bar
        ));
        assert!(covered(
            &state(vec![window(100.0, 20.0, false, false)]),
            "Monitor A",
            bar
        ));
        // Not by one that is minimized, nor by a border, nor on another display.
        assert!(!covered(
            &state(vec![window(100.0, 20.0, true, false)]),
            "Monitor A",
            bar
        ));
        assert!(!covered(
            &state(vec![window(100.0, 20.0, false, true)]),
            "Monitor A",
            bar
        ));
        assert!(!covered(
            &state(vec![window(100.0, 20.0, false, false)]),
            "Monitor B",
            bar
        ));
    }

    #[test]
    fn the_command_that_closes_a_window_is_one_the_model_reads() {
        use gevrek_model::command::{DispatchCommand, WindowSelector};
        let command: DispatchCommand = serde_json::from_str(&close_window(42)).expect("a command");
        assert_eq!(
            command,
            DispatchCommand::CloseWindow {
                selector: WindowSelector::Handle(42)
            }
        );
    }

    #[test]
    fn the_later_modules_show_what_is_known_and_nothing_where_nothing_is() {
        use gevrek_model::settings::{CustomModule, PanelStyle};
        let view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        let system = SystemView::default();
        // (Each module on a piece of its own: which of them are there.)
        let drawn = |bar: &PanelProfile, extras: &Extras| -> (usize, Asked) {
            let context = egui::Context::default();
            let mut asked = Asked::default();
            for _ in 0..4 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 36.0),
                    )),
                    ..Default::default()
                };
                let mut output = context.run_ui(input, |ui| {
                    asked = draw(
                        ui,
                        &Shown {
                            bar,
                            view: &view,
                            accent: egui::Color32::from_rgb(64, 224, 208),
                            quiet: false,
                            notices: 0,
                            mode: "",
                            recording: false,
                            sharing: false,
                            output: "Monitor ACM2702 (1)",
                            tray: &[],
                            system: &system,
                            keyboard: None,
                            extras,
                        },
                        None,
                    );
                });
                output.textures_delta.clear();
            }
            (asked.reach.len(), asked)
        };
        let mut bar = PanelProfile {
            style: PanelStyle::Islands,
            left: vec![PanelModule::Load],
            centre: vec![PanelModule::Media],
            right: vec![PanelModule::Custom],
            ..Default::default()
        };
        // Nothing known, and no words of its own: nothing stands there.
        assert_eq!(drawn(&bar, &Extras::default()).0, 0);
        // Its own words stand without anything being known.
        bar.custom = CustomModule {
            text: "hello".to_string(),
            ..CustomModule::default()
        };
        assert_eq!(drawn(&bar, &Extras::default()).0, 1);
        // All three, and what a program wrote in place of the words.
        let extras = Extras {
            load: Some(gevrek_system::status::Load {
                processor: 12,
                memory: 48,
            }),
            playing: Some(("A piece — by someone".to_string(), true)),
            custom: Some("21 °C".to_string()),
        };
        let (pieces, asked) = drawn(&bar, &extras);
        assert_eq!(pieces, 3);
        assert!(asked.media.is_none() && !asked.custom_pressed);
    }

    #[test]
    fn a_panels_own_menu_changes_the_configuration_as_it_is_asked() {
        use gevrek_model::settings::{PanelHide, PanelModel};
        let mut config = gevrek_model::config::Config::default();
        // Its settings change nothing: they are opened.
        assert!(!panel_asked(&mut config, "main", PanelAsk::Settings));
        // Another edge; the same edge is no change.
        assert!(panel_asked(
            &mut config,
            "main",
            PanelAsk::Move(PanelEdge::Left)
        ));
        assert!(!panel_asked(
            &mut config,
            "main",
            PanelAsk::Move(PanelEdge::Left)
        ));
        assert_eq!(config.desktop.shell.panels[0].edge, PanelEdge::Left);
        // Out of sight, and back.
        assert!(panel_asked(&mut config, "MAIN", PanelAsk::Hide(true)));
        assert_eq!(config.desktop.shell.panels[0].hide, PanelHide::Always);
        assert_eq!(config.desktop.shell.panels[0].reserved(), 0);
        assert!(!panel_asked(&mut config, "main", PanelAsk::Hide(true)));
        assert!(panel_asked(&mut config, "main", PanelAsk::Hide(false)));
        // A panel more, with an id of its own; and taken away again.
        assert!(panel_asked(
            &mut config,
            "main",
            PanelAsk::Add(PanelModel::Dock)
        ));
        assert!(panel_asked(
            &mut config,
            "main",
            PanelAsk::Add(PanelModel::Dock)
        ));
        let ids: Vec<&str> = config
            .desktop
            .shell
            .panels
            .iter()
            .map(|panel| panel.id.as_str())
            .collect();
        assert_eq!(ids, ["main", "dock", "dock-2"]);
        assert!(config.validate().is_empty(), "{:?}", config.validate());
        assert!(panel_asked(&mut config, "dock", PanelAsk::Remove));
        assert!(!panel_asked(&mut config, "dock", PanelAsk::Remove));
        assert!(!panel_asked(
            &mut config,
            "no such",
            PanelAsk::Move(PanelEdge::Top)
        ));
        assert_eq!(config.desktop.shell.panels.len(), 2);
        // The menu has a line for each, and the edge it is at is marked.
        let bar = &config.desktop.shell.panels[0];
        let lines = panel_lines(bar, 2);
        assert_eq!(lines.len(), 10);
        let marked: Vec<PanelAsk> = lines
            .iter()
            .filter(|line| line.3)
            .map(|line| line.0)
            .collect();
        assert_eq!(marked, [PanelAsk::Move(PanelEdge::Left)]);
        assert!(panel_menu_size(bar, 2).y > 200.0);
    }

    #[test]
    fn the_other_button_on_the_panel_itself_asks_for_its_menu() {
        let bar = PanelProfile {
            left: vec![PanelModule::Launcher],
            centre: Vec::new(),
            right: vec![PanelModule::Power],
            ..Default::default()
        };
        let view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        let context = egui::Context::default();
        let system = SystemView::default();
        let size = egui::vec2(1200.0, 36.0);
        // The other button, pressed and let go at `at`.
        let pressed_at = |at: egui::Pos2| -> Asked {
            let mut asked = Asked::default();
            for step in 0..4 {
                let events = match step {
                    0 => vec![egui::Event::PointerMoved(at)],
                    1 | 2 => vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Secondary,
                        pressed: step == 1,
                        modifiers: Default::default(),
                    }],
                    _ => Vec::new(),
                };
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                };
                let mut found = Asked::default();
                let mut output = context.run_ui(input, |ui| {
                    found = draw(
                        ui,
                        &Shown {
                            bar: &bar,
                            view: &view,
                            accent: egui::Color32::from_rgb(64, 224, 208),
                            quiet: false,
                            notices: 0,
                            mode: "",
                            recording: false,
                            sharing: false,
                            output: "Monitor ACM2702 (1)",
                            tray: &[],
                            system: &system,
                            keyboard: None,
                            extras: &Extras::default(),
                        },
                        None,
                    );
                });
                output.textures_delta.clear();
                if found.panel_menu.is_some() || found.pressed.is_some() {
                    asked = found;
                }
            }
            asked
        };
        // In the middle, where nothing stands: the panel's own menu.
        let middle = pressed_at(egui::pos2(600.0, 18.0));
        assert!(middle.panel_menu.is_some());
        // On the launcher's button: not the panel's menu.
        let on_button = pressed_at(egui::pos2(24.0, 18.0));
        assert!(on_button.panel_menu.is_none());
    }

    #[test]
    fn down_a_side_words_are_turned_kept_short_or_left_out() {
        use gevrek_model::settings::{PanelEdge, PanelStyle, SideText};
        let mut view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        view.title = "a window with a title of some length".to_string();
        view.tabs = ["Web", "Mail", "Terminal"]
            .iter()
            .enumerate()
            .map(|(index, name)| Tab {
                name: name.to_string(),
                label: name.to_string(),
                configured: true,
                shown: index == 0,
                windows: 1,
            })
            .collect();
        // (Each group on a piece of its own: how long each came to be.)
        let drawn = |side_text: SideText| -> Vec<f32> {
            let bar = PanelProfile {
                edge: PanelEdge::Left,
                style: PanelStyle::Islands,
                height: 40,
                padding: 0,
                side_text,
                left: vec![PanelModule::Workspaces],
                centre: vec![PanelModule::Title],
                right: vec![PanelModule::Sound],
                ..Default::default()
            };
            panel_drawn(&bar, &view, egui::vec2(40.0, 1000.0), 6, None)
                .reach
                .iter()
                .map(|piece| piece.height())
                .collect()
        };
        let (turned, stacked, signs) = (
            drawn(SideText::Rotated),
            drawn(SideText::Stacked),
            drawn(SideText::Signs),
        );
        // Turned: the workspaces by their whole names, which makes them
        // longer than their first letters; the title in the middle, as
        // long as it is told to be; the sound with its number beside its
        // sign's place.
        assert_eq!(turned.len(), 3, "{turned:?}");
        assert!(turned[0] > stacked[0] + 20.0, "{turned:?} {stacked:?}");
        assert!((turned[1] - 260.0).abs() < 1.0, "{turned:?}");
        // Upright and short: no title (it is as long as it is), and the
        // sound's number under its sign.
        assert_eq!(stacked.len(), 2, "{stacked:?}");
        assert!(turned[2] > stacked[1], "{turned:?} {stacked:?}");
        // The signs alone: the sound without its number; the workspaces
        // still by their letters, which is all they have.
        assert_eq!(signs.len(), 2, "{signs:?}");
        assert!(signs[1] < stacked[1], "{signs:?} {stacked:?}");
        assert_eq!(signs[0], stacked[0]);
        // Along the top nothing of this counts.
        let lying = |side_text: SideText| {
            let bar = PanelProfile {
                side_text,
                style: PanelStyle::Islands,
                left: vec![PanelModule::Workspaces],
                centre: vec![PanelModule::Title],
                right: vec![PanelModule::Sound],
                ..Default::default()
            };
            panel_drawn(&bar, &view, egui::vec2(1600.0, 36.0), 6, None).reach
        };
        assert_eq!(lying(SideText::Rotated), lying(SideText::Signs));
    }

    #[test]
    fn down_a_side_the_ends_of_the_panel_keep_apart() {
        use gevrek_model::settings::{PanelEdge, PanelStyle};
        let bar = PanelProfile {
            edge: PanelEdge::Left,
            style: PanelStyle::Islands,
            height: 40,
            left: vec![
                PanelModule::Launcher,
                PanelModule::Workspaces,
                PanelModule::Layout,
                PanelModule::Taskbar,
            ],
            centre: Vec::new(),
            right: vec![
                PanelModule::Sound,
                PanelModule::Battery,
                PanelModule::Clock,
                PanelModule::Power,
            ],
            ..Default::default()
        };
        let mut view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        view.tasks = (0..40)
            .map(|id| Task {
                id,
                title: format!("window {id}"),
                class: String::new(),
                icon: String::new(),
                focused: id == 3,
                shown: true,
                minimized: false,
                floating: false,
                on_top: false,
            })
            .collect();
        let system = SystemView {
            volume: Some((40, false)),
            battery: Some(gevrek_system::status::Battery {
                percent: 60,
                charging: false,
            }),
            network: None,
            brightness: None,
        };
        let context = egui::Context::default();
        let mut reach = Vec::new();
        // (Thrice: what is at the end is placed by how long it was.)
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(40.0, 700.0),
                )),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                reach = draw(
                    ui,
                    &Shown {
                        bar: &bar,
                        view: &view,
                        accent: egui::Color32::from_rgb(64, 224, 208),
                        quiet: false,
                        notices: 0,
                        mode: "",
                        recording: false,
                        sharing: false,
                        output: "Monitor ACM2702 (1)",
                        tray: &[],
                        system: &system,
                        keyboard: None,
                        extras: &Extras::default(),
                    },
                    None,
                )
                .reach;
            });
            output.textures_delta.clear();
        }
        assert_eq!(reach.len(), 2, "{reach:?}");
        let (start, end) = (reach[0], reach[1]);
        assert!(start.top() < end.top());
        // (Each piece reaches as far beyond what stands on it as the
        // panel's own edge is from its first module.)
        let beyond = bar.padding as f32;
        assert!(
            start.bottom() - beyond <= end.top() + beyond + 0.5,
            "{start:?} and {end:?}"
        );
        assert!(end.bottom() <= 700.5, "{end:?}");
        assert!(start.height() > 200.0 && end.height() > 80.0);
    }

    /// All the buttons stand while each has the least a button is long;
    /// then as many of the shortest as fit beside the one for the rest.
    #[test]
    fn the_buttons_of_the_taskbar_share_the_room_they_have() {
        // Room for all at the width that is wanted.
        assert_eq!(task_fit(1000.0, 8.0, 3, 150.0), (150.0, 3));
        // Narrower, each its share.
        let (each, fits) = task_fit(400.0, 8.0, 5, 150.0);
        assert_eq!(fits, 5);
        assert!((each - (400.0 - 32.0) / 5.0).abs() < 0.01, "{each}");
        // Down to the least, and not below it.
        assert_eq!(
            task_fit(5.0 * 28.0 + 4.0 * 8.0, 8.0, 5, 150.0),
            (TASK_LEAST, 5)
        );
        let (each, fits) = task_fit(5.0 * 28.0 + 4.0 * 8.0 - 1.0, 8.0, 5, 150.0);
        assert_eq!(each, TASK_LEAST);
        assert!(fits < 5);
        // What stands is never longer than the room.
        for room in [0.0_f32, 10.0, 29.0, 31.0, 66.0, 100.0, 333.0, 2000.0] {
            for count in [0_usize, 1, 2, 7, 40, 300] {
                for wanted in [0.0, 28.0, 150.0, f32::INFINITY] {
                    let (each, fits) = task_fit(room, 8.0, count, wanted);
                    assert!(each >= TASK_LEAST && fits <= count);
                    let more = if fits < count { TASK_MORE } else { 0.0 };
                    let pieces = fits + usize::from(fits < count);
                    let long = each * fits as f32 + more + 8.0 * pieces.saturating_sub(1) as f32;
                    // (But the button for the rest alone, which is there
                    // whatever the room.)
                    assert!(
                        long <= room.max(TASK_MORE) + 0.01,
                        "{long} in {room}: {count} at {wanted}"
                    );
                }
            }
        }
    }

    /// The windows of one program under one button: always, never, or
    /// when there is no room for a button each.
    #[test]
    fn the_windows_of_a_program_stand_under_one_button() {
        let task = |id: u64, icon: &str, class: &str| Task {
            id,
            title: format!("window {id}"),
            class: class.to_string(),
            icon: icon.to_string(),
            focused: false,
            shown: true,
            minimized: false,
            floating: false,
            on_top: false,
        };
        let tasks = [
            task(1, "firefox", "Navigator"),
            task(2, "", "term"),
            task(3, "firefox", "Navigator"),
            task(4, "", ""),
            task(5, "", "term"),
            task(6, "", ""),
        ];
        let ids = |buttons: Vec<Vec<&Task>>| -> Vec<Vec<u64>> {
            buttons
                .iter()
                .map(|button| button.iter().map(|task| task.id).collect())
                .collect()
        };
        let apart: Vec<Vec<u64>> = (1..=6).map(|id| vec![id]).collect();
        // (A window that says nothing of its program stands alone.)
        let together = vec![vec![1, 3], vec![2, 5], vec![4], vec![6]];
        use TaskGrouping::{Always, Never, WhenFull};
        assert_eq!(ids(task_buttons(&tasks, Never, false)), apart);
        assert_eq!(ids(task_buttons(&tasks, WhenFull, true)), apart);
        assert_eq!(ids(task_buttons(&tasks, WhenFull, false)), together);
        assert_eq!(ids(task_buttons(&tasks, Always, true)), together);
    }

    /// Where the groups of a panel came to stand (islands: each piece
    /// reaches as far beyond its group as the panel's padding), and what
    /// the last frame asked, after `frames` frames; a press at `press`
    /// in the last one.
    fn panel_drawn(
        bar: &PanelProfile,
        view: &View,
        size: egui::Vec2,
        frames: usize,
        press: Option<egui::Pos2>,
    ) -> Asked {
        let system = SystemView {
            volume: Some((40, false)),
            battery: None,
            network: None,
            brightness: None,
        };
        let context = egui::Context::default();
        let mut asked = Asked::default();
        for frame in 0..frames {
            let mut events = Vec::new();
            if let Some(at) = press {
                // (Over it, down on it, and up again: three frames.)
                let step = frame + 3;
                if step == frames {
                    events.push(egui::Event::PointerMoved(at));
                } else if step == frames + 1 {
                    events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                } else if step == frames + 2 {
                    events.push(egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                }
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                asked = draw(
                    ui,
                    &Shown {
                        bar,
                        view,
                        accent: egui::Color32::from_rgb(64, 224, 208),
                        quiet: false,
                        notices: 0,
                        mode: "",
                        recording: false,
                        sharing: false,
                        output: "Monitor ACM2702 (1)",
                        tray: &[],
                        system: &system,
                        keyboard: None,
                        extras: &Extras::default(),
                    },
                    None,
                );
            });
            output.textures_delta.clear();
        }
        asked
    }

    fn many_tasks(count: u64) -> Vec<Task> {
        (0..count)
            .map(|id| Task {
                id: id + 1,
                title: format!("window {id} of a program with a long title"),
                class: format!("program-{}", id % 7),
                icon: String::new(),
                focused: id == 3,
                shown: true,
                minimized: false,
                floating: false,
                on_top: false,
            })
            .collect()
    }

    /// What was reported: with many windows the taskbar's buttons went on
    /// over what stands in the middle of the panel and at its end. They
    /// do not, however many windows there are, wherever on the panel the
    /// taskbar stands, and whatever is set for it.
    #[test]
    fn the_taskbar_keeps_to_its_room_however_many_windows_there_are() {
        use gevrek_model::settings::{PanelEdge, PanelFit, PanelStyle};
        let size = egui::vec2(1100.0, 36.0);
        for count in [0, 1, 4, 13, 40, 300] {
            for grouping in [
                TaskGrouping::Never,
                TaskGrouping::WhenFull,
                TaskGrouping::Always,
            ] {
                for task_width in [0, 150] {
                    for place in 0..3 {
                        for fit in [PanelFit::Full, PanelFit::Content] {
                            let mut bar = PanelProfile {
                                edge: PanelEdge::Top,
                                style: PanelStyle::Islands,
                                fit,
                                task_width,
                                task_grouping: grouping,
                                left: vec![PanelModule::Launcher, PanelModule::Workspaces],
                                centre: vec![PanelModule::Clock],
                                right: vec![
                                    PanelModule::Sound,
                                    PanelModule::Clock,
                                    PanelModule::Power,
                                ],
                                ..Default::default()
                            };
                            match place {
                                0 => bar.left.push(PanelModule::Taskbar),
                                1 => bar.centre.insert(0, PanelModule::Taskbar),
                                _ => bar.right.insert(0, PanelModule::Taskbar),
                            }
                            let mut view =
                                view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
                            view.tasks = many_tasks(count);
                            let what = format!(
                                "{count} windows, {grouping:?}, {task_width} wide, in group {place}, {fit:?}"
                            );
                            let reach = panel_drawn(&bar, &view, size, 8, None).reach;
                            // (It has come to rest: a frame more is the same.)
                            let again = panel_drawn(&bar, &view, size, 9, None).reach;
                            assert_eq!(reach, again, "{what}");
                            assert_eq!(reach.len(), 3, "{what}: {reach:?}");
                            // What stands in a piece, without the room
                            // around it.
                            let beyond = bar.padding as f32;
                            let mut pieces: Vec<(f32, f32)> = reach
                                .iter()
                                .map(|piece| (piece.left() + beyond, piece.right() - beyond))
                                .collect();
                            pieces.sort_by(|one, other| one.0.total_cmp(&other.0));
                            assert!(pieces[0].0 >= -0.5, "{what}: {pieces:?}");
                            assert!(pieces[2].1 <= size.x + 0.5, "{what}: {pieces:?}");
                            for pair in pieces.windows(2) {
                                assert!(pair[0].1 <= pair[1].0 + 0.5, "{what}: {pieces:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Windows there is no room for are behind a button of their own: a
    /// press on it asks for the list of them, and of no others.
    #[test]
    fn the_windows_without_room_are_listed_by_a_button_of_their_own() {
        use gevrek_model::settings::PanelEdge;
        let size = egui::vec2(700.0, 36.0);
        let bar = PanelProfile {
            edge: PanelEdge::Top,
            task_grouping: TaskGrouping::Never,
            left: vec![PanelModule::Taskbar],
            centre: Vec::new(),
            right: vec![PanelModule::Power],
            ..Default::default()
        };
        let mut view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        view.tasks = many_tasks(60);
        // Its button is the last of the taskbar: found from the panel's
        // end, where the power's button stands before it.
        let mut listed = None;
        for from_end in (30..160).step_by(6) {
            let at = egui::pos2(size.x - from_end as f32, size.y / 2.0);
            let asked = panel_drawn(&bar, &view, size, 8, Some(at));
            if let Some((_, windows)) = asked.task_menu {
                listed = Some(windows);
                break;
            }
        }
        let listed = listed.expect("a button for the windows without room");
        assert!(!listed.is_empty() && listed.len() < 60, "{listed:?}");
        // (The last ones, in their order.)
        let last: Vec<u64> = (61 - listed.len() as u64..=60).collect();
        assert_eq!(listed, last);
        assert_eq!(tasks_of_ids(&view, &listed).len(), listed.len());
        // With the windows of a program together there is room for all:
        // a press on a program's button lists its windows.
        let bar = PanelProfile {
            task_grouping: TaskGrouping::WhenFull,
            ..bar
        };
        let mut listed = None;
        for from_start in (50..200).step_by(6) {
            let at = egui::pos2(from_start as f32, size.y / 2.0);
            let asked = panel_drawn(&bar, &view, size, 8, Some(at));
            if let Some((_, windows)) = asked.task_menu {
                listed = Some(windows);
                break;
            }
        }
        let listed = listed.expect("a program's button");
        assert!(listed.len() >= 8, "{listed:?}");
        let program = |id: &u64| (id - 1) % 7;
        assert!(
            listed.iter().all(|id| program(id) == program(&listed[0])),
            "{listed:?}"
        );
    }

    /// A window the person minimized is read from the state as such, in
    /// its place among the others, on the shown workspace and in the list
    /// of every workspace's.
    #[test]
    fn a_minimized_window_is_a_task_that_says_so() {
        let monitor = serde_json::json!({
            "workspace": {"id": 2},
            "workspaces": [
                {"id": 1, "name": "1", "detailed_windows": [
                    {"window_id": 7, "title": "away", "is_minimized": true}]},
                {"id": 2, "name": "2", "detailed_windows": [
                    {"window_id": 1, "title": "editor", "is_focused": true},
                    {"window_id": 2, "title": "notes", "is_minimized": true},
                    {"window_id": 3, "title": "mail"}]},
            ]
        });
        let view = view_of(&monitor);
        let flags: Vec<(u64, bool)> = view
            .tasks
            .iter()
            .map(|task| (task.id, task.minimized))
            .collect();
        assert_eq!(flags, [(1, false), (2, true), (3, false)]);
        let away = view
            .all_tasks
            .iter()
            .find(|task| task.id == 7)
            .expect("listed");
        assert!(away.minimized && !away.shown);
        // (The list under a button has it as the view does.)
        assert!(tasks_of_ids(&view, &[2])[0].minimized);
    }

    /// A minimized window's button is hollow: a line around it in place
    /// of the fill, its picture faded, never the accent's line; the
    /// pointer over it fills it a little. The others are as they were.
    #[test]
    fn a_minimized_windows_button_is_hollow() {
        let (text, accent) = (egui::Color32::WHITE, egui::Color32::from_rgb(64, 224, 208));
        let resting = task_look(text, accent, false, false, true);
        assert_eq!(resting.fill, egui::Color32::TRANSPARENT);
        assert!(resting.outline.is_some() && !resting.keys_line);
        assert!(resting.picture.a() < 255, "its picture is faded");
        let hovered = task_look(text, accent, true, false, true);
        assert_ne!(hovered.fill, egui::Color32::TRANSPARENT);
        // (Even were it told to have the keyboard, nothing of it is seen.)
        assert!(!task_look(text, accent, false, true, true).keys_line);
        for (hovered, has_keys) in [(false, false), (true, false), (false, true), (true, true)] {
            let look = task_look(text, accent, hovered, has_keys, false);
            assert!(look.outline.is_none() && look.fill != egui::Color32::TRANSPARENT);
            assert_eq!(look.keys_line, has_keys);
            assert_eq!(look.picture, egui::Color32::WHITE);
        }
        // Its words and its line stand out seven to one in the
        // high-contrast palettes: white on black, and black on white.
        for (ink, ground) in [
            (egui::Color32::WHITE, egui::Color32::BLACK),
            (egui::Color32::BLACK, egui::Color32::WHITE),
        ] {
            let faded = ink.gamma_multiply(MINIMIZED_INK);
            let seen = blend(faded, ground);
            assert!(
                theme::contrast(seen, ground) >= 7.0,
                "{ink:?} on {ground:?}"
            );
        }
    }

    /// A colour with some of it see-through, as it is seen over `ground`.
    fn blend(colour: egui::Color32, ground: egui::Color32) -> egui::Color32 {
        let over = colour.a() as f32 / 255.0;
        let mix = |front: u8, back: u8| {
            // (Premultiplied: what is in front is already as strong as
            // it is see-through.)
            (front as f32 + back as f32 * (1.0 - over))
                .round()
                .min(255.0) as u8
        };
        egui::Color32::from_rgb(
            mix(colour.r(), ground.r()),
            mix(colour.g(), ground.g()),
            mix(colour.b(), ground.b()),
        )
    }

    /// A press on a minimized window's button asks for it to be given the
    /// keyboard: the window manager brings it back for that.
    #[test]
    fn a_press_on_a_minimized_windows_button_brings_it_back() {
        use gevrek_model::settings::PanelEdge;
        let size = egui::vec2(700.0, 36.0);
        let bar = PanelProfile {
            edge: PanelEdge::Top,
            task_grouping: TaskGrouping::Never,
            left: vec![PanelModule::Taskbar],
            centre: Vec::new(),
            right: vec![PanelModule::Power],
            ..Default::default()
        };
        let mut view = view(&windows_state(), "Monitor ACM2702 (1)").expect("the display");
        view.tasks = vec![Task {
            minimized: true,
            floating: false,
            on_top: false,
            ..many_tasks(1).remove(0)
        }];
        let mut went = None;
        for from_start in (30..200).step_by(6) {
            let at = egui::pos2(from_start as f32, size.y / 2.0);
            let asked = panel_drawn(&bar, &view, size, 8, Some(at));
            if asked.go.is_some() {
                went = asked.go;
                break;
            }
        }
        assert_eq!(went, Some(1), "a press on its button");
        let command: serde_json::Value = serde_json::from_str(&go_to_window(1)).expect("a command");
        assert_eq!(command["command"], "focus_window");
        assert_eq!(command["selector"]["value"], 1);
        let parsed: gevrek_model::command::DispatchCommand =
            serde_json::from_value(command).expect("the window manager reads it");
        assert!(matches!(
            parsed,
            gevrek_model::command::DispatchCommand::FocusWindow { .. }
        ));
    }

    /// The windows of a program stand under one button with a minimized
    /// one among them, in its place; a press on it lists them all.
    #[test]
    fn a_minimized_window_is_counted_with_its_programs_others() {
        let mut tasks = many_tasks(14);
        tasks[7].minimized = true;
        let buttons = task_buttons(&tasks, TaskGrouping::Always, false);
        let group = buttons
            .iter()
            .find(|button| button.iter().any(|task| task.minimized))
            .expect("its program's button");
        let ids: Vec<u64> = group.iter().map(|task| task.id).collect();
        assert_eq!(
            ids,
            [1, 8],
            "with the other window of its program, in its place"
        );
        assert!(
            !group.iter().all(|task| task.minimized),
            "the button itself is not hollow"
        );
    }

    /// A panel drawn into a picture, to be looked at while it is worked
    /// on:
    ///
    ///     GEVREK_LOOK_OUT=panel.png cargo test -p gevrek-shell     ///         the_panel_is_drawn_to_be_looked_at -- --ignored
    ///
    /// `GEVREK_LOOK=left` (or `right`, `bottom`) for another edge,
    /// `GEVREK_LOOK_STYLE=islands` (or `bare`), `GEVREK_LOOK_HEIGHT=44`
    /// for how thick it is, `GEVREK_LOOK_SIZE=1600x900` for the display,
    /// `GEVREK_LOOK_WINDOWS=40` for so many windows on the taskbar.
    #[test]
    #[ignore = "draws a picture to look at: GEVREK_LOOK_OUT=<file>"]
    fn the_panel_is_drawn_to_be_looked_at() {
        use gevrek_model::settings::{PanelEdge, PanelStyle};
        let said = |name: &str| std::env::var(name).ok().filter(|text| !text.is_empty());
        let out =
            std::path::PathBuf::from(said("GEVREK_LOOK_OUT").expect("GEVREK_LOOK_OUT: a file"));
        let edge = match said("GEVREK_LOOK").as_deref() {
            Some("left") => PanelEdge::Left,
            Some("right") => PanelEdge::Right,
            Some("bottom") => PanelEdge::Bottom,
            _ => PanelEdge::Top,
        };
        let style = match said("GEVREK_LOOK_STYLE").as_deref() {
            Some("islands") => PanelStyle::Islands,
            Some("bare") => PanelStyle::Bare,
            _ => PanelStyle::Bar,
        };
        let display = said("GEVREK_LOOK_SIZE")
            .and_then(|text| {
                let (wide, high) = text.split_once('x')?;
                Some((
                    wide.trim().parse::<u32>().ok()?,
                    high.trim().parse::<u32>().ok()?,
                ))
            })
            .unwrap_or((1600, 900));
        let mut bar = PanelProfile {
            edge,
            style,
            left: vec![
                PanelModule::Launcher,
                PanelModule::Workspaces,
                PanelModule::Layout,
                PanelModule::Taskbar,
            ],
            centre: vec![PanelModule::Title],
            right: vec![
                PanelModule::Stats,
                PanelModule::Keyboard,
                PanelModule::Network,
                PanelModule::Sound,
                PanelModule::Brightness,
                PanelModule::Battery,
                PanelModule::Notifications,
                PanelModule::ControlCentre,
                PanelModule::Clock,
                PanelModule::Power,
            ],
            ..Default::default()
        };
        if let Some(height) = said("GEVREK_LOOK_HEIGHT").and_then(|text| text.parse().ok()) {
            bar.height = height;
        }
        // (`GEVREK_LOOK_EXTRAS=1`: the later modules too, with something
        // to show.)
        let extras = Extras {
            load: Some(gevrek_system::status::Load {
                processor: 12,
                memory: 48,
            }),
            playing: Some(("Gymnopédie No. 1 — Erik Satie".to_string(), true)),
            custom: Some("21 °C".to_string()),
        };
        if said("GEVREK_LOOK_EXTRAS").is_some() {
            bar.left = vec![
                PanelModule::Launcher,
                PanelModule::Workspaces,
                PanelModule::Separator,
                PanelModule::Media,
            ];
            bar.right = vec![
                PanelModule::Custom,
                PanelModule::Load,
                PanelModule::Separator,
                PanelModule::Sound,
                PanelModule::Clock,
            ];
        }
        // (`GEVREK_LOOK_TEXT=stacked` or `signs`: how words are written
        // down a side.)
        bar.side_text = match said("GEVREK_LOOK_TEXT").as_deref() {
            Some("stacked") => gevrek_model::settings::SideText::Stacked,
            Some("signs") => gevrek_model::settings::SideText::Signs,
            _ => gevrek_model::settings::SideText::Rotated,
        };
        let thick = bar.height.clamp(16, 200);
        let size = if edge.is_upright() {
            (thick, display.1)
        } else {
            (display.0, thick)
        };
        let tab = |name: &str, shown: bool, windows: usize| Tab {
            name: name.to_string(),
            label: name.to_string(),
            configured: true,
            shown,
            windows,
        };
        let task = |id: u64, title: &str, focused: bool| Task {
            id,
            title: title.to_string(),
            class: String::new(),
            icon: String::new(),
            focused,
            shown: true,
            minimized: false,
            floating: false,
            on_top: false,
        };
        let view = View {
            tabs: vec![
                tab("1", false, 2),
                tab("Web", true, 3),
                tab("3", false, 0),
                tab("Mail", false, 1),
            ],
            layout: "dwindle".to_string(),
            floating: false,
            title: "panel.rs - gevrek - a title that goes on for a while".to_string(),
            focused: true,
            place: gevrek_model::settings::DisplayRef::named("DP-1"),
            tasks: match said("GEVREK_LOOK_WINDOWS").and_then(|text| text.parse::<u64>().ok()) {
                Some(count) => many_tasks(count),
                None => vec![
                    task(1, "Terminal", false),
                    task(2, "Firefox", true),
                    task(3, "Files", false),
                    Task {
                        minimized: true,
                        floating: false,
                        on_top: false,
                        ..task(4, "Notes (minimized)", false)
                    },
                ],
            },
            all_tasks: Vec::new(),
        };
        let system = SystemView {
            volume: Some((62, false)),
            battery: Some(gevrek_system::status::Battery {
                percent: 78,
                charging: false,
            }),
            network: Some(gevrek_system::status::Network {
                connected: true,
                wireless: true,
                name: "Home".to_string(),
            }),
            brightness: Some(70),
        };
        let keyboard = KeyboardLayout {
            short: "tr".to_string(),
            name: "Turkish".to_string(),
            index: 0,
            count: 2,
        };
        let mut painter = gevrek_ui::picture::Painter::sized(size)
            .expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        let settings = gevrek_ui::Settings::default();
        theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208));
        theme::apply(&context, &palette, &settings);
        let mut pixels = Vec::new();
        for step in 0..4 {
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.4, Vec::new(), |ui| {
                draw(
                    ui,
                    &Shown {
                        bar: &bar,
                        view: &view,
                        accent: egui::Color32::from_rgb(64, 224, 208),
                        quiet: false,
                        notices: 0,
                        mode: "",
                        recording: false,
                        sharing: false,
                        output: "DP-1",
                        tray: &[],
                        system: &system,
                        keyboard: Some(&keyboard),
                        extras: &extras,
                    },
                    None,
                );
            });
        }
        painter.save(&pixels, &out).expect("the picture is written");
        println!("drawn: {}", out.display());
    }
}
