use crate::command::{DispatchCommand, WindowSelector, WorkspaceSelector};
use crate::focus::FocusConfig;
use crate::geometry::Rect;
use crate::graph::BorderStyle;
use crate::layout::{DwindleDirection, LayoutType, MasterPosition, NewWindowPlacement};
use crate::platform::windows::{SystemBackdrop, WindowCorner};
use crate::rules::{MatchKind, MatchStrategy};
use crate::zones::{presets as zone_presets, ZoneConfig, ZoneLayout};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const DESKTOP_CONFIG_SCHEMA_VERSION: u32 = 25;

/// What is wrong with the pattern in a command's window selector, where
/// it has one that cannot be read (`re:` and what follows is a regular
/// expression). A selector read from words on a command line is checked
/// as it is read; one out of a file was not checked at all.
fn selector_pattern_problem(command: &DispatchCommand) -> Option<String> {
    let written = serde_json::to_value(command).ok()?;
    let mut problem = None;
    // (Only what selects a window: a program's arguments and a
    // workspace's name may begin with any letters.)
    fn within_selector(value: &serde_json::Value, problem: &mut Option<String>) {
        match value {
            serde_json::Value::String(text) => {
                if let Some(pattern) = text.strip_prefix("re:") {
                    if let Err(error) = regex::Regex::new(pattern) {
                        *problem = Some(format!(
                            "The pattern '{pattern}' is no regular expression: {error}"
                        ));
                    }
                }
            }
            serde_json::Value::Array(items) => {
                items.iter().for_each(|item| within_selector(item, problem))
            }
            serde_json::Value::Object(fields) => fields
                .values()
                .for_each(|field| within_selector(field, problem)),
            _ => {}
        }
    }
    if let serde_json::Value::Object(fields) = &written {
        for (key, field) in fields {
            // (A workspace is selected by `selector` too, by its name,
            // which is no pattern.)
            if matches!(key.as_str(), "selector" | "window")
                && field["kind"].as_str() != Some("name")
            {
                within_selector(field, &mut problem);
            }
        }
    }
    problem
}

fn window_selector_is_empty(selector: &WindowSelector) -> bool {
    match selector {
        WindowSelector::Active | WindowSelector::Floating | WindowSelector::Tiled => false,
        WindowSelector::Handle(handle) => *handle <= 0,
        WindowSelector::ProcessId(pid) => *pid == 0,
        WindowSelector::Title(value)
        | WindowSelector::InitialTitle(value)
        | WindowSelector::Class(value)
        | WindowSelector::InitialClass(value)
        | WindowSelector::Executable(value)
        | WindowSelector::Path(value)
        | WindowSelector::Tag(value) => value.trim().is_empty(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct DesktopConfig {
    pub schema_version: u32,
    pub focus: FocusConfig,
    pub apply_system_rules: bool,
    /// Watch the configuration file for external changes and hot-reload on
    /// modification. On by default.
    pub watch_configuration: bool,
    /// Adopt windows that already exist when the WM starts into their
    /// monitor's active workspace. When false, existing
    /// windows are left in place and only newly created windows are managed.
    pub manage_existing_on_startup: bool,
    /// After adopting the windows that exist at start, put each back on the
    /// workspace it was on when the WM last stopped, and show the workspace
    /// each monitor showed. Needs `manage_existing_on_startup`. Off by default.
    pub restore_session_on_startup: bool,
    pub appearance: AppearanceProfile,
    /// What only one backend can apply (`platform.windows`,
    /// `platform.wayland`).
    pub platform: crate::platform::PlatformConfig,
    pub animations: AnimationProfile,
    /// Global work-area offset applied to every monitor after the Windows
    /// work area and per-monitor reservations.
    pub work_area_offset: ReservedArea,
    pub monitors: Vec<MonitorProfile>,
    pub workspaces: Vec<WorkspaceProfile>,
    pub zones: ZoneSettings,
    pub startup: Vec<StartupProfile>,
    pub scratchpads: Vec<ScratchpadProfile>,
    pub bindings: Vec<BindingProfile>,
    pub gestures: GestureSettings,
    pub window_rules: Vec<WindowRuleProfile>,
    /// The desktop's own surfaces: the panel.
    pub shell: ShellConfig,
    /// Keyboards, mice and touchpads, where gevrek owns them.
    pub input: crate::input::InputConfig,
    /// The displays: place, mode, scale, primary.
    pub outputs: Vec<crate::outputs::OutputProfile>,
    /// The session: what every program in it is told.
    pub session: crate::session::SessionConfig,
    /// What is behind the windows.
    pub wallpaper: crate::wallpaper::WallpaperConfig,
    /// Warmer colours in the evening.
    pub night_light: crate::night_light::NightLightConfig,
    /// Rules for the surfaces of shells (panels, launchers,
    /// notifications), by the name each says it has. Where gevrek draws
    /// the desktop (Linux).
    pub layer_rules: Vec<crate::layer_rules::LayerRule>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            schema_version: DESKTOP_CONFIG_SCHEMA_VERSION,
            focus: FocusConfig::default(),
            apply_system_rules: true,
            // (A change of the file is a change of the desktop: the
            // plan's principle 7. Off, a file edited by hand or by
            // another program waited for the next login without a word.)
            watch_configuration: true,
            manage_existing_on_startup: true,
            restore_session_on_startup: false,
            appearance: AppearanceProfile::default(),
            platform: Default::default(),
            animations: AnimationProfile::default(),
            work_area_offset: ReservedArea::default(),
            monitors: Vec::new(),
            workspaces: Vec::new(),
            zones: ZoneSettings::default(),
            startup: Vec::new(),
            scratchpads: Vec::new(),
            bindings: Vec::new(),
            gestures: GestureSettings::default(),
            window_rules: Vec::new(),
            shell: ShellConfig::default(),
            input: Default::default(),
            outputs: Vec::new(),
            session: Default::default(),
            wallpaper: Default::default(),
            night_light: Default::default(),
            layer_rules: Vec::new(),
        }
    }
}

/// The desktop's own surfaces.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ShellConfig {
    /// The panel at an edge of every display.
    pub panel: PanelProfile,
    /// Where notifications are shown, how many at once and for how long.
    pub notifications: NotificationsProfile,
    /// Where pictures of the screen go.
    pub screenshots: ScreenshotsProfile,
    /// The look of the control centre and the notifications' panel:
    /// frosted glass over the wallpaper, or plain.
    pub glass: GlassProfile,
    /// What is said in the middle of the screen for a moment.
    pub osd: OsdProfile,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// The on-screen display: what is said in the middle of the screen for a
/// moment when something changes (how loud the sound is, do not disturb).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct OsdProfile {
    /// A change of the volume is shown. Off: it is left to the system's
    /// own display (Windows shows one for its volume keys); on Linux
    /// nothing else shows it, and the volume changes without a word.
    pub volume: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for OsdProfile {
    fn default() -> Self {
        Self {
            volume: true,
            extra: Default::default(),
        }
    }
}

/// Frosted glass for the shell's larger surfaces (the control centre,
/// the notifications' panel): what is on the display shows through
/// them blurred, under a tint, as it was when the surface was opened
/// (the wallpaper alone where no picture of the display can be had).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GlassProfile {
    /// Off: the surfaces are plain, of one colour.
    pub enabled: bool,
    /// How dark the glass is, from 0 (the blurred wallpaper as it is) to
    /// 255 (nothing of it is seen).
    pub tint: u8,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for GlassProfile {
    fn default() -> Self {
        Self {
            enabled: true,
            tint: 165,
            extra: Default::default(),
        }
    }
}

/// Where the pictures the screenshot keys take go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ScreenshotsProfile {
    /// The folder they are written into (`~` is the home folder). Empty:
    /// `Screenshots` in the pictures folder.
    pub folder: String,
    /// Put each on the clipboard as well, to be pasted at once.
    pub clipboard: bool,
    /// Write each into the folder. (With this off and the clipboard on, a
    /// picture is only there to be pasted.)
    pub save: bool,
    /// A recording of the screen has what the machine plays in it, too
    /// (not the microphone). Off: a film without sound.
    pub record_sound: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for ScreenshotsProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            folder: String::new(),
            clipboard: true,
            save: true,
            record_sound: false,
        }
    }
}

/// The corner of the display notifications are shown in.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCorner {
    #[default]
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

/// How notifications are shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NotificationsProfile {
    pub corner: NotificationCorner,
    /// How many are shown at once; the others wait their turn. 1 to 10.
    pub shown: u32,
    /// How long one stays that does not say itself, in seconds. 1 to 60.
    pub stay_seconds: u32,
    /// A sound when one arrives (the desktop's sound for a message, or
    /// the one the sender names). Not while do not disturb is on, unless
    /// the sender says it cannot wait; not when the sender asks for none.
    pub sound: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for NotificationsProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            corner: NotificationCorner::TopRight,
            shown: 5,
            stay_seconds: 6,
            sound: false,
        }
    }
}

impl NotificationsProfile {
    /// How many are shown at once, within what makes sense.
    pub fn shown_at_once(&self) -> usize {
        self.shown.clamp(1, 10) as usize
    }

    /// How long one stays, within what makes sense.
    pub fn stay(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.stay_seconds.clamp(1, 60)))
    }
}

/// Where the panel is on a display.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PanelEdge {
    #[default]
    #[serde(alias = "Top")]
    Top,
    #[serde(alias = "Bottom")]
    Bottom,
    /// Down the left side: what the panel shows stands under each other
    /// (`height` is then how wide the panel is).
    Left,
    /// Down the right side.
    Right,
}

impl PanelEdge {
    /// Down a side of the display, not along its top or bottom.
    pub fn is_upright(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// How wide the panel is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PanelFit {
    /// From one side of the display to the other.
    #[default]
    Full,
    /// As wide as what it shows.
    Content,
}

/// Where a panel that is not as wide as the display stands along its edge.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PanelAlign {
    Start,
    #[default]
    Centre,
    End,
}

/// How the panel is drawn.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PanelStyle {
    /// One strip with everything on it.
    #[default]
    Bar,
    /// Each group (left, middle, right) on a piece of its own, with the
    /// desktop showing between them.
    Islands,
    /// No background: what it shows stands on the desktop itself.
    Bare,
}

/// One thing a panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanelModule {
    /// A button that opens the launcher.
    Launcher,
    /// The workspaces of the display; a press shows one.
    Workspaces,
    /// The layout of the shown workspace; a press opens the menu of layouts.
    Layout,
    /// The title of the focused window.
    Title,
    /// The time, as `clock_format` writes it.
    Clock,
    /// How many windows the shown workspace has.
    Stats,
    /// A bell: the notifications that were shown, and do-not-disturb.
    Notifications,
    /// A button that opens the power menu.
    Power,
    /// How loud the sound is: the wheel over it makes it louder and
    /// quieter, a press mutes it.
    Sound,
    /// How full the battery is, on a machine that has one.
    Battery,
    /// Whether the machine is on a network, and which.
    Network,
    /// The icons of programs that run in the background. (A panel that
    /// does not name it has them beside its marks all the same: taking
    /// the module off does not hide them.)
    Tray,
    /// The windows of the shown workspace, each a button that goes to
    /// it.
    Taskbar,
    /// How bright the machine's own display is, on a machine where that
    /// can be set (a laptop); the wheel over it makes it brighter and
    /// darker.
    Brightness,
    /// The keyboard layout in use (`tr`); a press goes to the next of
    /// `desktop.input.keyboard.layouts`, the other button to the one
    /// before.
    Keyboard,
    /// A gap between two modules.
    Spacer,
    /// A module this version does not know, by its name: kept in its
    /// place and written back as it was, not drawn.
    #[serde(untagged)]
    #[schemars(skip)]
    Other(String),
}

impl PanelModule {
    /// The modules this version draws.
    pub fn all() -> &'static [PanelModule] {
        &[
            Self::Launcher,
            Self::Workspaces,
            Self::Layout,
            Self::Title,
            Self::Clock,
            Self::Stats,
            Self::Notifications,
            Self::Power,
            Self::Sound,
            Self::Battery,
            Self::Network,
            Self::Tray,
            Self::Taskbar,
            Self::Brightness,
            Self::Keyboard,
            Self::Spacer,
        ]
    }
}

/// The panel: what it shows at its left, in its middle and at its right,
/// on every display. Its room is kept free of windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PanelProfile {
    /// Show the panel and keep its place free of windows.
    pub enabled: bool,
    pub edge: PanelEdge,
    /// How it is drawn: one strip, a piece for each group, or nothing
    /// behind what it shows.
    pub style: PanelStyle,
    /// From side to side, or as wide as what it shows.
    pub fit: PanelFit,
    /// A width of its own in pixels (0: by `fit`).
    pub width: u32,
    /// Where it stands along its edge when it is not as wide as the
    /// display: at the start, in the middle or at the end.
    pub align: PanelAlign,
    /// It floats over the windows: no room is kept free for it, and
    /// windows reach to the display's edge under it.
    pub floating: bool,
    /// Height of the panel in pixels.
    pub height: u32,
    /// Room between the panel and the edges of the screen, in pixels (0:
    /// the panel reaches from edge to edge).
    pub margin: u32,
    pub corner_radius: u32,
    /// Room between the panel's ends and what it shows, in pixels.
    pub padding: u32,
    /// Room between two things it shows, in pixels.
    pub spacing: u32,
    /// 0-255.
    pub opacity: u8,
    pub background: (u8, u8, u8),
    pub text: (u8, u8, u8),
    /// The colour of what is active; the active border's colour when not given.
    pub accent: Option<(u8, u8, u8)>,
    /// What stands at the left, in this order.
    pub left: Vec<PanelModule>,
    /// What stands in the middle.
    pub centre: Vec<PanelModule>,
    /// What stands at the right, the last one at the edge.
    pub right: Vec<PanelModule>,
    /// How wide the title of the focused window is, in pixels: it keeps
    /// that width whatever it says, and a longer title is cut off with
    /// "…" (0: as wide as its words, which moves everything after it
    /// when they change).
    pub title_width: u32,
    /// How wide a window's button on the taskbar is at most, in pixels;
    /// every button is that wide, or narrower when there are many (0:
    /// each as wide as its words).
    pub task_width: u32,
    /// Whether the taskbar has the windows of every workspace of its
    /// display, or only those of the workspace that is shown. (The
    /// taskbar has a button of its own that says which, and changes it.)
    #[serde(default)]
    pub tasks_of_all_workspaces: bool,
    /// The clock, in `strftime` form.
    pub clock_format: String,
    /// Show the icons of older programs in the tray too: those that
    /// embed a little window in the panel, X11's way (Wine, old Java and
    /// GTK 2 programs). They are carried over by a program of its own
    /// (`xembedsniproxy`), where that is installed and X11 programs can
    /// run; without it nothing is different.
    pub older_tray_icons: bool,
    /// What is otherwise on single displays: no panel there, or other
    /// things on it. A display that is not named has the panel as it is
    /// set here.
    pub displays: Vec<PanelOnDisplay>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// The panel of one display, where it is other than the others'.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct PanelOnDisplay {
    /// The display, by its name (`DP-2`, `HDMI-A-1`; upper and lower
    /// case are the same).
    pub name: String,
    /// Off: this display has no panel.
    pub enabled: bool,
    /// What stands at its left, in its middle and at its right there;
    /// not given: as on the other displays.
    pub left: Option<Vec<PanelModule>>,
    pub centre: Option<Vec<PanelModule>>,
    pub right: Option<Vec<PanelModule>>,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for PanelOnDisplay {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            left: None,
            centre: None,
            right: None,
            extra: Default::default(),
        }
    }
}

impl Default for PanelProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: true,
            edge: PanelEdge::Top,
            style: PanelStyle::Bar,
            fit: PanelFit::Full,
            width: 0,
            align: PanelAlign::Centre,
            floating: false,
            height: 36,
            margin: 6,
            corner_radius: 10,
            padding: 10,
            spacing: 8,
            opacity: 235,
            background: (17, 19, 27),
            text: (214, 220, 232),
            accent: None,
            left: vec![
                PanelModule::Launcher,
                PanelModule::Workspaces,
                PanelModule::Layout,
                PanelModule::Title,
            ],
            centre: Vec::new(),
            right: vec![
                PanelModule::Tray,
                PanelModule::Network,
                PanelModule::Sound,
                PanelModule::Battery,
                PanelModule::Clock,
            ],
            title_width: 260,
            task_width: 150,
            tasks_of_all_workspaces: false,
            clock_format: "%a %d %b  %H:%M".to_string(),
            older_tray_icons: true,
            displays: Vec::new(),
        }
    }
}

impl PanelProfile {
    /// The panel as it is on the display of this name: with what is set
    /// for that display in place of the common settings; `None` where
    /// that display has no panel (or the panel is switched off).
    pub fn on(&self, display: &str) -> Option<PanelProfile> {
        if !self.enabled {
            return None;
        }
        let own = self.displays.iter().find(|entry| {
            !entry.name.trim().is_empty() && entry.name.trim().eq_ignore_ascii_case(display)
        });
        let Some(own) = own else {
            return Some(self.clone());
        };
        if !own.enabled {
            return None;
        }
        let mut panel = self.clone();
        if let Some(left) = &own.left {
            panel.left = left.clone();
        }
        if let Some(centre) = &own.centre {
            panel.centre = centre.clone();
        }
        if let Some(right) = &own.right {
            panel.right = right.clone();
        }
        Some(panel)
    }

    /// How much of the monitor's edge is kept free of windows.
    pub fn reserved(&self) -> i32 {
        if self.enabled && !self.floating {
            (self.height.clamp(16, 200) + self.margin.min(100)) as i32
        } else {
            0
        }
    }

    /// The module is somewhere on the panel.
    pub fn shows(&self, module: PanelModule) -> bool {
        self.left
            .iter()
            .chain(&self.centre)
            .chain(&self.right)
            .any(|shown| *shown == module)
    }

    /// Put a module on the panel (at the left what is about the windows,
    /// at the right the rest, before the clock), or take it off.
    pub fn set_shown(&mut self, module: PanelModule, shown: bool) {
        if shown == self.shows(module.clone()) {
            return;
        }
        if !shown {
            for side in [&mut self.left, &mut self.centre, &mut self.right] {
                side.retain(|there| *there != module);
            }
        } else if matches!(module, PanelModule::Launcher) {
            self.left.insert(0, module);
        } else if matches!(
            module,
            PanelModule::Workspaces
                | PanelModule::Layout
                | PanelModule::Title
                | PanelModule::Taskbar
        ) {
            self.left.push(module);
        } else if matches!(module, PanelModule::Clock | PanelModule::Power) {
            self.right.push(module);
        } else {
            let before_clock = self
                .right
                .iter()
                .position(|there| matches!(there, PanelModule::Clock | PanelModule::Power));
            self.right
                .insert(before_clock.unwrap_or(self.right.len()), module);
        }
    }
}

impl MonitorSelector {
    /// How closely it names a display: a display's own name before its
    /// place among the displays, that before "the primary one", that
    /// before "every display". Of two entries for one workspace that both
    /// mean a display, the closer one counts there.
    pub fn closeness(&self) -> u8 {
        match self {
            Self::Name(_) => 3,
            Self::Index(_) => 2,
            Self::Primary => 1,
            Self::Any => 0,
        }
    }
}

/// A display, as far as a configuration can name one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayRef {
    /// Every name it answers to (its name, its connector, its device).
    pub names: Vec<String>,
    /// Its place among the displays, from 0, where that is known.
    pub index: Option<usize>,
    pub primary: bool,
}

impl DisplayRef {
    /// A display known by its name alone.
    pub fn named(name: &str) -> Self {
        Self {
            names: vec![name.trim().to_string()],
            index: None,
            primary: false,
        }
    }

    /// A display of the window manager's state (one of its `monitors`),
    /// with its place among them where that is known. Its names, the one
    /// a configuration writes for it first: its connector (on Windows the
    /// device's path, which no two displays share), then its name.
    pub fn of_state(monitor: &serde_json::Value, index: Option<usize>) -> Self {
        let mut names: Vec<String> = Vec::new();
        for name in [monitor["connector"].as_str(), monitor["name"].as_str()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if !names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                names.push(name.to_string());
            }
        }
        Self {
            names,
            index,
            primary: monitor["is_primary"].as_bool().unwrap_or(false),
        }
    }

    /// The name a configuration writes for it: the first it has.
    pub fn name(&self) -> &str {
        self.names.first().map_or("", String::as_str)
    }

    pub fn is_called(&self, wanted: &str) -> bool {
        let wanted = wanted.trim();
        !wanted.is_empty()
            && self
                .names
                .iter()
                .any(|name| name.trim().eq_ignore_ascii_case(wanted))
    }

    /// Whether a selector means this display.
    pub fn is_meant_by(&self, selector: &MonitorSelector) -> bool {
        match selector {
            MonitorSelector::Any => true,
            MonitorSelector::Primary => self.primary,
            MonitorSelector::Index(index) => self.index == Some(*index),
            MonitorSelector::Name(name) => self.is_called(name),
        }
    }
}

/// What came of giving a workspace a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Titled {
    /// It has the title now.
    Changed,
    /// It had it already.
    Same,
    /// No workspace of that name is written down for the display.
    Unknown,
    /// Another workspace of the display is shown by that title, or named
    /// so.
    Taken,
}

impl Titled {
    pub fn changed(self) -> bool {
        self == Self::Changed
    }
}

fn same_name(one: &str, other: &str) -> bool {
    one.trim().eq_ignore_ascii_case(other.trim())
}

/// Of a list of workspace entries, the ones a display has
/// (`DesktopConfig::workspaces_where`).
pub fn workspaces_where(
    profiles: &[WorkspaceProfile],
    means: impl Fn(&MonitorSelector) -> bool,
) -> Vec<&WorkspaceProfile> {
    let mut chosen: Vec<&WorkspaceProfile> = Vec::new();
    for profile in profiles.iter().filter(|profile| means(&profile.monitor)) {
        match chosen
            .iter_mut()
            .find(|there| same_name(&there.name, &profile.name))
        {
            Some(there) => {
                if profile.monitor.closeness() > there.monitor.closeness() {
                    *there = profile;
                }
            }
            None => chosen.push(profile),
        }
    }
    chosen.retain(|profile| profile.enabled && !profile.name.trim().is_empty());
    chosen
}

impl WorkspaceProfile {
    /// What is shown of it: its title, or its name where it has none.
    pub fn label(&self) -> &str {
        match self.title.as_deref().map(str::trim) {
            Some(title) if !title.is_empty() => title,
            _ => self.name.trim(),
        }
    }

    /// The title to write down for what a person typed: none for nothing
    /// and for the workspace's own name.
    pub fn title_for(&self, typed: &str) -> Option<String> {
        let typed = typed.trim();
        (!typed.is_empty() && typed != self.name.trim()).then(|| typed.to_string())
    }
}

impl DesktopConfig {
    /// The workspaces a display has, in the order they are written in:
    /// for each name the entry that names the display most closely (its
    /// own before the one for every display), and of those only the ones
    /// that are switched on. `means` says whether a selector means the
    /// display (`DisplayRef::is_meant_by`, or the window manager's own
    /// knowledge of its displays).
    pub fn workspaces_where(
        &self,
        means: impl Fn(&MonitorSelector) -> bool,
    ) -> Vec<&WorkspaceProfile> {
        workspaces_where(&self.workspaces, means)
    }

    /// The workspaces of a display (`workspaces_where`).
    pub fn workspaces_on(&self, display: &DisplayRef) -> Vec<&WorkspaceProfile> {
        self.workspaces_where(|selector| display.is_meant_by(selector))
    }

    /// The entry a display has for the workspace `name`.
    pub fn workspace_on(&self, display: &DisplayRef, name: &str) -> Option<&WorkspaceProfile> {
        self.workspaces_on(display)
            .into_iter()
            .find(|profile| same_name(&profile.name, name))
    }

    /// Give the workspace `name` of one display the title `title` (what
    /// is shown of it), and leave the workspaces of that name on the other
    /// displays as they are. Nothing that names the workspace changes: a
    /// title is only what is shown.
    ///
    /// A workspace that is written down for this display by its name is
    /// given the title. One that it has from an entry for several
    /// displays keeps that entry, and the display gets one of its own, a
    /// copy with the title, right after it. An entry of the display's own
    /// that says nothing else than the one it stands for is taken away
    /// again (the title given back).
    ///
    /// An empty title, or the workspace's name, is no title: the
    /// workspace is shown by its name.
    pub fn title_workspace_on(&mut self, display: &DisplayRef, name: &str, title: &str) -> Titled {
        if display.name().is_empty() {
            return Titled::Unknown;
        }
        let Some(current) = self.workspace_on(display, name) else {
            return Titled::Unknown;
        };
        let wanted = current.title_for(title);
        if current.title_for(current.label()) == wanted {
            return Titled::Same;
        }
        let shown = wanted
            .clone()
            .unwrap_or_else(|| current.name.trim().to_string());
        let taken = self.workspaces_on(display).into_iter().any(|other| {
            !same_name(&other.name, name)
                && (same_name(other.label(), &shown) || same_name(&other.name, &shown))
        });
        if taken {
            return Titled::Taken;
        }
        let own = |profile: &WorkspaceProfile| {
            matches!(&profile.monitor, MonitorSelector::Name(named) if display.is_called(named))
                && same_name(&profile.name, name)
        };
        let at = match self.workspaces.iter().position(own) {
            Some(at) => at,
            None => {
                // (Right after the one it stands for: a display's
                // workspaces are in the order they are written in.)
                let Some(shared) = self.workspaces.iter().position(|profile| {
                    same_name(&profile.name, name) && display.is_meant_by(&profile.monitor)
                }) else {
                    return Titled::Unknown;
                };
                let mut entry = self.workspaces[shared].clone();
                entry.monitor = MonitorSelector::Name(display.name().to_string());
                self.workspaces.insert(shared + 1, entry);
                shared + 1
            }
        };
        self.workspaces[at].title = wanted;
        self.drop_needless_entry(display, at);
        Titled::Changed
    }

    /// An entry of a display's own that says exactly what the display
    /// would have without it is taken out.
    fn drop_needless_entry(&mut self, display: &DisplayRef, at: usize) {
        let own = self.workspaces[at].clone();
        let without = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(index, profile)| {
                *index != at
                    && same_name(&profile.name, &own.name)
                    && display.is_meant_by(&profile.monitor)
            })
            .max_by_key(|(_, profile)| profile.monitor.closeness())
            .map(|(_, profile)| profile.clone());
        let Some(mut without) = without else {
            return;
        };
        without.monitor = own.monitor.clone();
        if serde_json::to_value(&without).ok() == serde_json::to_value(&own).ok() {
            self.workspaces.remove(at);
        }
    }

    /// Give the workspace `name` the title `title` on every display: in
    /// every entry that is written down for it.
    pub fn title_workspace(&mut self, name: &str, title: &str) -> Titled {
        if !self.workspaces.iter().any(|profile| same_name(&profile.name, name)) {
            return Titled::Unknown;
        }
        let typed = title.trim();
        let taken = !typed.is_empty()
            && self.workspaces.iter().any(|other| {
                !same_name(&other.name, name)
                    && other.enabled
                    && (same_name(other.label(), typed) || same_name(&other.name, typed))
            });
        if taken {
            return Titled::Taken;
        }
        let mut changed = false;
        for profile in &mut self.workspaces {
            if same_name(&profile.name, name) {
                let wanted = profile.title_for(typed);
                changed |= profile.title != wanted;
                profile.title = wanted;
            }
        }
        // (Entries of single displays that only had another title say
        // nothing of their own now.)
        let mut at = 0;
        while at < self.workspaces.len() {
            let before = self.workspaces.len();
            if let MonitorSelector::Name(display) = self.workspaces[at].monitor.clone() {
                if same_name(&self.workspaces[at].name, name) {
                    self.drop_needless_entry(&DisplayRef::named(&display), at);
                }
            }
            if self.workspaces.len() == before {
                at += 1;
            }
        }
        if changed {
            Titled::Changed
        } else {
            Titled::Same
        }
    }

    /// Write down a workspace that a display has only for now (one that
    /// was opened while gevrek ran), so that it is there the next time
    /// too: an entry of that display's own, after its others, with the
    /// settings of the workspace the display starts with. Nothing happens
    /// when the display has one of that name already. Returns whether it
    /// was written down.
    pub fn keep_workspace_on(&mut self, display: &DisplayRef, name: &str, title: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || display.name().is_empty() {
            return false;
        }
        if self
            .workspaces
            .iter()
            .any(|profile| same_name(&profile.name, name) && display.is_meant_by(&profile.monitor))
        {
            return false;
        }
        let like = {
            let there = self.workspaces_on(display);
            there
                .iter()
                .find(|profile| profile.default)
                .or(there.first())
                .map(|profile| (*profile).clone())
        };
        let mut entry = like.unwrap_or_else(|| WorkspaceProfile {
            // (No borders of its own: the look's.)
            appearance_override: false,
            ..WorkspaceProfile::default()
        });
        entry.name = name.to_string();
        entry.title = None;
        entry.title = entry.title_for(title);
        entry.monitor = MonitorSelector::Name(display.name().to_string());
        entry.default = false;
        entry.persistent = true;
        entry.enabled = true;
        entry.on_created_empty = None;
        self.workspaces.push(entry);
        true
    }

    /// Change the workspace `name` for one display alone: in an entry of
    /// that display's own, made from what the display has now where it
    /// has none yet. What the workspace is named and which display the
    /// entry is for stay as they are, whatever `change` does. An entry of
    /// the display's own that says nothing else than the one it stands
    /// for is taken out again. Returns whether there is such a workspace.
    pub fn edit_workspace_on(
        &mut self,
        display: &DisplayRef,
        name: &str,
        change: impl FnOnce(&mut WorkspaceProfile),
    ) -> bool {
        if display.name().is_empty() {
            return false;
        }
        let own = |profile: &WorkspaceProfile| {
            matches!(&profile.monitor, MonitorSelector::Name(named) if display.is_called(named))
                && same_name(&profile.name, name)
        };
        let at = match self.workspaces.iter().position(own) {
            Some(at) => at,
            None => {
                // (One that is switched off for every display can be had
                // by one display: the entry it is made from need not be
                // on.)
                let Some(shared) = self
                    .workspaces
                    .iter()
                    .enumerate()
                    .filter(|(_, profile)| {
                        same_name(&profile.name, name) && display.is_meant_by(&profile.monitor)
                    })
                    .max_by_key(|(_, profile)| profile.monitor.closeness())
                    .map(|(index, _)| index)
                else {
                    return false;
                };
                let mut entry = self.workspaces[shared].clone();
                entry.monitor = MonitorSelector::Name(display.name().to_string());
                self.workspaces.insert(shared + 1, entry);
                shared + 1
            }
        };
        let (key, monitor) = (
            self.workspaces[at].name.clone(),
            self.workspaces[at].monitor.clone(),
        );
        change(&mut self.workspaces[at]);
        self.workspaces[at].name = key;
        self.workspaces[at].monitor = monitor;
        self.drop_needless_entry(display, at);
        true
    }

    /// Change the workspace `name` on every display: in every entry that
    /// is written down for it. Returns whether there is one.
    pub fn edit_workspace(&mut self, name: &str, change: impl Fn(&mut WorkspaceProfile)) -> bool {
        let mut found = false;
        for profile in &mut self.workspaces {
            if same_name(&profile.name, name) {
                let (key, monitor) = (profile.name.clone(), profile.monitor.clone());
                change(profile);
                profile.name = key;
                profile.monitor = monitor;
                found = true;
            }
        }
        found
    }

    /// The entry of a display's own for the workspace `name`, where it
    /// has one.
    pub fn own_workspace_entry(&self, display: &DisplayRef, name: &str) -> Option<usize> {
        self.workspaces.iter().position(|profile| {
            matches!(&profile.monitor, MonitorSelector::Name(named) if display.is_called(named))
                && same_name(&profile.name, name)
        })
    }

    /// A display has the workspace `name` as every display has it again:
    /// its own entry for it is taken out. Returns whether it had one.
    /// (A workspace that only this display has is left: taking its only
    /// entry out would be taking the workspace away.)
    pub fn share_workspace_on(&mut self, display: &DisplayRef, name: &str) -> bool {
        let Some(at) = self.own_workspace_entry(display, name) else {
            return false;
        };
        let has_another = self.workspaces.iter().enumerate().any(|(index, profile)| {
            index != at && same_name(&profile.name, name) && display.is_meant_by(&profile.monitor)
        });
        if !has_another {
            return false;
        }
        self.workspaces.remove(at);
        true
    }

    /// The workspace `name` is the one a display starts with (`None`: every
    /// display, in every entry). No other workspace is that there.
    pub fn start_with_workspace(&mut self, display: Option<&DisplayRef>, name: &str) {
        match display {
            None => {
                for profile in &mut self.workspaces {
                    profile.default = same_name(&profile.name, name);
                }
            }
            Some(display) => {
                let there: Vec<String> = self
                    .workspaces_on(display)
                    .iter()
                    .map(|profile| profile.name.clone())
                    .collect();
                for key in there {
                    let wanted = same_name(&key, name);
                    let is = self
                        .workspace_on(display, &key)
                        .is_some_and(|profile| profile.default);
                    if is != wanted {
                        self.edit_workspace_on(display, &key, |profile| profile.default = wanted);
                    }
                }
            }
        }
    }

    /// A name for a new workspace: the lowest number none is named or
    /// shown by.
    pub fn free_workspace_name(&self) -> String {
        (1usize..)
            .map(|number| number.to_string())
            .find(|text| {
                !self
                    .workspaces
                    .iter()
                    .any(|profile| same_name(&profile.name, text) || same_name(profile.label(), text))
            })
            .unwrap_or_default()
    }

    /// Write down a new workspace, after the others: for one display, or
    /// (`None`) for every display. It is laid out as the workspace that is
    /// started with, takes its look from the appearance, and stays when it
    /// is empty. Returns its name.
    pub fn add_workspace(&mut self, display: Option<&DisplayRef>) -> String {
        let like = self
            .workspaces
            .iter()
            .find(|profile| profile.enabled && profile.default)
            .or(self.workspaces.iter().find(|profile| profile.enabled))
            .cloned();
        let mut entry = like.unwrap_or_else(|| WorkspaceProfile {
            appearance_override: false,
            layout: crate::layout::LayoutType::Dwindle,
            ..WorkspaceProfile::default()
        });
        entry.name = self.free_workspace_name();
        entry.title = None;
        entry.monitor = match display {
            Some(display) if !display.name().is_empty() => {
                MonitorSelector::Name(display.name().to_string())
            }
            _ => MonitorSelector::Any,
        };
        entry.default = self.workspaces.iter().all(|profile| !profile.enabled);
        entry.persistent = true;
        entry.enabled = true;
        entry.on_created_empty = None;
        let name = entry.name.clone();
        self.workspaces.push(entry);
        name
    }

    /// Take the workspace `name` out of the configuration: every entry
    /// that is written down for it. (The keys that name it are left: they
    /// open a workspace of the moment by that name.) Returns whether
    /// there was one.
    pub fn remove_workspace(&mut self, name: &str) -> bool {
        let before = self.workspaces.len();
        self.workspaces.retain(|profile| !same_name(&profile.name, name));
        self.workspaces.len() != before
    }

    /// A display does not have the workspace `name` (`false`), or has it
    /// again. One that only this display has is taken out for good.
    pub fn have_workspace_on(&mut self, display: &DisplayRef, name: &str, have: bool) -> bool {
        let Some(own) = self.own_workspace_entry(display, name) else {
            return self.edit_workspace_on(display, name, |profile| profile.enabled = have);
        };
        let only_here = !self.workspaces.iter().enumerate().any(|(index, profile)| {
            index != own && same_name(&profile.name, name) && display.is_meant_by(&profile.monitor)
        });
        if only_here && !have {
            self.workspaces.remove(own);
            return true;
        }
        self.edit_workspace_on(display, name, |profile| profile.enabled = have)
    }

    /// Put the workspace `name` before the workspace `before` (`None`: at
    /// the end), with every entry it has. The order they are written in
    /// is the order a display's workspaces are in. Returns whether
    /// anything moved.
    pub fn move_workspace(&mut self, name: &str, before: Option<&str>) -> bool {
        if before.is_some_and(|before| same_name(before, name)) {
            return false;
        }
        let was: Vec<String> = self.workspaces.iter().map(|p| p.name.clone()).collect();
        let (moved, mut rest): (Vec<WorkspaceProfile>, Vec<WorkspaceProfile>) =
            std::mem::take(&mut self.workspaces)
                .into_iter()
                .partition(|profile| same_name(&profile.name, name));
        let at = before
            .and_then(|before| rest.iter().position(|profile| same_name(&profile.name, before)))
            .unwrap_or(rest.len());
        rest.splice(at..at, moved);
        self.workspaces = rest;
        was != self.workspaces.iter().map(|p| p.name.clone()).collect::<Vec<_>>()
    }

    /// Give the workspace `old` the name `new` (the name it is known by
    /// to what names it, not what is shown of it: `title_workspace`), in
    /// everything that is written down of it: its entries, and the keys
    /// and gestures that name it (they go on meaning the same workspace).
    /// Nothing happens
    /// when no workspace is written down by the old name, when the new
    /// one is empty, or when another workspace already has it. Returns
    /// whether anything changed.
    pub fn rename_workspace(&mut self, old: &str, new: &str) -> bool {
        let (old, new) = (old.trim(), new.trim());
        if new.is_empty() || old == new {
            return false;
        }
        let is = |name: &str, wanted: &str| name.trim().eq_ignore_ascii_case(wanted);
        let taken = !old.eq_ignore_ascii_case(new)
            && self.workspaces.iter().any(|profile| is(&profile.name, new));
        if taken || !self.workspaces.iter().any(|profile| is(&profile.name, old)) {
            return false;
        }
        for profile in &mut self.workspaces {
            if is(&profile.name, old) {
                profile.name = new.to_string();
            }
        }
        for binding in &mut self.bindings {
            binding.command.follow_workspace_rename(old, new);
        }
        for gesture in &mut self.gestures.profiles {
            gesture.command.follow_workspace_rename(old, new);
        }
        true
    }

    pub fn validate(&self) -> Vec<ConfigIssue> {
        let mut issues = Vec::new();
        for (path, message) in self
            .input
            .problems()
            .into_iter()
            .chain(self.session.problems())
            .chain(self.night_light.problems())
            .chain(crate::outputs::problems(&self.outputs))
        {
            issues.push(ConfigIssue::error(path, message));
        }
        if self.schema_version > DESKTOP_CONFIG_SCHEMA_VERSION {
            issues.push(ConfigIssue::error(
                "desktop.schema_version",
                format!(
                    "Configuration schema {} is newer than supported schema {}",
                    self.schema_version, DESKTOP_CONFIG_SCHEMA_VERSION
                ),
            ));
        }
        if self.focus.history_size == 0 {
            issues.push(ConfigIssue::error(
                "desktop.focus.history_size",
                "Focus history must contain at least one entry",
            ));
        }
        if !(1..=32).contains(&self.appearance.border_width) {
            issues.push(ConfigIssue::error(
                "desktop.appearance.border_width",
                "Border width must be between 1 and 32 pixels",
            ));
        }
        if !is_supported_border_style(&self.appearance.border_style) {
            issues.push(ConfigIssue::error(
                "desktop.appearance.border_style",
                unsupported_border_style_message(&self.appearance.border_style),
            ));
        }
        // How far a shadow and a glow reach, and how far a shadow lies
        // from its window: what a display can show.
        let effects = &self.appearance.effects;
        if effects.shadow.radius > 200 {
            issues.push(ConfigIssue::error(
                "desktop.appearance.effects.shadow.radius",
                "A shadow's radius must be at most 200 pixels",
            ));
        }
        for (name, offset) in [
            ("offset_x", effects.shadow.offset_x),
            ("offset_y", effects.shadow.offset_y),
        ] {
            if !(-200..=200).contains(&offset) {
                issues.push(ConfigIssue::error(
                    format!("desktop.appearance.effects.shadow.{name}"),
                    "A shadow's offset must be between -200 and 200 pixels",
                ));
            }
        }
        if effects.glow.radius > 200 {
            issues.push(ConfigIssue::error(
                "desktop.appearance.effects.glow.radius",
                "A glow's radius must be at most 200 pixels",
            ));
        }
        let floating = &self.appearance.floating;
        if floating.separate && floating.border_enabled && !(1..=32).contains(&floating.border_width)
        {
            issues.push(ConfigIssue::error(
                "desktop.appearance.floating.border_width",
                "Border width must be between 1 and 32 pixels",
            ));
        }
        if !(0..=5000).contains(&self.animations.session.duration_ms) {
            issues.push(ConfigIssue::error(
                "desktop.animations.session.duration_ms",
                "The change between lock screen and desktop takes at most 5000 milliseconds",
            ));
        }
        if self.appearance.window_opacity < 20 {
            issues.push(ConfigIssue::error(
                "desktop.appearance.window_opacity",
                "Window opacity must be at least 20",
            ));
        }
        if self.appearance.inactive_window_opacity < 20 {
            issues.push(ConfigIssue::error(
                "desktop.appearance.inactive_window_opacity",
                "Inactive window opacity must be at least 20",
            ));
        }
        if !(10..=200).contains(&self.appearance.stackbar.height) {
            issues.push(ConfigIssue::error(
                "desktop.appearance.stackbar.height",
                "Stackbar height must be between 10 and 200 pixels",
            ));
        }
        if let Some(palette) = &self.appearance.theme.palette {
            if crate::theme::find_palette(palette).is_none() {
                issues.push(ConfigIssue::error(
                    "desktop.appearance.theme.palette",
                    format!(
                        "Unknown Base16 palette '{palette}'. Built-in palettes: {}",
                        crate::theme::palette_names()
                    ),
                ));
            }
        }
        if let Some(accent) = &self.appearance.theme.accent {
            if crate::theme::accent_index(accent).is_none()
                && !crate::theme::follows_wallpaper(&self.appearance.theme)
            {
                issues.push(ConfigIssue::error(
                    "desktop.appearance.theme.accent",
                    format!(
                        "Unknown Base16 accent '{accent}'. Use a slot name like base0d (base00..base0f), or 'wallpaper' for the colour of the wallpaper"
                    ),
                ));
            }
        }
        if self.appearance.invisible_borders.enabled
            && !(1..=200).contains(&self.appearance.invisible_borders.size)
        {
            issues.push(ConfigIssue::error(
                "desktop.appearance.invisible_borders.size",
                "Invisible border size must be between 1 and 200 pixels",
            ));
        }
        for (edge, value) in [
            ("left", self.work_area_offset.left),
            ("top", self.work_area_offset.top),
            ("right", self.work_area_offset.right),
            ("bottom", self.work_area_offset.bottom),
        ] {
            if !(0..=10_000).contains(&value) {
                issues.push(ConfigIssue::error(
                    format!("desktop.work_area_offset.{edge}"),
                    "Work-area offset must be between 0 and 10000 pixels",
                ));
            }
        }
        if !(30..=240).contains(&self.animations.fps) {
            issues.push(ConfigIssue::error(
                "desktop.animations.fps",
                "Animation fps must be between 30 and 240",
            ));
        }
        for (name, channel) in self.animations.channels() {
            if !(16..=5000).contains(&channel.duration_ms) {
                issues.push(ConfigIssue::error(
                    format!("desktop.animations.{name}.duration_ms"),
                    "Animation duration must be between 16 and 5000 milliseconds",
                ));
            }
        }

        for (index, monitor) in self.monitors.iter().enumerate() {
            let path = format!("desktop.monitors[{index}]");
            if matches!(&monitor.monitor, MonitorSelector::Name(name) if name.trim().is_empty()) {
                issues.push(ConfigIssue::error(
                    format!("{path}.monitor"),
                    "Named monitor profiles require a monitor name",
                ));
            }
            if monitor.index_preference.is_some()
                && !matches!(&monitor.monitor, MonitorSelector::Name(_))
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.index_preference"),
                    "Monitor index preferences require a name: monitor selector (the stable display name keys the preference)",
                ));
            }
            for (edge, value) in [
                ("left", monitor.reserved.left),
                ("top", monitor.reserved.top),
                ("right", monitor.reserved.right),
                ("bottom", monitor.reserved.bottom),
            ] {
                if !(0..=10_000).contains(&value) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.reserved.{edge}"),
                        "Reserved monitor space must be between 0 and 10000 pixels",
                    ));
                }
            }
        }

        let mut startup_ids = HashSet::new();
        let mut enabled_startup_ids = HashSet::new();
        for (index, profile) in self.startup.iter().enumerate() {
            let path = format!("desktop.startup[{index}]");
            let id = profile.id.trim().to_ascii_lowercase();
            if id.is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.id"),
                    "Startup application ID cannot be empty",
                ));
            } else if !startup_ids.insert(id) {
                issues.push(ConfigIssue::error(
                    format!("{path}.id"),
                    format!("Duplicate startup application ID '{}'", profile.id),
                ));
            }
            if profile.enabled {
                enabled_startup_ids.insert(profile.id.trim().to_ascii_lowercase());
            }
            validate_launch_profile(&profile.launch, &format!("{path}.launch"), &mut issues);
            if profile.delay_ms > 300_000 {
                issues.push(ConfigIssue::error(
                    format!("{path}.delay_ms"),
                    "Startup delay cannot exceed five minutes",
                ));
            }
            if profile.restart_delay_ms > 300_000 {
                issues.push(ConfigIssue::error(
                    format!("{path}.restart_delay_ms"),
                    "Restart delay cannot exceed five minutes",
                ));
            }
        }

        let mut workspace_names = HashSet::new();
        for (index, workspace) in self.workspaces.iter().enumerate() {
            let path = format!("desktop.workspaces[{index}]");
            if workspace.enabled && workspace.name.trim().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.name"),
                    "Workspace name cannot be empty",
                ));
            }
            // (One entry for a workspace and the same displays: a display
            // may have one of its own beside the one for every display.)
            let entry = (
                workspace.name.trim().to_lowercase(),
                serde_json::to_string(&workspace.monitor)
                    .unwrap_or_default()
                    .to_lowercase(),
            );
            if !workspace_names.insert(entry) {
                issues.push(ConfigIssue::error(
                    format!("{path}.name"),
                    format!(
                        "Workspace '{}' is written down twice for the same display",
                        workspace.name
                    ),
                ));
            }
            if workspace
                .title
                .as_deref()
                .is_some_and(|title| title.chars().count() > 64)
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.title"),
                    "A workspace's title cannot be longer than 64 letters",
                ));
            }
            // Room between and around the windows: nothing less than
            // none, and nothing that leaves the windows no room at all.
            for (name, gap) in [("gaps_in", workspace.gaps_in), ("gaps_out", workspace.gaps_out)] {
                if !(0..=500).contains(&gap) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.{name}"),
                        "A gap must be between 0 and 500 pixels",
                    ));
                }
            }
            if workspace.appearance_override && !(1..=32).contains(&workspace.border_width) {
                issues.push(ConfigIssue::error(
                    format!("{path}.border_width"),
                    "Border width must be between 1 and 32 pixels",
                ));
            }
            if !(0.1..=0.9).contains(&workspace.master_ratio) {
                issues.push(ConfigIssue::error(
                    format!("{path}.master_ratio"),
                    "Master ratio must be between 0.1 and 0.9",
                ));
            }
            if !(1..=8).contains(&workspace.master_count) {
                issues.push(ConfigIssue::error(
                    format!("{path}.master_count"),
                    "Master count must be between 1 and 8",
                ));
            }
            if workspace.appearance_override && !is_supported_border_style(&workspace.border_style)
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.border_style"),
                    unsupported_border_style_message(&workspace.border_style),
                ));
            }
            for (edge, value) in [
                ("left", workspace.work_area_offset.left),
                ("top", workspace.work_area_offset.top),
                ("right", workspace.work_area_offset.right),
                ("bottom", workspace.work_area_offset.bottom),
            ] {
                if !(0..=10_000).contains(&value) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.work_area_offset.{edge}"),
                        "Workspace work-area offset must be between 0 and 10000 pixels",
                    ));
                }
            }
            if let Some(launch) = &workspace.on_created_empty {
                validate_launch_profile(launch, &format!("{path}.on_created_empty"), &mut issues);
            }
        }

        if !(0.0..=1.0).contains(&self.zones.behavior.snap_threshold) {
            issues.push(ConfigIssue::error(
                "desktop.zones.behavior.snap_threshold",
                "Zone snap threshold must be between 0 and 1",
            ));
        }
        if !(0.0..=1.0).contains(&self.zones.behavior.preview_opacity) {
            issues.push(ConfigIssue::error(
                "desktop.zones.behavior.preview_opacity",
                "Zone preview opacity must be between 0 and 1",
            ));
        }
        Self::validate_zone_layout(
            &self.zones.default_layout,
            "desktop.zones.default_layout",
            &mut issues,
        );
        for (index, monitor_layout) in self.zones.monitor_layouts.iter().enumerate() {
            let path = format!("desktop.zones.monitor_layouts[{index}]");
            if matches!(&monitor_layout.monitor, MonitorSelector::Name(name) if name.trim().is_empty())
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.monitor"),
                    "Named zone monitor selectors require a name",
                ));
            }
            Self::validate_zone_layout(
                &monitor_layout.layout,
                &format!("{path}.layout"),
                &mut issues,
            );
        }

        let mut binding_chords = HashSet::new();
        let mut binding_input_policies = std::collections::HashMap::new();
        let mut binding_submaps = HashSet::from(["default".to_string()]);
        for binding in &self.bindings {
            let submap = binding.normalized_submap();
            if !submap.is_empty() {
                binding_submaps.insert(submap);
            }
        }
        let mut scratchpad_ids = HashSet::new();
        let mut enabled_scratchpad_ids = HashSet::new();
        for (index, scratchpad) in self.scratchpads.iter().enumerate() {
            let path = format!("desktop.scratchpads[{index}]");
            let id = scratchpad.id.trim().to_ascii_lowercase();
            if id.is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.id"),
                    "Scratchpad ID cannot be empty",
                ));
            } else if !scratchpad_ids.insert(id.clone()) {
                issues.push(ConfigIssue::error(
                    format!("{path}.id"),
                    format!("Duplicate scratchpad ID '{}'", scratchpad.id),
                ));
            }
            if !scratchpad.enabled {
                // (Its pattern is checked all the same: it can be read
                // once the scratchpad is switched on.)
                if matches!(scratchpad.match_strategy, MatchStrategy::Regex)
                    && regex::Regex::new(&scratchpad.pattern).is_err()
                {
                    issues.push(ConfigIssue::error(
                        format!("{path}.pattern"),
                        "Scratchpad pattern is an invalid regular expression",
                    ));
                }
                continue;
            }
            enabled_scratchpad_ids.insert(id);
            if !matches!(
                scratchpad.match_kind,
                MatchKind::Title | MatchKind::Class | MatchKind::Exe | MatchKind::Path
            ) {
                issues.push(ConfigIssue::error(
                    format!("{path}.match_kind"),
                    "Scratchpads can match title, class, executable, or path",
                ));
            }
            if scratchpad.pattern.trim().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.pattern"),
                    "Scratchpad match pattern cannot be empty",
                ));
            } else if matches!(scratchpad.match_strategy, MatchStrategy::Regex)
                && regex::RegexBuilder::new(&scratchpad.pattern)
                    .case_insensitive(!scratchpad.case_sensitive)
                    .build()
                    .is_err()
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.pattern"),
                    "Scratchpad pattern contains an invalid regular expression",
                ));
            }
            if !(1..=100).contains(&scratchpad.width_percent)
                || !(1..=100).contains(&scratchpad.height_percent)
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.size"),
                    "Scratchpad width and height must be between 1 and 100 percent",
                ));
            }
            if matches!(&scratchpad.monitor, MonitorSelector::Name(name) if name.trim().is_empty())
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.monitor"),
                    "Named scratchpad monitor selectors require a name",
                ));
            }
            if scratchpad
                .launch
                .as_ref()
                .is_some_and(|launch| launch.program.trim().is_empty())
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.launch.program"),
                    "Scratchpad launch program cannot be empty",
                ));
            }
            if let Some(launch) = &scratchpad.launch {
                validate_launch_environment(launch, &format!("{path}.launch"), &mut issues);
            }
        }

        for (index, binding) in self.bindings.iter().enumerate() {
            let path = format!("desktop.bindings[{index}]");
            if !binding.enabled {
                continue;
            }
            if binding.key.trim().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.key"),
                    "Binding key cannot be empty",
                ));
            }
            if binding.normalized_submap().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.submap"),
                    "Binding submap cannot be empty",
                ));
            }
            if binding.device == BindingDevice::Mouse
                && !matches!(
                    binding.normalized_key().as_str(),
                    "LEFT" | "RIGHT" | "MIDDLE" | "X1" | "X2" | "WHEEL_UP" | "WHEEL_DOWN"
                )
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.key"),
                    "Mouse bindings use LEFT, RIGHT, MIDDLE, X1, X2, WHEEL_UP, or WHEEL_DOWN",
                ));
            }
            if binding.device == BindingDevice::Keyboard
                && crate::keys::key_name_to_vk(&binding.normalized_key()).is_none()
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.key"),
                    format!("Unknown keyboard key '{}'", binding.key.trim()),
                ));
            }
            if binding.device == BindingDevice::Mouse
                && binding.normalized_key().starts_with("WHEEL_")
                && binding.trigger != BindingTrigger::Press
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.trigger"),
                    "Mouse wheel bindings only support the press trigger",
                ));
            }
            if binding.trigger == BindingTrigger::LongPress
                && !(100..=10_000).contains(&binding.long_press_ms)
            {
                issues.push(ConfigIssue::error(
                    format!("{path}.long_press_ms"),
                    "Long-press duration must be between 100 and 10000 milliseconds",
                ));
            }
            let chord = binding.chord_key();
            if !binding_chords.insert(chord) {
                issues.push(ConfigIssue::error(
                    path.clone(),
                    "Another enabled binding uses the same key chord",
                ));
            }
            let input = binding.input_key();
            if let Some(existing_consume) = binding_input_policies.insert(input, binding.consume) {
                if existing_consume != binding.consume {
                    issues.push(ConfigIssue::error(
                        format!("{path}.consume"),
                        "All triggers for the same input chord and submap must use the same consume/pass-through policy",
                    ));
                }
            }
            if matches!(
                &binding.command,
                DispatchCommand::SwitchWorkspaceSelector {
                    selector: WorkspaceSelector::Name(name)
                } | DispatchCommand::MoveToWorkspaceSelector {
                    selector: WorkspaceSelector::Name(name)
                } | DispatchCommand::MoveToWorkspaceAndFollow {
                    selector: WorkspaceSelector::Name(name)
                } | DispatchCommand::RenameWorkspace {
                    selector: WorkspaceSelector::Name(name),
                    ..
                } | DispatchCommand::MoveWindowToWorkspace {
                    workspace: WorkspaceSelector::Name(name),
                    ..
                } | DispatchCommand::MoveWindowToWorkspaceAndFollow {
                    workspace: WorkspaceSelector::Name(name),
                    ..
                } | DispatchCommand::FocusMonitor {
                    target: crate::command::MonitorTarget::Name(name)
                } | DispatchCommand::MoveWorkspaceToMonitor {
                    target: crate::command::MonitorTarget::Name(name)
                } | DispatchCommand::SwapActiveWorkspaces {
                    target: crate::command::MonitorTarget::Name(name)
                } | DispatchCommand::FocusMonitorWorkspace {
                    target: crate::command::MonitorTarget::Name(name),
                    ..
                } | DispatchCommand::FocusMonitorWorkspace {
                    workspace: WorkspaceSelector::Name(name),
                    ..
                } if name.trim().is_empty()
            ) {
                issues.push(ConfigIssue::error(
                    format!("{path}.command.selector"),
                    "Named workspace and monitor selectors require a non-empty name",
                ));
            }
            let empty_argument = match &binding.command {
                DispatchCommand::AddWindowTag { tag }
                | DispatchCommand::RemoveWindowTag { tag }
                | DispatchCommand::ToggleWindowTag { tag } => tag.trim().is_empty(),
                DispatchCommand::AddSelectedWindowTag { selector, tag }
                | DispatchCommand::RemoveSelectedWindowTag { selector, tag }
                | DispatchCommand::ToggleSelectedWindowTag { selector, tag } => {
                    window_selector_is_empty(selector) || tag.trim().is_empty()
                }
                DispatchCommand::EnableRule { name }
                | DispatchCommand::DisableRule { name }
                | DispatchCommand::ToggleRule { name } => name.trim().is_empty(),
                DispatchCommand::ToggleScratchpad { id }
                | DispatchCommand::LaunchStartup { id }
                | DispatchCommand::StopStartup { id }
                | DispatchCommand::RestartStartup { id } => id.trim().is_empty(),
                DispatchCommand::Exec { program, .. } => program.trim().is_empty(),
                DispatchCommand::SetBindingSubmap { name } => name.trim().is_empty(),
                DispatchCommand::RenameWorkspace { name, .. } => name.trim().is_empty(),
                DispatchCommand::FocusWindow { selector }
                | DispatchCommand::CloseWindow { selector }
                | DispatchCommand::ManageWindow { selector }
                | DispatchCommand::UnmanageWindow { selector }
                | DispatchCommand::ToggleWindowFullscreen { selector }
                | DispatchCommand::ToggleWindowPseudotile { selector }
                | DispatchCommand::ToggleWindowGroup { selector }
                | DispatchCommand::ToggleWindowGroupLock { selector }
                | DispatchCommand::CycleWindowGroup { selector, .. }
                | DispatchCommand::FocusWindowGroupIndex { selector, .. }
                | DispatchCommand::MoveWindowInGroup { selector, .. }
                | DispatchCommand::ClearSelectedWindowTags { selector }
                | DispatchCommand::ToggleWindowPinned { selector }
                | DispatchCommand::ToggleWindowFloating { selector }
                | DispatchCommand::CenterWindow { selector }
                | DispatchCommand::RaiseWindow { selector }
                | DispatchCommand::LowerWindow { selector }
                | DispatchCommand::MoveWindow { selector, .. }
                | DispatchCommand::ResizeWindow { selector, .. }
                | DispatchCommand::SetWindowPinned { selector, .. }
                | DispatchCommand::SetWindowFloating { selector, .. }
                | DispatchCommand::MoveWindowToWorkspace {
                    window: selector, ..
                }
                | DispatchCommand::MoveWindowToWorkspaceAndFollow {
                    window: selector, ..
                } => window_selector_is_empty(selector),
                _ => false,
            };
            if empty_argument {
                issues.push(ConfigIssue::error(
                    format!("{path}.command"),
                    "Tag, named-rule, and scratchpad binding arguments cannot be empty",
                ));
            }
            if let Some(problem) = selector_pattern_problem(&binding.command) {
                issues.push(ConfigIssue::error(format!("{path}.command.selector"), problem));
            }
            if let DispatchCommand::SetBindingSubmap { name } = &binding.command {
                let target = name.trim().to_ascii_lowercase();
                if !target.is_empty() && !binding_submaps.contains(&target) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.command.name"),
                        format!("No binding submap named '{}' exists", name.trim()),
                    ));
                }
            }
            if let DispatchCommand::ToggleScratchpad { id } = &binding.command {
                let id = id.trim().to_ascii_lowercase();
                if !id.is_empty() && !scratchpad_ids.contains(&id) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.command.id"),
                        format!("No scratchpad profile named '{}' exists", id),
                    ));
                } else if !id.is_empty() && !enabled_scratchpad_ids.contains(&id) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.command.id"),
                        format!("Scratchpad profile '{}' is disabled", id),
                    ));
                }
            }
            if let DispatchCommand::LaunchStartup { id }
            | DispatchCommand::StopStartup { id }
            | DispatchCommand::RestartStartup { id } = &binding.command
            {
                let id = id.trim().to_ascii_lowercase();
                if !id.is_empty() && !startup_ids.contains(&id) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.command.id"),
                        format!("No startup application named '{}' exists", id),
                    ));
                } else if matches!(&binding.command, DispatchCommand::LaunchStartup { .. })
                    && !id.is_empty()
                    && !enabled_startup_ids.contains(&id)
                {
                    issues.push(ConfigIssue::error(
                        format!("{path}.command.id"),
                        format!("Startup application '{}' is disabled", id),
                    ));
                }
            }
        }

        if self.gestures.enabled {
            if !(50..=5_000).contains(&self.gestures.idle_timeout_ms) {
                issues.push(ConfigIssue::error(
                    "desktop.gestures.idle_timeout_ms",
                    "Gesture idle timeout must be between 50 and 5000 milliseconds",
                ));
            }
            if !(1.0..=3.0).contains(&self.gestures.axis_lock_ratio) {
                issues.push(ConfigIssue::error(
                    "desktop.gestures.axis_lock_ratio",
                    "Gesture axis lock ratio must be between 1.0 and 3.0",
                ));
            }
        }
        let mut gesture_names = HashSet::new();
        let mut gesture_chords = HashSet::new();
        for (index, gesture) in self.gestures.profiles.iter().enumerate() {
            let path = format!("desktop.gestures.profiles[{index}]");
            if gesture.name.trim().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.name"),
                    "Gesture name cannot be empty",
                ));
            } else if !gesture_names.insert(gesture.name.trim().to_ascii_lowercase()) {
                issues.push(ConfigIssue::error(
                    format!("{path}.name"),
                    format!("Duplicate gesture name '{}'", gesture.name),
                ));
            }
            if !gesture.enabled {
                continue;
            }
            if !(2..=5).contains(&gesture.fingers) {
                issues.push(ConfigIssue::error(
                    format!("{path}.fingers"),
                    "Swipe gestures require between 2 and 5 fingers",
                ));
            }
            if !(2.0..=100.0).contains(&gesture.distance_percent) {
                issues.push(ConfigIssue::error(
                    format!("{path}.distance_percent"),
                    "Gesture commit distance must be between 2 and 100 percent of the touchpad",
                ));
            }
            if !gesture_chords.insert((gesture.fingers, gesture.direction)) {
                issues.push(ConfigIssue::error(
                    path.clone(),
                    "Another enabled gesture uses the same finger count and direction",
                ));
            }
            if matches!(
                &gesture.command,
                DispatchCommand::SwitchWorkspaceSelector {
                    selector: WorkspaceSelector::Name(name)
                } | DispatchCommand::MoveToWorkspaceSelector {
                    selector: WorkspaceSelector::Name(name)
                } | DispatchCommand::MoveToWorkspaceAndFollow {
                    selector: WorkspaceSelector::Name(name)
                } | DispatchCommand::RenameWorkspace {
                    selector: WorkspaceSelector::Name(name),
                    ..
                } | DispatchCommand::MoveWindowToWorkspace {
                    workspace: WorkspaceSelector::Name(name),
                    ..
                } | DispatchCommand::MoveWindowToWorkspaceAndFollow {
                    workspace: WorkspaceSelector::Name(name),
                    ..
                } | DispatchCommand::FocusMonitor {
                    target: crate::command::MonitorTarget::Name(name)
                } | DispatchCommand::MoveWorkspaceToMonitor {
                    target: crate::command::MonitorTarget::Name(name)
                } | DispatchCommand::SwapActiveWorkspaces {
                    target: crate::command::MonitorTarget::Name(name)
                } | DispatchCommand::FocusMonitorWorkspace {
                    target: crate::command::MonitorTarget::Name(name),
                    ..
                } | DispatchCommand::FocusMonitorWorkspace {
                    workspace: WorkspaceSelector::Name(name),
                    ..
                } if name.trim().is_empty()
            ) {
                issues.push(ConfigIssue::error(
                    format!("{path}.command.selector"),
                    "Named workspace and monitor selectors require a non-empty name",
                ));
            }
            let empty_argument = match &gesture.command {
                DispatchCommand::AddWindowTag { tag }
                | DispatchCommand::RemoveWindowTag { tag }
                | DispatchCommand::ToggleWindowTag { tag } => tag.trim().is_empty(),
                DispatchCommand::AddSelectedWindowTag { selector, tag }
                | DispatchCommand::RemoveSelectedWindowTag { selector, tag }
                | DispatchCommand::ToggleSelectedWindowTag { selector, tag } => {
                    window_selector_is_empty(selector) || tag.trim().is_empty()
                }
                DispatchCommand::EnableRule { name }
                | DispatchCommand::DisableRule { name }
                | DispatchCommand::ToggleRule { name } => name.trim().is_empty(),
                DispatchCommand::ToggleScratchpad { id }
                | DispatchCommand::LaunchStartup { id }
                | DispatchCommand::StopStartup { id }
                | DispatchCommand::RestartStartup { id } => id.trim().is_empty(),
                DispatchCommand::SetBindingSubmap { name } => name.trim().is_empty(),
                DispatchCommand::RenameWorkspace { name, .. } => name.trim().is_empty(),
                DispatchCommand::FocusWindow { selector }
                | DispatchCommand::CloseWindow { selector }
                | DispatchCommand::ManageWindow { selector }
                | DispatchCommand::UnmanageWindow { selector }
                | DispatchCommand::ToggleWindowFullscreen { selector }
                | DispatchCommand::ToggleWindowPseudotile { selector }
                | DispatchCommand::ToggleWindowGroup { selector }
                | DispatchCommand::ToggleWindowGroupLock { selector }
                | DispatchCommand::CycleWindowGroup { selector, .. }
                | DispatchCommand::FocusWindowGroupIndex { selector, .. }
                | DispatchCommand::MoveWindowInGroup { selector, .. }
                | DispatchCommand::ClearSelectedWindowTags { selector }
                | DispatchCommand::ToggleWindowPinned { selector }
                | DispatchCommand::ToggleWindowFloating { selector }
                | DispatchCommand::CenterWindow { selector }
                | DispatchCommand::RaiseWindow { selector }
                | DispatchCommand::LowerWindow { selector }
                | DispatchCommand::MoveWindow { selector, .. }
                | DispatchCommand::ResizeWindow { selector, .. }
                | DispatchCommand::SetWindowPinned { selector, .. }
                | DispatchCommand::SetWindowFloating { selector, .. }
                | DispatchCommand::MoveWindowToWorkspace {
                    window: selector, ..
                }
                | DispatchCommand::MoveWindowToWorkspaceAndFollow {
                    window: selector, ..
                } => window_selector_is_empty(selector),
                _ => false,
            };
            let empty_argument = empty_argument
                || matches!(&gesture.command, DispatchCommand::Exec { program, .. } if program.trim().is_empty());
            if empty_argument {
                issues.push(ConfigIssue::error(
                    format!("{path}.command"),
                    "Gesture action arguments cannot be empty",
                ));
            }
            // What the command names is there: as for a key's command.
            match &gesture.command {
                DispatchCommand::SetBindingSubmap { name } => {
                    let target = name.trim().to_ascii_lowercase();
                    if !target.is_empty() && !binding_submaps.contains(&target) {
                        issues.push(ConfigIssue::error(
                            format!("{path}.command.name"),
                            format!("No binding submap named '{}' exists", name.trim()),
                        ));
                    }
                }
                DispatchCommand::ToggleScratchpad { id } => {
                    let id = id.trim().to_ascii_lowercase();
                    if !id.is_empty() && !scratchpad_ids.contains(&id) {
                        issues.push(ConfigIssue::error(
                            format!("{path}.command.id"),
                            format!("No scratchpad profile named '{id}' exists"),
                        ));
                    } else if !id.is_empty() && !enabled_scratchpad_ids.contains(&id) {
                        issues.push(ConfigIssue::error(
                            format!("{path}.command.id"),
                            format!("Scratchpad profile '{id}' is disabled"),
                        ));
                    }
                }
                DispatchCommand::LaunchStartup { id }
                | DispatchCommand::StopStartup { id }
                | DispatchCommand::RestartStartup { id } => {
                    let id = id.trim().to_ascii_lowercase();
                    if !id.is_empty() && !startup_ids.contains(&id) {
                        issues.push(ConfigIssue::error(
                            format!("{path}.command.id"),
                            format!("No startup application named '{id}' exists"),
                        ));
                    } else if matches!(&gesture.command, DispatchCommand::LaunchStartup { .. })
                        && !id.is_empty()
                        && !enabled_startup_ids.contains(&id)
                    {
                        issues.push(ConfigIssue::error(
                            format!("{path}.command.id"),
                            format!("Startup application '{id}' is disabled"),
                        ));
                    }
                }
                _ => {}
            }
            if let Some(problem) = selector_pattern_problem(&gesture.command) {
                issues.push(ConfigIssue::error(format!("{path}.command.selector"), problem));
            }
        }

        let mut rule_names = HashSet::new();
        for (index, rule) in self.window_rules.iter().enumerate() {
            let path = format!("desktop.window_rules[{index}]");
            if rule.name.trim().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.name"),
                    "Rule name cannot be empty",
                ));
            } else if !rule_names.insert(rule.name.trim().to_lowercase()) {
                issues.push(ConfigIssue::error(
                    format!("{path}.name"),
                    format!("Duplicate rule name '{}'", rule.name),
                ));
            }
            if !rule.enabled {
                // (Its patterns are checked all the same: a rule can be
                // switched on while gevrek runs, by a key or a command,
                // and nothing reads the file again then.)
                for (predicate_index, predicate) in rule.predicates.iter().enumerate() {
                    if matches!(predicate.strategy, MatchStrategy::Regex)
                        && regex::RegexBuilder::new(&predicate.value)
                            .case_insensitive(!predicate.case_sensitive)
                            .build()
                            .is_err()
                    {
                        issues.push(ConfigIssue::error(
                            format!("{path}.predicates[{predicate_index}].value"),
                            "Predicate contains an invalid regular expression",
                        ));
                    }
                }
                continue;
            }
            if rule.predicates.is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.predicates"),
                    "An enabled rule needs at least one predicate",
                ));
            }
            if rule.effects.is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{path}.effects"),
                    "An enabled rule needs at least one effect",
                ));
            }
            for (predicate_index, predicate) in rule.predicates.iter().enumerate() {
                if predicate.value.trim().is_empty() && !predicate.kind.is_boolean_property() {
                    issues.push(ConfigIssue::error(
                        format!("{path}.predicates[{predicate_index}].value"),
                        "Predicate value cannot be empty",
                    ));
                }
                if matches!(predicate.strategy, MatchStrategy::Regex)
                    && regex::RegexBuilder::new(&predicate.value)
                        .case_insensitive(!predicate.case_sensitive)
                        .build()
                        .is_err()
                {
                    issues.push(ConfigIssue::error(
                        format!("{path}.predicates[{predicate_index}].value"),
                        "Predicate contains an invalid regular expression",
                    ));
                }
            }
            for (effect_index, effect) in rule.effects.iter().enumerate() {
                if matches!(
                    effect,
                    RuleEffect::AddTag(tag) | RuleEffect::RemoveTag(tag) if tag.trim().is_empty()
                ) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.effects[{effect_index}]"),
                        "Tag effects require a non-empty tag",
                    ));
                }
                if matches!(effect, RuleEffect::Opacity(opacity) if *opacity < 20) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.effects[{effect_index}]"),
                        "A rule's opacity must be at least 20, as every window's (a window nobody sees cannot be found again)",
                    ));
                }
                if matches!(
                    effect,
                    RuleEffect::Resize { width, height }
                        | RuleEffect::MinSize { width, height }
                        | RuleEffect::MaxSize { width, height }
                        if width.resolve(100) <= 0 || height.resolve(100) <= 0
                ) {
                    issues.push(ConfigIssue::error(
                        format!("{path}.effects[{effect_index}]"),
                        "Resize and size-constraint dimensions must resolve to a positive size",
                    ));
                }
            }
        }
        issues
    }

    fn validate_zone_layout(layout: &ZoneLayout, path: &str, issues: &mut Vec<ConfigIssue>) {
        if layout.id.trim().is_empty() {
            issues.push(ConfigIssue::error(
                format!("{path}.id"),
                "Zone layout ID cannot be empty",
            ));
        }
        if layout.zones.is_empty() {
            issues.push(ConfigIssue::error(
                format!("{path}.zones"),
                "A zone layout requires at least one zone",
            ));
        }
        let mut ids = HashSet::new();
        for (index, zone) in layout.zones.iter().enumerate() {
            let zone_path = format!("{path}.zones[{index}]");
            if !ids.insert(zone.id) {
                issues.push(ConfigIssue::error(
                    format!("{zone_path}.id"),
                    format!("Duplicate zone ID {}", zone.id),
                ));
            }
            if zone.name.trim().is_empty() {
                issues.push(ConfigIssue::error(
                    format!("{zone_path}.name"),
                    "Zone name cannot be empty",
                ));
            }
            let valid_geometry = zone.x_percent >= 0.0
                && zone.y_percent >= 0.0
                && zone.width_percent > 0.0
                && zone.height_percent > 0.0
                && zone.x_percent + zone.width_percent <= 1.0001
                && zone.y_percent + zone.height_percent <= 1.0001;
            if !valid_geometry {
                issues.push(ConfigIssue::error(
                    format!("{zone_path}.geometry"),
                    "Zone geometry must be positive and remain inside normalized monitor bounds",
                ));
            }
            if !(0..=500).contains(&zone.gap) {
                issues.push(ConfigIssue::error(
                    format!("{zone_path}.gap"),
                    "Zone gap must be between 0 and 500 pixels",
                ));
            }
        }
    }

    pub fn has_errors(&self) -> bool {
        self.validate()
            .iter()
            .any(|issue| issue.severity == ConfigIssueSeverity::Error)
    }
}

/// Border styles the live GDI border renderer actually renders
/// (`dwm/window_manager.rs::apply_border_window_region`). Anything else used
/// to be silently rendered as a solid square, so it is rejected here instead.
fn is_supported_border_style(style: &BorderStyle) -> bool {
    matches!(
        style,
        BorderStyle::Square | BorderStyle::Rounded | BorderStyle::Double | BorderStyle::Dashed
    )
}

fn unsupported_border_style_message(style: &BorderStyle) -> String {
    format!(
        "Border style {:?} is not supported by the border renderer; supported styles: Square, Rounded, Double, Dashed",
        style
    )
}

fn validate_launch_profile(launch: &LaunchProfile, path: &str, issues: &mut Vec<ConfigIssue>) {
    if launch.program.trim().is_empty() {
        issues.push(ConfigIssue::error(
            format!("{path}.program"),
            "Launch program cannot be empty",
        ));
    }
    validate_launch_environment(launch, path, issues);
}

fn validate_launch_environment(launch: &LaunchProfile, path: &str, issues: &mut Vec<ConfigIssue>) {
    for name in launch.environment.keys() {
        if name.trim().is_empty() || name.contains('=') || name.contains('\0') {
            issues.push(ConfigIssue::error(
                format!("{path}.environment"),
                "Environment variable names cannot be empty or contain '=' or NUL",
            ));
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AppearanceProfile {
    pub border_enabled: bool,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_style: BorderStyle,
    pub active_border_color: (u8, u8, u8),
    pub inactive_border_color: (u8, u8, u8),
    pub active_border_alpha: u8,
    pub inactive_border_alpha: u8,
    /// Base alpha for every managed window before focus-relative effects.
    pub window_opacity: u8,
    /// Apply a lower effective alpha to unfocused managed windows.
    pub dim_inactive: bool,
    /// Maximum effective alpha for an inactive window when dimming is enabled.
    pub inactive_window_opacity: u8,
    /// Keep fullscreen windows fully opaque even while they are not focused.
    pub dim_fullscreen: bool,
    /// Tab strip shown above containers holding 2+ windows.
    pub stackbar: StackbarProfile,
    /// Master switch for the transparency system.
    /// When false, every managed window renders fully opaque regardless of
    /// the configured opacity values; `toggle-transparency` flips this at
    /// runtime.
    pub transparency_enabled: bool,
    /// For eyes that need it: the borders of the windows are drawn wide
    /// and in colours that stand out whatever is behind them (white
    /// around the window with the keyboard, grey around the others),
    /// whatever the settings above and the theme say; the desktop's own
    /// surfaces (the panel, the launcher, notifications, the lock screen)
    /// take their high-contrast colours, and applications are told that
    /// more contrast is wanted.
    pub high_contrast: bool,
    /// Base16 theme driving border and stackbar accent colors. Explicit
    /// per-color settings override theme-derived colors.
    pub theme: ThemeProfile,
    /// Transparent border reservation: tiled windows are shrunk by `size` on
    /// every edge as if a border were drawn, but no border is rendered
    ///.
    pub invisible_borders: InvisibleBordersProfile,
    /// Drawn effects around and over windows: shadow, glow, a gradient
    /// border, a tint. All off by default.
    pub effects: WindowEffectsProfile,
    /// The border and glow of floating windows, set apart from those of
    /// the tiled ones (the settings above).
    pub floating: FloatingAppearance,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// Effects drawn around and over the managed windows. They are drawn by the
/// window manager itself (not by the programs), so they look the same on
/// every window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(default)]
pub struct WindowEffectsProfile {
    /// Radius of the corners of the drawn border, glow and shadow, in pixels.
    pub corner_radius: CornerRadius,
    pub shadow: ShadowEffect,
    pub glow: GlowEffect,
    pub gradient: BorderGradient,
    pub tint: TintEffect,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl WindowEffectsProfile {
    /// Whether anything is drawn around the windows (the tint is drawn over
    /// them and does not count here).
    pub fn draws_around(&self) -> bool {
        self.shadow.enabled || self.glow.enabled || self.gradient.enabled
    }

    pub fn any(&self) -> bool {
        self.draws_around() || self.tint.enabled
    }

    /// How far the drawing reaches beyond the border, in pixels.
    pub fn margin(&self) -> i32 {
        let mut margin = 0;
        if self.shadow.enabled {
            // (Numbers out of a file, of any size: none of this overflows.)
            let far = |value: i32| value.checked_abs().unwrap_or(i32::MAX);
            let reach = i32::try_from(self.shadow.radius)
                .unwrap_or(i32::MAX)
                .saturating_add(far(self.shadow.offset_x).max(far(self.shadow.offset_y)));
            margin = margin.max(reach);
        }
        if self.glow.enabled {
            margin = margin.max(i32::try_from(self.glow.radius).unwrap_or(i32::MAX));
        }
        margin.clamp(0, 160)
    }
}

/// A corner radius in pixels (10 by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
pub struct CornerRadius(pub u32);

impl Default for CornerRadius {
    fn default() -> Self {
        Self(10)
    }
}

/// A soft shadow under every window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ShadowEffect {
    pub enabled: bool,
    /// How far the shadow fades out, in pixels.
    pub radius: u32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub color: (u8, u8, u8),
    /// Strength under the focused window, 0-255.
    pub opacity: u8,
    /// Strength under the other windows, 0-255.
    pub inactive_opacity: u8,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for ShadowEffect {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: false,
            radius: 28,
            offset_x: 0,
            offset_y: 8,
            color: (0, 0, 0),
            opacity: 150,
            inactive_opacity: 90,
        }
    }
}

/// A coloured light around the focused window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GlowEffect {
    pub enabled: bool,
    /// How far the light reaches, in pixels.
    pub radius: u32,
    /// Its colour; the border's colour when not given.
    pub color: Option<(u8, u8, u8)>,
    /// Strength around the focused window, 0-255.
    pub opacity: u8,
    /// Strength around the other windows, 0-255 (0: only the focused one glows).
    pub inactive_opacity: u8,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for GlowEffect {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: false,
            radius: 18,
            color: None,
            opacity: 170,
            inactive_opacity: 0,
        }
    }
}

/// The border runs from its own colour into a second one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BorderGradient {
    pub enabled: bool,
    /// The colour the focused window's border runs into.
    pub active_end: (u8, u8, u8),
    /// The colour the other windows' borders run into.
    pub inactive_end: (u8, u8, u8),
    /// Direction of the run, in degrees (0: left to right, 90: top to bottom).
    pub angle: f32,
    /// Seconds for one turn of the focused window's gradient; 0 keeps it still.
    pub spin_seconds: f32,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for BorderGradient {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: false,
            active_end: (200, 80, 255),
            inactive_end: (40, 45, 60),
            angle: 45.0,
            spin_seconds: 0.0,
        }
    }
}

/// A veil of colour over windows, to set the focused one apart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TintEffect {
    pub enabled: bool,
    pub color: (u8, u8, u8),
    /// Strength over the windows that are not focused, 0-255.
    pub inactive_opacity: u8,
    /// Strength over the focused window, 0-255 (0: none).
    pub active_opacity: u8,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for TintEffect {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: false,
            color: (12, 16, 32),
            inactive_opacity: 70,
            active_opacity: 0,
        }
    }
}

impl Default for AppearanceProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            border_enabled: true,
            border_width: 4,
            border_offset: -1,
            border_style: BorderStyle::Rounded,
            active_border_color: (0, 170, 255),
            inactive_border_color: (70, 80, 95),
            active_border_alpha: 255,
            inactive_border_alpha: 180,
            window_opacity: 255,
            dim_inactive: false,
            inactive_window_opacity: 220,
            dim_fullscreen: false,
            stackbar: StackbarProfile::default(),
            transparency_enabled: true,
            high_contrast: false,
            theme: ThemeProfile::default(),
            invisible_borders: InvisibleBordersProfile::default(),
            effects: WindowEffectsProfile::default(),
            floating: FloatingAppearance::softer_than(4, -1, &GlowEffect::default()),
        }
    }
}

/// The border and the glow of a floating window. A window that floats
/// lies over the others, and reads as lighter with a thinner border and
/// less of a glow than the tiled windows have; what is not set here (the
/// colours when they are left out, the border's style, the shadow, the
/// gradient, the corners) is as for every window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct FloatingAppearance {
    /// Floating windows have the look set here. Off: they look as the
    /// tiled windows do, and nothing else of this section is read.
    pub separate: bool,
    pub border_enabled: bool,
    pub border_width: i32,
    pub border_offset: i32,
    /// The border's colour around the focused floating window; the tiled
    /// windows' colour when not given.
    pub active_border_color: Option<(u8, u8, u8)>,
    /// The same for the other floating windows.
    pub inactive_border_color: Option<(u8, u8, u8)>,
    /// How opaque the focused floating window's border is, 0-255; as for
    /// the tiled windows when not given.
    pub active_border_alpha: Option<u8>,
    pub inactive_border_alpha: Option<u8>,
    /// How far the glow reaches around a floating window, in pixels (the
    /// glow itself is switched on and coloured under `effects.glow`).
    pub glow_radius: u32,
    /// Its strength around the focused floating window, 0-255.
    pub glow_opacity: u8,
    /// Its strength around the other floating windows, 0-255.
    pub glow_inactive_opacity: u8,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl FloatingAppearance {
    /// The look that goes with a tiled look: a border one pixel thinner
    /// (never none at all), and two thirds of the glow, in reach and in
    /// strength. The colours are the tiled windows'.
    pub fn softer_than(border_width: i32, border_offset: i32, glow: &GlowEffect) -> Self {
        let two_thirds = |value: u32| value.saturating_mul(2).div_ceil(3);
        Self {
            extra: Default::default(),
            separate: true,
            border_enabled: true,
            border_width: if border_width > 1 {
                border_width - 1
            } else {
                border_width
            },
            border_offset,
            active_border_color: None,
            inactive_border_color: None,
            active_border_alpha: None,
            inactive_border_alpha: None,
            glow_radius: two_thirds(glow.radius),
            glow_opacity: two_thirds(u32::from(glow.opacity)) as u8,
            glow_inactive_opacity: two_thirds(u32::from(glow.inactive_opacity)) as u8,
        }
    }

    /// The look that goes with the rest of an appearance (see
    /// [`Self::softer_than`]).
    pub fn softer_than_tiled(appearance: &AppearanceProfile) -> Self {
        Self::softer_than(
            appearance.border_width,
            appearance.border_offset,
            &appearance.effects.glow,
        )
    }
}

impl Default for FloatingAppearance {
    fn default() -> Self {
        Self::softer_than(4, -1, &GlowEffect::default())
    }
}

/// Base16 theme selection: a built-in palette plus the
/// accent slot used for the active border and stackbar accent. Resolution
/// lives in `crate::theme`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ThemeProfile {
    /// Built-in Base16 palette name (see `crate::theme::BASE16_PALETTES`);
    /// `None` disables theming.
    pub palette: Option<String>,
    /// Base16 accent slot (`base00`..`base0F`); defaults to `base0D` when a
    /// palette is configured. `wallpaper`: the colour that stands out in
    /// the wallpaper, with or without a palette.
    pub accent: Option<String>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// Transparent border reservation: tiled
/// windows are shrunk by `size` on every edge as if a border were drawn, but
/// no border is rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct InvisibleBordersProfile {
    pub enabled: bool,
    /// Width in pixels of the transparent band reserved around every tiled
    /// window.
    pub size: i32,
}

impl Default for InvisibleBordersProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            size: 8,
        }
    }
}

/// Tab strip rendered above a container that holds 2+ windows, showing one
/// tab per window with the focused tab highlighted.
/// Opt-in: disabled by default; `toggle-stackbar` flips
/// `enabled` at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct StackbarProfile {
    pub enabled: bool,
    /// Height of the tab strip in pixels; the container's windows get the
    /// layout rect minus this strip at the top.
    pub height: i32,
    pub background_color: (u8, u8, u8),
    /// Text (and active-tab accent) color of the focused tab.
    pub active_text_color: (u8, u8, u8),
    /// Text color of unfocused tabs.
    pub inactive_text_color: (u8, u8, u8),
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for StackbarProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: false,
            // (Lower and darker than it was until 2026-11-15, as the
            // panel is; the tab of the window shown is marked in that
            // window's border colour where it has a border, in this
            // one where it has none.)
            height: 30,
            background_color: (22, 22, 28),
            active_text_color: (64, 224, 208),
            inactive_text_color: (170, 172, 184),
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ShadowPolicy {
    /// Respect the target window's native style and Windows compositor policy.
    #[default]
    Default,
    /// Ask DWM to render the native non-client frame and its shadow.
    Enabled,
    /// Disable DWM non-client rendering, including the native shadow.
    Disabled,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AnimationCurve {
    Linear,
    EaseIn,
    EaseOut,
    #[default]
    EaseInOut,
    CubicOut,
    BackOut,
    /// A spring that comes to rest without swinging past: quick at first,
    /// then settling.
    Spring,
    /// A spring that swings a little past where it is going and back.
    SpringBouncy,
}

/// How a spring with this much damping (1: just no swing) has moved after
/// the part `t` of its time, when it is at rest at the end of it.
fn spring(t: f32, damping: f32) -> f32 {
    if t >= 1.0 {
        return 1.0;
    }
    // Stiff enough to be within a thousandth of rest when the time is up.
    let rate = 6.9 / damping;
    if damping >= 1.0 {
        // No swing: the fastest way to rest.
        return 1.0 - (1.0 + rate * t) * (-rate * t).exp();
    }
    let swing = rate * (1.0 - damping * damping).sqrt();
    let decay = (-damping * rate * t).exp();
    1.0 - decay * ((swing * t).cos() + damping * rate / swing * (swing * t).sin())
}

/// Where a spring is, measured from where it comes to rest, after the part
/// `t` of its time: it began `displacement` away, moving at `velocity`
/// (away from rest is the direction of `displacement`; per whole time).
/// With `displacement` -1 and no velocity this is `spring(t, damping) - 1`.
fn spring_motion(t: f32, damping: f32, displacement: f32, velocity: f32) -> f32 {
    if t >= 1.0 {
        return 0.0;
    }
    let t = t.max(0.0);
    let rate = 6.9 / damping;
    if damping >= 1.0 {
        return (displacement + (velocity + rate * displacement) * t) * (-rate * t).exp();
    }
    let swing = rate * (1.0 - damping * damping).sqrt();
    let decay = (-damping * rate * t).exp();
    decay
        * (displacement * (swing * t).cos()
            + (velocity + damping * rate * displacement) / swing * (swing * t).sin())
}

impl AnimationCurve {
    /// The damping of the springs (1: just no swing); `None` for the curves
    /// that are a path through time and not a spring.
    pub fn spring_damping(self) -> Option<f32> {
        match self {
            Self::Spring => Some(1.0),
            Self::SpringBouncy => Some(0.55),
            _ => None,
        }
    }

    /// For the springs: how far from its end a movement still is after the
    /// part `t` of its time, when it began `displacement` from the end and
    /// already moving at `velocity` (in the same unit, per whole time of
    /// the movement). This is what lets a movement that interrupts another
    /// go on at the speed the first one had, instead of starting from rest.
    /// `None` for the other curves: they have no speed to take over.
    pub fn spring_remaining(self, t: f32, displacement: f32, velocity: f32) -> Option<f32> {
        self.spring_damping()
            .map(|damping| spring_motion(t, damping, displacement, velocity))
    }

    pub fn sample(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(2),
            Self::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
            Self::CubicOut => 1.0 - (1.0 - t).powi(3),
            Self::BackOut => {
                let c1 = 1.70158;
                let c3 = c1 + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
            }
            Self::Spring => spring(t, 1.0),
            Self::SpringBouncy => spring(t, 0.55),
        }
    }
}

#[cfg(test)]
mod curve_tests {
    use super::AnimationCurve;

    const ALL: [AnimationCurve; 8] = [
        AnimationCurve::Linear,
        AnimationCurve::EaseIn,
        AnimationCurve::EaseOut,
        AnimationCurve::EaseInOut,
        AnimationCurve::CubicOut,
        AnimationCurve::BackOut,
        AnimationCurve::Spring,
        AnimationCurve::SpringBouncy,
    ];

    #[test]
    fn every_curve_starts_where_it_was_and_ends_where_it_goes() {
        for curve in ALL {
            assert!(
                curve.sample(0.0).abs() < 1e-4,
                "{curve:?} starts at {}",
                curve.sample(0.0)
            );
            assert!(
                (curve.sample(1.0) - 1.0).abs() < 1e-4,
                "{curve:?} ends at {}",
                curve.sample(1.0)
            );
            // Just before the end it is all but there: no jump on the last frame.
            assert!(
                (curve.sample(0.99) - 1.0).abs() < 0.03,
                "{curve:?} at 0.99 is {}",
                curve.sample(0.99)
            );
        }
    }

    #[test]
    fn a_spring_is_quick_at_first_and_only_the_bouncy_one_swings_past() {
        let steps = (0..=100).map(|step| step as f32 / 100.0);
        let highest = |curve: AnimationCurve| {
            steps
                .clone()
                .map(|t| curve.sample(t))
                .fold(0.0f32, f32::max)
        };
        assert!(
            highest(AnimationCurve::Spring) <= 1.0005,
            "the plain spring does not swing past"
        );
        let bounce = highest(AnimationCurve::SpringBouncy);
        assert!(
            bounce > 1.03 && bounce < 1.2,
            "the bouncy one swings a little past: {bounce}"
        );
        // Most of the way in the first third.
        assert!(AnimationCurve::Spring.sample(0.33) > 0.6);
        assert!(AnimationCurve::SpringBouncy.sample(0.33) > 0.8);
        // The plain spring only ever moves forward.
        let mut last = 0.0;
        for t in steps {
            let now = AnimationCurve::Spring.sample(t);
            assert!(now + 1e-5 >= last, "at {t}: {now} after {last}");
            last = now;
        }
    }

    #[test]
    fn a_spring_from_rest_is_the_curve_and_one_already_moving_gets_further() {
        for curve in [AnimationCurve::Spring, AnimationCurve::SpringBouncy] {
            for step in 0..=20 {
                let t = step as f32 / 20.0;
                // From rest, a whole way of 1: what `sample` draws.
                let remaining = curve.spring_remaining(t, -1.0, 0.0).unwrap();
                assert!(
                    (1.0 + remaining - curve.sample(t)).abs() < 1e-4,
                    "{curve:?} at {t}: {remaining}"
                );
            }
            // Already moving towards the end: nearer to it early on.
            let rest = curve.spring_remaining(0.05, -1.0, 0.0).unwrap();
            let moving = curve.spring_remaining(0.05, -1.0, 4.0).unwrap();
            assert!(moving > rest + 0.1, "{curve:?}: {moving} against {rest}");
            // Moving away from the end: further from it first, then back.
            let away = curve.spring_remaining(0.01, -1.0, -4.0).unwrap();
            assert!(away < -1.0, "{curve:?}: {away}");
            // At rest when the time is up, whatever the speed was.
            assert_eq!(curve.spring_remaining(1.0, -1.0, 9.0), Some(0.0));
            assert!(curve.spring_remaining(0.99, -1.0, 4.0).unwrap().abs() < 0.03);
        }
        // The other curves have no speed to take over.
        assert_eq!(
            AnimationCurve::EaseOut.spring_remaining(0.5, -1.0, 0.0),
            None
        );
        assert_eq!(AnimationCurve::Linear.spring_damping(), None);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AnimationChannelProfile {
    pub enabled: bool,
    pub duration_ms: u32,
    pub curve: AnimationCurve,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl AnimationChannelProfile {
    pub const fn new(enabled: bool, duration_ms: u32, curve: AnimationCurve) -> Self {
        Self {
            extra: crate::Extra::new(),
            enabled,
            duration_ms,
            curve,
        }
    }
}

impl Default for AnimationChannelProfile {
    fn default() -> Self {
        Self::new(true, 180, AnimationCurve::EaseInOut)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct AnimationProfile {
    pub enabled: bool,
    /// Nothing moves across the screen: windows and workspaces are at
    /// their new places at once, whatever the channels below say. What
    /// stays where it is and only changes (a border's colour, a window
    /// fading in or out) still does. For people whom movement on the
    /// screen makes unwell, and for anyone who finds it in the way.
    pub reduce_motion: bool,
    /// Scheduler frame rate in frames per second.
    /// The tween math is wall-clock based; this controls how often the
    /// runtime loop advances and applies animation frames.
    pub fps: u32,
    pub layout: AnimationChannelProfile,
    pub workspace: AnimationChannelProfile,
    pub border: AnimationChannelProfile,
    pub window: AnimationChannelProfile,
    pub window_enter_style: WindowTransitionStyle,
    pub window_exit_style: WindowTransitionStyle,
    /// How one workspace gives way to another: `slide` (the old one goes
    /// out sideways as the new one comes in), `slide_vertical` (upwards
    /// and downwards), `fade` (the old one fades as the new one appears).
    /// On Windows, where a window is seen wherever it is, a workspace
    /// comes from a side on which no other display lies, and the old one
    /// goes out only where its way is clear of other displays too (else
    /// it is gone at once).
    pub workspace_style: WorkspaceTransitionStyle,
    /// On Windows: when one workspace gives way to another, pictures of
    /// the windows slide and the windows themselves stay where they are
    /// (unseen until the movement is over). Their programs are then not
    /// told of a new place sixty times a second, which costs less and is
    /// smoother with programs that are slow to follow. Off: the windows
    /// themselves are moved. (A window whose picture cannot be had is
    /// moved itself either way. On Linux the compositor draws every
    /// window itself and has no use for this.)
    pub workspace_slides_as_pictures: bool,
    /// On Windows: while a workspace's windows slide, their shadow and
    /// glow are left out, and are back when the windows have arrived.
    /// They are large see-through pictures, and carrying them along at
    /// every frame is most of what a change of workspace costs the
    /// graphics card. The border itself stays.
    pub workspace_slides_without_shadows: bool,
    /// The changes of the whole picture: between the desktop and the
    /// lock screen, and between the login and the desktop.
    pub session: SessionChangeProfile,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

/// How the desktop and the lock screen, and the login and the desktop,
/// change into each other: the picture goes under a veil and the other
/// comes out from under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SessionChangeProfile {
    /// Off: the lock screen and the desktop are there at once.
    pub enabled: bool,
    /// How long the desktop takes to go under the veil (the lock screen
    /// then comes out in about as long again; between login and desktop,
    /// through black, it is a little longer).
    pub duration_ms: u32,
    /// The veil is the wallpaper blurred (off: a plain dark one).
    pub blur: bool,
    /// The picture goes dark from its edges first and clears there last.
    pub vignette: bool,
    /// Keys this version does not know: kept, and written back as they were.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for SessionChangeProfile {
    fn default() -> Self {
        Self {
            enabled: true,
            duration_ms: 480,
            blur: true,
            vignette: true,
            extra: Default::default(),
        }
    }
}

impl AnimationProfile {
    pub fn channels(&self) -> [(&'static str, &AnimationChannelProfile); 4] {
        [
            ("layout", &self.layout),
            ("workspace", &self.workspace),
            ("border", &self.border),
            ("window", &self.window),
        ]
    }

    /// Interval between scheduler ticks derived from the configured frame
    /// rate (`animation-fps`).
    pub fn frame_interval_ms(&self) -> u32 {
        (1000 / self.fps.max(1)).max(1)
    }
}

impl Default for AnimationProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: true,
            reduce_motion: false,
            fps: 60,
            layout: AnimationChannelProfile::new(true, 180, AnimationCurve::EaseInOut),
            workspace: AnimationChannelProfile::new(true, 240, AnimationCurve::CubicOut),
            border: AnimationChannelProfile::new(true, 140, AnimationCurve::EaseOut),
            window: AnimationChannelProfile::new(true, 220, AnimationCurve::BackOut),
            window_enter_style: WindowTransitionStyle::Scale,
            window_exit_style: WindowTransitionStyle::Fade,
            workspace_style: WorkspaceTransitionStyle::Slide,
            workspace_slides_as_pictures: false,
            workspace_slides_without_shadows: false,
            session: SessionChangeProfile::default(),
        }
    }
}

/// How one workspace gives way to another.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTransitionStyle {
    /// Sideways: the old one out, the new one in.
    #[default]
    Slide,
    /// Up and down.
    SlideVertical,
    /// The old one fades, the new one appears.
    Fade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowTransitionStyle {
    None,
    Fade,
    Slide,
    Scale,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum MonitorSelector {
    /// The primary display.
    #[default]
    Primary,
    /// A display by its place among the displays, from 0.
    Index(usize),
    /// A display by its name (its connector, `DP-1`, where it has one).
    Name(String),
    /// Every display. For a workspace: each display has one of its own
    /// by this name. For a scratchpad: the display that is in use.
    Any,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(default)]
pub struct ReservedArea {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ReservedArea {
    pub fn apply_to(self, rect: Rect) -> Rect {
        let left = self.left.max(0).min(rect.width.saturating_sub(1));
        let top = self.top.max(0).min(rect.height.saturating_sub(1));
        let remaining_width = (rect.width - left).max(1);
        let remaining_height = (rect.height - top).max(1);
        let right = self.right.max(0).min(remaining_width.saturating_sub(1));
        let bottom = self.bottom.max(0).min(remaining_height.saturating_sub(1));
        Rect::new(
            rect.x + left,
            rect.y + top,
            (remaining_width - right).max(1),
            (remaining_height - bottom).max(1),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MonitorProfile {
    pub enabled: bool,
    pub monitor: MonitorSelector,
    pub reserved: ReservedArea,
    /// Remap this monitor to a specific index in the monitor ring after
    /// discovery. Only honored on `Name` selectors, where
    /// the stable display name keys the preference; `None` keeps the default
    /// primary-then-geometry order.
    pub index_preference: Option<usize>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for MonitorProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: true,
            monitor: MonitorSelector::Primary,
            reserved: ReservedArea::default(),
            index_preference: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ZoneSettings {
    pub behavior: ZoneConfig,
    pub default_layout: ZoneLayout,
    pub monitor_layouts: Vec<MonitorZoneLayout>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for ZoneSettings {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            behavior: ZoneConfig::default(),
            default_layout: zone_presets::columns_2(),
            monitor_layouts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct MonitorZoneLayout {
    pub enabled: bool,
    pub monitor: MonitorSelector,
    pub layout: ZoneLayout,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for MonitorZoneLayout {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: true,
            monitor: MonitorSelector::Primary,
            layout: zone_presets::columns_2(),
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum ScratchpadPosition {
    Top,
    Bottom,
    #[default]
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
#[derive(Default)]
pub struct LaunchProfile {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: Option<String>,
    pub environment: BTreeMap<String, String>,
    /// Hide the console window for console-subsystem applications on Windows.
    pub create_no_window: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum StartupRestartPolicy {
    #[default]
    Never,
    OnFailure,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct StartupProfile {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub launch: LaunchProfile,
    pub delay_ms: u32,
    pub restart: StartupRestartPolicy,
    pub restart_delay_ms: u32,
    /// None means unlimited automated restarts.
    pub max_restarts: Option<u32>,
    /// Stop and relaunch this process whenever the configuration is applied.
    pub restart_on_reload: bool,
    /// Terminate a still-running process when it is disabled, removed, or the WM exits.
    pub terminate_on_exit: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for StartupProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            id: "terminal".to_string(),
            name: "Terminal".to_string(),
            enabled: true,
            launch: LaunchProfile {
                program: "wt.exe".to_string(),
                ..LaunchProfile::default()
            },
            delay_ms: 0,
            restart: StartupRestartPolicy::Never,
            restart_delay_ms: 1_000,
            max_restarts: Some(3),
            restart_on_reload: false,
            terminate_on_exit: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ScratchpadProfile {
    pub id: String,
    pub enabled: bool,
    pub match_kind: MatchKind,
    pub match_strategy: MatchStrategy,
    pub pattern: String,
    pub case_sensitive: bool,
    pub launch: Option<LaunchProfile>,
    /// `Any` follows the active monitor at toggle time.
    pub monitor: MonitorSelector,
    pub width_percent: u8,
    pub height_percent: u8,
    pub position: ScratchpadPosition,
    pub hide_on_focus_loss: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for ScratchpadProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            id: "terminal".to_string(),
            enabled: true,
            match_kind: MatchKind::Exe,
            match_strategy: MatchStrategy::Contains,
            pattern: "WindowsTerminal.exe".to_string(),
            case_sensitive: false,
            launch: Some(LaunchProfile {
                program: "wt.exe".to_string(),
                ..LaunchProfile::default()
            }),
            monitor: MonitorSelector::Any,
            width_percent: 78,
            height_percent: 62,
            position: ScratchpadPosition::Top,
            hide_on_focus_loss: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WorkspaceProfile {
    pub enabled: bool,
    /// What the workspace is known by to everything that names it: the
    /// keys that show it, window rules, the session that is restored. It
    /// does not change when the workspace is given another title, so
    /// nothing that names it has to follow.
    pub name: String,
    /// What is shown of it (its tab on the panel, the overview, the
    /// studio), where that is something else than its name. A workspace
    /// is renamed by giving it a title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The display it is on. `Any`: every display has a workspace of its
    /// own by this name (the keys that show it show the one of the display
    /// in use). A display can have an entry of its own for a workspace
    /// that every display has (the same `name`, this display as
    /// `monitor`): on that display the entry of its own counts, whole,
    /// and the one for every display does not. So a display can call the
    /// workspace something else, lay it out another way, or (with
    /// `enabled` off) not have it.
    pub monitor: MonitorSelector,
    pub layout: LayoutType,
    pub default: bool,
    pub persistent: bool,
    /// Structured command launched whenever this profile is materialized as an
    /// empty workspace through a workspace switch.
    pub on_created_empty: Option<LaunchProfile>,
    pub gaps_in: i32,
    pub gaps_out: i32,
    /// Per-edge offset applied to this workspace's work area after the
    /// monitor reservations and the global work-area offset.
    pub work_area_offset: ReservedArea,
    pub master_ratio: f32,
    pub master_count: usize,
    pub master_position: MasterPosition,
    pub new_window_placement: NewWindowPlacement,
    /// Whether new windows on this workspace get their own container (Column)
    /// or append to the focused container (Append).
    pub window_container_behaviour: crate::layout::WindowContainerBehaviour,
    pub dwindle_direction: DwindleDirection,
    pub dwindle_preserve_split: bool,
    pub appearance_override: bool,
    pub border_enabled: bool,
    pub border_width: i32,
    pub border_offset: i32,
    pub border_style: BorderStyle,
    pub active_border_color: (u8, u8, u8),
    pub inactive_border_color: (u8, u8, u8),
    pub active_border_alpha: u8,
    pub inactive_border_alpha: u8,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for WorkspaceProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            title: None,
            enabled: true,
            name: "Workspace".to_string(),
            // Every display has the workspace, each its own: a display
            // beside the primary one is not left with a single workspace
            // while every name the keys know is on the primary. A
            // workspace meant for one display names it.
            monitor: MonitorSelector::Any,
            layout: LayoutType::BSP,
            default: false,
            persistent: true,
            on_created_empty: None,
            gaps_in: 5,
            gaps_out: 10,
            work_area_offset: ReservedArea::default(),
            master_ratio: 0.5,
            master_count: 1,
            master_position: MasterPosition::Left,
            new_window_placement: NewWindowPlacement::StackEnd,
            window_container_behaviour: crate::layout::WindowContainerBehaviour::Column,
            dwindle_direction: DwindleDirection::Alternate,
            dwindle_preserve_split: false,
            appearance_override: true,
            border_enabled: true,
            border_width: 4,
            border_offset: -1,
            border_style: BorderStyle::Rounded,
            active_border_color: (0, 170, 255),
            inactive_border_color: (70, 80, 95),
            active_border_alpha: 255,
            inactive_border_alpha: 180,
        }
    }
}

impl WorkspaceProfile {}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BindingProfile {
    pub name: String,
    pub enabled: bool,
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Super key (the Windows key on Windows). Written as `super`;
    /// `win` is accepted when loading older files.
    #[serde(rename = "super", alias = "win")]
    pub win: bool,
    pub device: BindingDevice,
    pub trigger: BindingTrigger,
    pub submap: String,
    /// Consume the matching input instead of passing it to the focused app.
    pub consume: bool,
    pub long_press_ms: u32,
    pub command: DispatchCommand,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl BindingProfile {
    /// (By the key's one name: `RETURN` and `ENTER` are one key, and two
    /// bindings on it are two bindings on one key.)
    pub fn normalized_key(&self) -> String {
        crate::keys::canonical_key_name(&self.key)
    }

    pub fn normalized_submap(&self) -> String {
        self.submap.trim().to_ascii_lowercase()
    }

    pub fn chord_key(&self) -> String {
        format!("{:?}:{}", self.trigger, self.input_key())
    }

    pub fn input_key(&self) -> String {
        format!(
            "{:?}:{}:{}:{}:{}:{}:{}",
            self.device,
            self.normalized_submap(),
            self.ctrl,
            self.alt,
            self.shift,
            self.win,
            self.normalized_key()
        )
    }
}

impl Default for BindingProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            name: "New binding".to_string(),
            enabled: true,
            key: String::new(),
            ctrl: false,
            alt: false,
            shift: false,
            win: true,
            device: BindingDevice::Keyboard,
            trigger: BindingTrigger::Press,
            submap: "default".to_string(),
            consume: true,
            long_press_ms: 500,
            command: DispatchCommand::FocusNext,
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BindingDevice {
    #[default]
    Keyboard,
    Mouse,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BindingTrigger {
    #[default]
    Press,
    Release,
    Repeat,
    LongPress,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GestureSettings {
    pub enabled: bool,
    /// End an incomplete gesture after this many milliseconds without a frame.
    pub idle_timeout_ms: u32,
    /// Dominant-axis ratio required before direction is locked.
    pub axis_lock_ratio: f32,
    /// A sideways swipe with this many fingers moves the view along the
    /// strip of the scrolling layout, following the fingers (where the
    /// display under the pointer shows one; it comes before the profiles
    /// there). 0: no swipe does.
    pub scroll_fingers: u8,
    pub profiles: Vec<GestureProfile>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for GestureSettings {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            enabled: true,
            idle_timeout_ms: 350,
            axis_lock_ratio: 1.25,
            scroll_fingers: 3,
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GestureProfile {
    pub name: String,
    pub enabled: bool,
    pub fingers: u8,
    pub direction: GestureDirection,
    /// Required centroid travel as a percentage of the touchpad's logical range.
    pub distance_percent: f32,
    pub command: DispatchCommand,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for GestureProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            name: "New gesture".to_string(),
            enabled: true,
            fingers: 3,
            direction: GestureDirection::Left,
            distance_percent: 18.0,
            command: DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Next,
            },
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GestureDirection {
    #[default]
    Left,
    Right,
    Up,
    Down,
}

impl GestureDirection {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Up => "Up",
            Self::Down => "Down",
        }
    }

    pub fn config_name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum PredicateMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct RulePredicate {
    pub kind: MatchKind,
    pub strategy: MatchStrategy,
    pub value: String,
    pub negate: bool,
    pub case_sensitive: bool,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for RulePredicate {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            kind: MatchKind::Class,
            strategy: MatchStrategy::Regex,
            value: ".*".to_string(),
            negate: false,
            case_sensitive: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum RuleMeasure {
    /// A device-independent pixel value.
    Pixels(i32),
    /// An integer percentage of the current monitor work area.
    Percent(i32),
}

impl RuleMeasure {
    pub fn resolve(&self, reference: i32) -> i32 {
        match self {
            Self::Pixels(value) => *value,
            Self::Percent(value) => ((i64::from(reference) * i64::from(*value)) / 100) as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum RuleColor {
    Default,
    Hidden,
    Rgb { r: u8, g: u8, b: u8 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(remote = "Self")]
pub enum RuleEffect {
    /// Something this version does not know (written by a newer gevrek, or
    /// mistyped): kept as it was read and written back unchanged. It does
    /// nothing when run; `validate` reports it.
    #[serde(skip)]
    Unknown(serde_json::Value),
    Float,
    Tile,
    Ignore,
    Manage,
    MoveToWorkspace {
        monitor_index: Option<usize>,
        workspace_index: usize,
    },
    AlwaysOnTop(bool),
    /// Prevent this window from taking focus when it first appears.
    NoInitialFocus(bool),
    /// Prevent this window from receiving focus at all.
    NoFocus(bool),
    /// Exclude this window from focus-follows-mouse while retaining explicit focus.
    NoFollowMouse(bool),
    /// Keep focus on this window while it remains visible.
    StayFocused(bool),
    /// Remember and restore the floating size for matching future windows.
    PersistentSize(bool),
    Opacity(u8),
    /// Opt this window in to or out of inactive-window dimming.
    DimInactive(bool),
    /// Enable or disable the Windows compositor shadow for this window.
    Shadow(bool),
    /// Position relative to the current monitor work area's top-left corner.
    Move {
        x: RuleMeasure,
        y: RuleMeasure,
    },
    Resize {
        width: RuleMeasure,
        height: RuleMeasure,
    },
    /// Clamp a floating window to at least this size.
    MinSize {
        width: RuleMeasure,
        height: RuleMeasure,
    },
    /// Clamp a floating window to at most this size.
    MaxSize {
        width: RuleMeasure,
        height: RuleMeasure,
    },
    /// Preserve the floating window's aspect ratio during resize.
    KeepAspectRatio(bool),
    /// Center the window in its current monitor work area after resizing.
    Center,
    Maximize,
    Fullscreen,
    BorderColor(RuleColor),
    CaptionColor(RuleColor),
    Backdrop(SystemBackdrop),
    Corners(WindowCorner),
    AddTag(String),
    RemoveTag(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WindowRuleProfile {
    pub name: String,
    pub enabled: bool,
    /// Re-evaluate this rule when title, state, workspace, or tags change.
    pub dynamic: bool,
    pub mode: PredicateMode,
    pub predicates: Vec<RulePredicate>,
    pub effects: Vec<RuleEffect>,
    /// Keys this version does not know: kept, and written back as they
    /// were, so a file from a newer gevrek survives being edited here.
    #[serde(flatten)]
    pub extra: crate::Extra,
}

impl Default for WindowRuleProfile {
    fn default() -> Self {
        Self {
            extra: Default::default(),
            name: "New rule".to_string(),
            enabled: true,
            dynamic: true,
            mode: PredicateMode::All,
            predicates: vec![RulePredicate::default()],
            effects: vec![RuleEffect::Float],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ConfigIssueSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfigIssue {
    pub path: String,
    pub message: String,
    pub severity: ConfigIssueSeverity,
}

impl ConfigIssue {
    pub fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: ConfigIssueSeverity::Error,
        }
    }

    /// Something to know about that does not stop the configuration from
    /// being saved or applied.
    pub fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            severity: ConfigIssueSeverity::Warning,
        }
    }
}

/// A short, one-line form of a value this version does not know, for a
/// message.
pub(crate) fn unknown_summary(raw: &serde_json::Value) -> String {
    let text = raw.to_string();
    if text.chars().count() > 80 {
        format!("{}...", text.chars().take(77).collect::<String>())
    } else {
        text
    }
}

trait MatchKindExt {
    fn is_boolean_property(&self) -> bool;
}

impl MatchKindExt for MatchKind {
    fn is_boolean_property(&self) -> bool {
        matches!(
            self,
            MatchKind::HasParent
                | MatchKind::IsVisible
                | MatchKind::IsToolWindow
                | MatchKind::IsNoActivate
                | MatchKind::HasOwner
                | MatchKind::IsCloaked
                | MatchKind::IsLayered
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_on_screen_display_shows_the_volume_unless_it_is_said_not_to() {
        let shell = |text: &str| -> ShellConfig { serde_json::from_str(text).expect("a shell") };
        assert!(shell("{}").osd.volume);
        assert!(shell(r#"{"osd": {}}"#).osd.volume);
        assert!(!shell(r#"{"osd": {"volume": false}}"#).osd.volume);
        // A file from before the setting had its place kept it among the
        // keys nobody knew: it is read into its place, and stays there.
        let read = shell(r#"{"osd": {"volume": false, "later": 1}}"#);
        assert!(read.extra.0.is_empty());
        let written = serde_json::to_value(&read).unwrap();
        assert_eq!(written["osd"]["volume"], false);
        assert_eq!(written["osd"]["later"], 1);
    }

    #[test]
    fn a_display_has_its_own_panel_or_none() {
        let panel = PanelProfile {
            displays: vec![
                PanelOnDisplay {
                    name: "DP-3".to_string(),
                    right: Some(vec![PanelModule::Clock]),
                    ..Default::default()
                },
                PanelOnDisplay {
                    name: "hdmi-a-1".to_string(),
                    enabled: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        // Not named: the panel as it is.
        assert_eq!(
            panel.on("DP-2").map(|panel| panel.right),
            Some(panel.right.clone())
        );
        // Named: what is set for it, the rest as on the others.
        let third = panel.on("dp-3").expect("a panel");
        assert_eq!(third.right, vec![PanelModule::Clock]);
        assert_eq!(third.left, panel.left);
        // Switched off for one display, and for all.
        assert_eq!(panel.on("HDMI-A-1"), None);
        let off = PanelProfile {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(off.on("DP-2"), None);
    }

    #[test]
    fn defaults_are_valid_and_backward_safe() {
        assert!(DesktopConfig::default().validate().is_empty());
    }

    #[test]
    fn unsupported_border_styles_fail_validation_loudly() {
        for style in [
            BorderStyle::Glow,
            BorderStyle::Shadow,
            BorderStyle::Gradient,
            BorderStyle::Native,
        ] {
            let mut config = DesktopConfig::default();
            config.appearance.border_style = style;
            let issue = config
                .validate()
                .into_iter()
                .find(|issue| issue.path == "desktop.appearance.border_style")
                .unwrap_or_else(|| panic!("{style:?} should fail validation"));
            assert!(
                issue.message.contains("Square, Rounded, Double, Dashed"),
                "error should name the supported styles: {}",
                issue.message
            );
        }
    }

    #[test]
    fn supported_border_styles_pass_validation() {
        for style in [
            BorderStyle::Square,
            BorderStyle::Rounded,
            BorderStyle::Double,
            BorderStyle::Dashed,
        ] {
            let mut config = DesktopConfig::default();
            config.appearance.border_style = style;
            assert!(config.validate().is_empty(), "{style:?} should be valid");
        }
    }

    #[test]
    fn workspace_border_style_validated_only_when_overriding_appearance() {
        let mut config = DesktopConfig::default();
        config.workspaces.push(WorkspaceProfile {
            border_style: BorderStyle::Gradient,
            appearance_override: true,
            ..WorkspaceProfile::default()
        });
        assert!(config
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.workspaces[0].border_style"));

        // Without an appearance override the workspace value is inert; the
        // global appearance style applies instead.
        config.workspaces[0].appearance_override = false;
        assert!(config.validate().is_empty());
    }

    #[test]
    fn targeted_focus_bindings_require_a_usable_selector() {
        let mut config = DesktopConfig {
            bindings: vec![BindingProfile {
                key: "F".to_string(),
                command: DispatchCommand::FocusWindow {
                    selector: WindowSelector::Tag(String::new()),
                },
                ..BindingProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(config
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.bindings[0].command"));

        config.bindings[0].command = DispatchCommand::FocusWindow {
            selector: WindowSelector::Tag("communication".to_string()),
        };
        assert!(config.validate().is_empty());
    }

    #[test]
    fn detects_binding_conflicts_and_invalid_rules() {
        let binding = BindingProfile {
            key: "K".to_string(),
            ..BindingProfile::default()
        };
        let mut config = DesktopConfig {
            bindings: vec![binding.clone(), binding],
            window_rules: vec![WindowRuleProfile {
                predicates: Vec::new(),
                effects: Vec::new(),
                ..WindowRuleProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert_eq!(
            config
                .validate()
                .iter()
                .filter(|issue| issue.severity == ConfigIssueSeverity::Error)
                .count(),
            3
        );

        config.bindings[1].key = "J".to_string();
        config.window_rules[0].enabled = false;
        assert!(config.validate().is_empty());
    }

    #[test]
    fn advanced_bindings_round_trip_and_validate_by_trigger_and_submap() {
        let enter = BindingProfile {
            key: "R".to_string(),
            command: DispatchCommand::SetBindingSubmap {
                name: "resize".to_string(),
            },
            ..BindingProfile::default()
        };
        let press = BindingProfile {
            name: "Move immediately".to_string(),
            key: "H".to_string(),
            win: false,
            submap: "resize".to_string(),
            trigger: BindingTrigger::Press,
            command: DispatchCommand::MoveActive {
                delta_x: -20,
                delta_y: 0,
            },
            ..BindingProfile::default()
        };
        let repeat = BindingProfile {
            name: "Keep moving".to_string(),
            trigger: BindingTrigger::Repeat,
            ..press.clone()
        };
        let release_mouse = BindingProfile {
            name: "Release mouse button".to_string(),
            key: "X1".to_string(),
            win: false,
            device: BindingDevice::Mouse,
            trigger: BindingTrigger::Release,
            consume: false,
            command: DispatchCommand::ToggleExpose,
            ..BindingProfile::default()
        };
        let config = DesktopConfig {
            bindings: vec![enter, press, repeat, release_mouse],
            ..DesktopConfig::default()
        };
        assert!(config.validate().is_empty(), "{:?}", config.validate());

        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bindings[2].trigger, BindingTrigger::Repeat);
        assert_eq!(decoded.bindings[3].device, BindingDevice::Mouse);
        assert!(!decoded.bindings[3].consume);

        let invalid_wheel = DesktopConfig {
            bindings: vec![BindingProfile {
                key: "WHEEL_UP".to_string(),
                device: BindingDevice::Mouse,
                trigger: BindingTrigger::Release,
                ..BindingProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(invalid_wheel.validate().iter().any(|issue| {
            issue.path == "desktop.bindings[0].trigger"
                && issue.severity == ConfigIssueSeverity::Error
        }));
    }

    #[test]
    fn rejects_empty_named_workspace_selectors() {
        let config = DesktopConfig {
            bindings: vec![BindingProfile {
                key: "1".to_string(),
                command: DispatchCommand::SwitchWorkspaceSelector {
                    selector: WorkspaceSelector::Name(" ".to_string()),
                },
                ..BindingProfile::default()
            }],
            ..DesktopConfig::default()
        };

        assert!(config.validate().iter().any(|issue| {
            issue.path == "desktop.bindings[0].command.selector"
                && issue.severity == ConfigIssueSeverity::Error
        }));
    }

    #[test]
    fn older_configuration_gets_current_profile_defaults() {
        let config: DesktopConfig = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.appearance.window_opacity, 255);
        assert!(!config.appearance.dim_inactive);
        assert_eq!(config.appearance.inactive_window_opacity, 220);
        assert_eq!(config.platform.windows.shadow, ShadowPolicy::Default);
        assert_eq!(config.animations.layout.duration_ms, 180);
        assert_eq!(config.animations.workspace.duration_ms, 240);
        assert_eq!(config.animations.border.duration_ms, 140);
        assert_eq!(config.animations.window.duration_ms, 220);
        assert!(config.validate().is_empty());
    }

    #[test]
    fn animation_fps_defaults_to_60_and_drives_the_frame_interval() {
        let config = DesktopConfig::default();
        assert_eq!(config.animations.fps, 60);
        assert_eq!(config.animations.frame_interval_ms(), 16);
        assert!(config.validate().is_empty());

        let fast = AnimationProfile {
            fps: 144,
            ..AnimationProfile::default()
        };
        assert_eq!(fast.frame_interval_ms(), 6);

        for fps in [29, 241] {
            let config = DesktopConfig {
                animations: AnimationProfile {
                    fps,
                    ..AnimationProfile::default()
                },
                ..DesktopConfig::default()
            };
            assert!(config
                .validate()
                .iter()
                .any(|issue| issue.path == "desktop.animations.fps"));
        }
        for fps in [30, 240] {
            let config = DesktopConfig {
                animations: AnimationProfile {
                    fps,
                    ..AnimationProfile::default()
                },
                ..DesktopConfig::default()
            };
            assert!(config.validate().is_empty());
        }
    }

    #[test]
    fn theme_validation_rejects_unknown_palette_and_accent() {
        let config = DesktopConfig::default();
        assert!(config.appearance.theme.palette.is_none());
        assert!(config.validate().is_empty());

        let unknown_palette = DesktopConfig {
            appearance: AppearanceProfile {
                theme: ThemeProfile {
                    extra: Default::default(),
                    palette: Some("dracula".to_string()),
                    accent: None,
                },
                ..AppearanceProfile::default()
            },
            ..DesktopConfig::default()
        };
        assert!(unknown_palette
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.appearance.theme.palette"));

        let unknown_accent = DesktopConfig {
            appearance: AppearanceProfile {
                theme: ThemeProfile {
                    extra: Default::default(),
                    palette: Some("Ocean".to_string()),
                    accent: Some("red".to_string()),
                },
                ..AppearanceProfile::default()
            },
            ..DesktopConfig::default()
        };
        assert!(unknown_accent
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.appearance.theme.accent"));

        let valid = DesktopConfig {
            appearance: AppearanceProfile {
                theme: ThemeProfile {
                    extra: Default::default(),
                    palette: Some("gruvbox-dark".to_string()),
                    accent: Some("base0b".to_string()),
                },
                ..AppearanceProfile::default()
            },
            ..DesktopConfig::default()
        };
        assert!(valid.validate().is_empty());
        let json = serde_json::to_string(&valid).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.appearance.theme.palette.as_deref(),
            Some("gruvbox-dark")
        );
        assert_eq!(decoded.appearance.theme.accent.as_deref(), Some("base0b"));
    }

    #[test]
    fn invisible_borders_size_validated_only_when_enabled() {
        let config = DesktopConfig::default();
        assert!(!config.appearance.invisible_borders.enabled);
        assert!(config.validate().is_empty());

        let invalid = DesktopConfig {
            appearance: AppearanceProfile {
                invisible_borders: InvisibleBordersProfile {
                    enabled: true,
                    size: 0,
                },
                ..AppearanceProfile::default()
            },
            ..DesktopConfig::default()
        };
        assert!(invalid
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.appearance.invisible_borders.size"));

        let valid = DesktopConfig {
            appearance: AppearanceProfile {
                invisible_borders: InvisibleBordersProfile {
                    enabled: true,
                    size: 12,
                },
                ..AppearanceProfile::default()
            },
            ..DesktopConfig::default()
        };
        assert!(valid.validate().is_empty());
        let json = serde_json::to_string(&valid).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert!(decoded.appearance.invisible_borders.enabled);
        assert_eq!(decoded.appearance.invisible_borders.size, 12);
    }

    #[test]
    fn monitor_index_preference_requires_a_named_selector_and_round_trips() {
        let valid = DesktopConfig {
            monitors: vec![MonitorProfile {
                monitor: MonitorSelector::Name("DELL U2723QE".to_string()),
                index_preference: Some(1),
                ..MonitorProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(valid.validate().is_empty());
        let json = serde_json::to_string(&valid).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.monitors[0].index_preference, Some(1));

        // Old configs without the field still parse.
        let legacy: MonitorProfile =
            serde_json::from_str(r#"{"monitor": {"Name": "DELL U2723QE"}}"#).unwrap();
        assert_eq!(legacy.index_preference, None);

        for selector in [MonitorSelector::Primary, MonitorSelector::Index(0)] {
            let invalid = DesktopConfig {
                monitors: vec![MonitorProfile {
                    monitor: selector,
                    index_preference: Some(0),
                    ..MonitorProfile::default()
                }],
                ..DesktopConfig::default()
            };
            assert!(invalid
                .validate()
                .iter()
                .any(|issue| issue.path == "desktop.monitors[0].index_preference"));
        }
    }

    #[test]
    fn workspace_profile_container_behaviour_defaults_to_column_and_round_trips() {
        let profile = WorkspaceProfile::default();
        assert_eq!(
            profile.window_container_behaviour,
            crate::layout::WindowContainerBehaviour::Column
        );

        let append: WorkspaceProfile =
            serde_json::from_str(r#"{"window_container_behaviour": "append"}"#).unwrap();
        assert_eq!(
            append.window_container_behaviour,
            crate::layout::WindowContainerBehaviour::Append
        );
        let json = serde_json::to_string(&append).unwrap();
        assert!(json.contains(r#""window_container_behaviour":"append""#));
    }

    #[test]
    fn work_area_offsets_validate_and_round_trip() {
        let config = DesktopConfig::default();
        assert_eq!(config.work_area_offset, ReservedArea::default());
        assert!(config.validate().is_empty());

        let invalid_global = DesktopConfig {
            work_area_offset: ReservedArea {
                left: -5,
                ..ReservedArea::default()
            },
            ..DesktopConfig::default()
        };
        assert!(invalid_global
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.work_area_offset.left"));

        let invalid_workspace = DesktopConfig {
            workspaces: vec![WorkspaceProfile {
                name: "code".to_string(),
                work_area_offset: ReservedArea {
                    bottom: 20_000,
                    ..ReservedArea::default()
                },
                ..WorkspaceProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(invalid_workspace
            .validate()
            .iter()
            .any(|issue| issue.path == "desktop.workspaces[0].work_area_offset.bottom"));

        let valid = DesktopConfig {
            work_area_offset: ReservedArea {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            },
            workspaces: vec![WorkspaceProfile {
                name: "code".to_string(),
                work_area_offset: ReservedArea {
                    left: 1,
                    top: 2,
                    right: 3,
                    bottom: 4,
                },
                ..WorkspaceProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(valid.validate().is_empty());
        let json = serde_json::to_string(&valid).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.work_area_offset.left, 10);
        assert_eq!(decoded.work_area_offset.bottom, 40);
        assert_eq!(decoded.workspaces[0].work_area_offset.right, 3);
    }

    #[test]
    fn stackbar_defaults_off_and_round_trips() {
        let config: DesktopConfig = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert!(!config.appearance.stackbar.enabled);
        assert_eq!(config.appearance.stackbar.height, 30);
        assert!(config.validate().is_empty());

        let config = DesktopConfig {
            appearance: AppearanceProfile {
                stackbar: StackbarProfile {
                    extra: Default::default(),
                    enabled: true,
                    height: 32,
                    background_color: (10, 20, 30),
                    active_text_color: (200, 210, 220),
                    inactive_text_color: (90, 95, 100),
                },
                ..AppearanceProfile::default()
            },
            ..DesktopConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert!(decoded.validate().is_empty());
        assert!(decoded.appearance.stackbar.enabled);
        assert_eq!(decoded.appearance.stackbar.height, 32);
        assert_eq!(decoded.appearance.stackbar.background_color, (10, 20, 30));
        assert_eq!(
            decoded.appearance.stackbar.active_text_color,
            (200, 210, 220)
        );
        assert_eq!(
            decoded.appearance.stackbar.inactive_text_color,
            (90, 95, 100)
        );
    }

    #[test]
    fn stackbar_height_is_validated() {
        for height in [0, 9, 201, -5] {
            let config = DesktopConfig {
                appearance: AppearanceProfile {
                    stackbar: StackbarProfile {
                        height,
                        ..StackbarProfile::default()
                    },
                    ..AppearanceProfile::default()
                },
                ..DesktopConfig::default()
            };
            assert!(
                config.validate().iter().any(|issue| {
                    issue.path == "desktop.appearance.stackbar.height"
                        && issue.severity == ConfigIssueSeverity::Error
                }),
                "height {height} should fail validation"
            );
        }
        for height in [10, 40, 200] {
            let config = DesktopConfig {
                appearance: AppearanceProfile {
                    stackbar: StackbarProfile {
                        height,
                        ..StackbarProfile::default()
                    },
                    ..AppearanceProfile::default()
                },
                ..DesktopConfig::default()
            };
            assert!(
                config.validate().is_empty(),
                "height {height} should pass validation"
            );
        }
    }

    #[test]
    fn focus_effects_and_rule_overrides_round_trip() {
        let config = DesktopConfig {
            appearance: AppearanceProfile {
                dim_inactive: true,
                inactive_window_opacity: 176,
                dim_fullscreen: true,
                ..AppearanceProfile::default()
            },
            platform: crate::platform::PlatformConfig {
                windows: crate::platform::windows::WindowsPlatformConfig {
                    shadow: ShadowPolicy::Enabled,
                    ..Default::default()
                },
                ..Default::default()
            },
            window_rules: vec![WindowRuleProfile {
                effects: vec![
                    RuleEffect::Opacity(210),
                    RuleEffect::DimInactive(false),
                    RuleEffect::Shadow(false),
                ],
                ..WindowRuleProfile::default()
            }],
            ..DesktopConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();

        assert!(decoded.validate().is_empty());
        assert!(decoded.appearance.dim_inactive);
        assert_eq!(decoded.appearance.inactive_window_opacity, 176);
        assert_eq!(decoded.platform.windows.shadow, ShadowPolicy::Enabled);
        assert_eq!(
            decoded.window_rules[0].effects,
            vec![
                RuleEffect::Opacity(210),
                RuleEffect::DimInactive(false),
                RuleEffect::Shadow(false)
            ]
        );
    }

    #[test]
    fn startup_profiles_round_trip_and_bindings_reference_valid_ids() {
        let config = DesktopConfig {
            startup: vec![StartupProfile {
                id: "status-bar".to_string(),
                name: "Status bar".to_string(),
                launch: LaunchProfile {
                    program: "status-bar.exe".to_string(),
                    args: vec!["--compact".to_string()],
                    environment: BTreeMap::from([(
                        "GEVREK_MONITOR".to_string(),
                        "primary".to_string(),
                    )]),
                    create_no_window: true,
                    ..LaunchProfile::default()
                },
                delay_ms: 500,
                restart: StartupRestartPolicy::OnFailure,
                restart_delay_ms: 2_000,
                max_restarts: Some(5),
                restart_on_reload: true,
                terminate_on_exit: true,
                ..StartupProfile::default()
            }],
            bindings: vec![BindingProfile {
                key: "R".to_string(),
                command: DispatchCommand::RestartStartup {
                    id: "status-bar".to_string(),
                },
                ..BindingProfile::default()
            }],
            ..DesktopConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();

        assert!(decoded.validate().is_empty(), "{:?}", decoded.validate());
        assert_eq!(decoded.startup[0].restart, StartupRestartPolicy::OnFailure);
        assert_eq!(decoded.startup[0].max_restarts, Some(5));
        assert_eq!(
            decoded.startup[0].launch.environment["GEVREK_MONITOR"],
            "primary"
        );
    }

    #[test]
    fn dwindle_workspace_profile_round_trips() {
        let profile = WorkspaceProfile {
            layout: LayoutType::Dwindle,
            dwindle_direction: DwindleDirection::Vertical,
            dwindle_preserve_split: true,
            ..WorkspaceProfile::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: WorkspaceProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.layout, LayoutType::Dwindle);
        assert_eq!(decoded.dwindle_direction, DwindleDirection::Vertical);
        assert!(decoded.dwindle_preserve_split);
    }

    #[test]
    fn workspace_lifecycle_hook_round_trips_and_is_validated() {
        let mut config = DesktopConfig {
            workspaces: vec![WorkspaceProfile {
                name: "Ephemeral notes".to_string(),
                persistent: false,
                on_created_empty: Some(LaunchProfile {
                    program: "notepad.exe".to_string(),
                    args: vec!["notes.txt".to_string()],
                    environment: BTreeMap::from([(
                        "GEVREK_WORKSPACE".to_string(),
                        "Ephemeral notes".to_string(),
                    )]),
                    ..LaunchProfile::default()
                }),
                ..WorkspaceProfile::default()
            }],
            ..DesktopConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        let launch = decoded.workspaces[0].on_created_empty.as_ref().unwrap();
        assert!(!decoded.workspaces[0].persistent);
        assert_eq!(launch.program, "notepad.exe");
        assert_eq!(launch.environment["GEVREK_WORKSPACE"], "Ephemeral notes");
        assert!(decoded.validate().is_empty());

        config.workspaces[0]
            .on_created_empty
            .as_mut()
            .unwrap()
            .program
            .clear();
        assert!(config
            .validate()
            .iter()
            .any(|issue| { issue.path == "desktop.workspaces[0].on_created_empty.program" }));
    }

    #[test]
    fn monitor_reservations_and_master_policy_round_trip() {
        let config = DesktopConfig {
            monitors: vec![MonitorProfile {
                monitor: MonitorSelector::Name("Studio Display".to_string()),
                reserved: ReservedArea {
                    left: 24,
                    top: 40,
                    right: 280,
                    bottom: 12,
                },
                ..MonitorProfile::default()
            }],
            workspaces: vec![WorkspaceProfile {
                layout: LayoutType::MasterStack,
                master_position: MasterPosition::Bottom,
                new_window_placement: NewWindowPlacement::Master,
                ..WorkspaceProfile::default()
            }],
            ..DesktopConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();

        assert!(decoded.validate().is_empty());
        assert_eq!(decoded.monitors[0].reserved.right, 280);
        assert_eq!(
            decoded.workspaces[0].master_position,
            MasterPosition::Bottom
        );
        assert_eq!(
            decoded.workspaces[0].new_window_placement,
            NewWindowPlacement::Master
        );
        assert_eq!(
            decoded.monitors[0]
                .reserved
                .apply_to(Rect::new(100, 200, 1920, 1080)),
            Rect::new(124, 240, 1616, 1028)
        );
    }

    #[test]
    fn animation_curves_preserve_endpoints() {
        for curve in [
            AnimationCurve::Linear,
            AnimationCurve::EaseIn,
            AnimationCurve::EaseOut,
            AnimationCurve::EaseInOut,
            AnimationCurve::CubicOut,
            AnimationCurve::BackOut,
        ] {
            assert!((curve.sample(0.0) - 0.0).abs() < f32::EPSILON);
            assert!((curve.sample(1.0) - 1.0).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn animation_channels_round_trip_and_validate_independently() {
        let mut config = DesktopConfig::default();
        config.animations.layout = AnimationChannelProfile::new(true, 90, AnimationCurve::Linear);
        config.animations.workspace =
            AnimationChannelProfile::new(false, 420, AnimationCurve::CubicOut);
        config.animations.border = AnimationChannelProfile::new(true, 75, AnimationCurve::EaseOut);
        config.animations.window = AnimationChannelProfile::new(true, 310, AnimationCurve::BackOut);
        config.animations.window_enter_style = WindowTransitionStyle::Slide;
        config.animations.window_exit_style = WindowTransitionStyle::Scale;

        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.animations, config.animations);
        assert_eq!(
            decoded.animations.window_enter_style,
            WindowTransitionStyle::Slide
        );
        assert!(decoded.validate().is_empty());

        config.animations.workspace.duration_ms = 5;
        assert!(config.validate().iter().any(|issue| {
            issue.path == "desktop.animations.workspace.duration_ms"
                && issue.severity == ConfigIssueSeverity::Error
        }));
    }

    #[test]
    fn geometry_rule_measures_round_trip_and_invalid_sizes_are_rejected() {
        let effect = RuleEffect::Move {
            x: RuleMeasure::Percent(75),
            y: RuleMeasure::Pixels(-20),
        };
        let json = serde_json::to_string(&effect).unwrap();
        assert_eq!(serde_json::from_str::<RuleEffect>(&json).unwrap(), effect);
        assert_eq!(RuleMeasure::Percent(25).resolve(1920), 480);
        assert_eq!(RuleMeasure::Pixels(-20).resolve(1920), -20);

        let config = DesktopConfig {
            window_rules: vec![WindowRuleProfile {
                effects: vec![RuleEffect::Resize {
                    width: RuleMeasure::Percent(0),
                    height: RuleMeasure::Pixels(400),
                }],
                ..WindowRuleProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(config.validate().iter().any(|issue| {
            issue.path == "desktop.window_rules[0].effects[0]"
                && issue.severity == ConfigIssueSeverity::Error
        }));
    }

    #[test]
    fn scratchpad_profiles_round_trip_with_structured_launches() {
        let profile = ScratchpadProfile {
            launch: Some(LaunchProfile {
                extra: Default::default(),
                program: "pwsh.exe".to_string(),
                args: vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
                working_directory: Some(r"C:\dev".to_string()),
                environment: BTreeMap::from([("GEVREK_TEST".to_string(), "1".to_string())]),
                create_no_window: true,
            }),
            monitor: MonitorSelector::Name("DELL U2723QE".to_string()),
            position: ScratchpadPosition::BottomRight,
            hide_on_focus_loss: true,
            ..ScratchpadProfile::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: ScratchpadProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.launch.as_ref().unwrap().args,
            ["-NoLogo", "-NoProfile"]
        );
        assert_eq!(
            decoded.launch.as_ref().unwrap().environment["GEVREK_TEST"],
            "1"
        );
        assert!(decoded.launch.as_ref().unwrap().create_no_window);
        assert_eq!(decoded.monitor, profile.monitor);
        assert_eq!(decoded.position, ScratchpadPosition::BottomRight);
        assert!(decoded.hide_on_focus_loss);
    }

    #[test]
    fn scratchpad_bindings_require_a_valid_profile() {
        let config = DesktopConfig {
            bindings: vec![BindingProfile {
                key: "Oem3".to_string(),
                command: DispatchCommand::ToggleScratchpad {
                    id: "terminal".to_string(),
                },
                ..BindingProfile::default()
            }],
            ..DesktopConfig::default()
        };
        assert!(config.validate().iter().any(|issue| {
            issue.path == "desktop.bindings[0].command.id"
                && issue.severity == ConfigIssueSeverity::Error
        }));

        let valid = DesktopConfig {
            scratchpads: vec![ScratchpadProfile::default()],
            ..config
        };
        assert!(valid.validate().is_empty());
    }

    #[test]
    fn zone_settings_validate_geometry_and_round_trip_monitor_overrides() {
        let settings = ZoneSettings {
            default_layout: zone_presets::priority_grid(),
            monitor_layouts: vec![MonitorZoneLayout {
                monitor: MonitorSelector::Name("Studio Display".to_string()),
                layout: zone_presets::ultrawide(),
                ..MonitorZoneLayout::default()
            }],
            ..ZoneSettings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let decoded: ZoneSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.default_layout.id, "priority_grid");
        assert_eq!(decoded.monitor_layouts[0].layout.id, "ultrawide");

        let mut config = DesktopConfig {
            zones: decoded,
            ..DesktopConfig::default()
        };
        assert!(config.validate().is_empty());
        config.zones.default_layout.zones[0].width_percent = 1.5;
        assert!(config
            .validate()
            .iter()
            .any(|issue| issue.path.ends_with(".geometry")));
    }

    #[test]
    fn gesture_profiles_round_trip_and_reject_ambiguous_swipes() {
        let profile = GestureProfile {
            name: "Workspace left".to_string(),
            fingers: 3,
            direction: GestureDirection::Left,
            distance_percent: 16.0,
            command: DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Next,
            },
            ..GestureProfile::default()
        };
        let config = DesktopConfig {
            gestures: GestureSettings {
                idle_timeout_ms: 420,
                axis_lock_ratio: 1.4,
                profiles: vec![profile.clone()],
                ..GestureSettings::default()
            },
            ..DesktopConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: DesktopConfig = serde_json::from_str(&json).unwrap();
        assert!(decoded.validate().is_empty(), "{:?}", decoded.validate());
        assert_eq!(decoded.gestures.profiles[0].fingers, 3);
        assert_eq!(
            decoded.gestures.profiles[0].direction,
            GestureDirection::Left
        );
        assert_eq!(decoded.gestures.idle_timeout_ms, 420);

        let ambiguous = DesktopConfig {
            gestures: GestureSettings {
                profiles: vec![
                    profile.clone(),
                    GestureProfile {
                        name: "Duplicate direction".to_string(),
                        ..profile
                    },
                ],
                ..GestureSettings::default()
            },
            ..DesktopConfig::default()
        };
        assert!(ambiguous.validate().iter().any(|issue| {
            issue.path == "desktop.gestures.profiles[1]"
                && issue.severity == ConfigIssueSeverity::Error
        }));
    }
}

crate::tolerant_enum!(RuleEffect);

#[cfg(test)]
mod workspace_rename_tests {
    use crate::command::{DispatchCommand, WorkspaceSelector};

    #[test]
    fn a_renamed_workspace_keeps_its_entry_and_its_keys() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        let shows = |desktop: &super::DesktopConfig, wanted: &str| {
            desktop.bindings.iter().any(|binding| {
                matches!(&binding.command, DispatchCommand::SwitchWorkspaceSelector {
                    selector: WorkspaceSelector::Name(name),
                } if name == wanted)
            })
        };
        assert!(shows(&desktop, "1"), "the starting keys show workspace 1 by its name");

        assert!(desktop.rename_workspace("1", " web "));
        assert!(desktop.workspaces.iter().any(|profile| profile.name == "web"));
        assert!(!desktop.workspaces.iter().any(|profile| profile.name == "1"));
        // The key that showed it shows it still.
        assert!(shows(&desktop, "web") && !shows(&desktop, "1"));
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());

        // No workspace by that name, a name that is taken, no name at all.
        assert!(!desktop.rename_workspace("1", "mail"));
        assert!(!desktop.rename_workspace("web", "2"));
        assert!(!desktop.rename_workspace("web", "  "));
        // Only its letters' case is another: still the same workspace.
        assert!(desktop.rename_workspace("web", "Web"));
    }
}

#[cfg(test)]
mod odd_value_tests {
    use super::*;

    fn problems(desktop: &DesktopConfig) -> Vec<String> {
        desktop.validate().into_iter().map(|issue| issue.path).collect()
    }

    #[test]
    fn a_key_by_its_other_name_is_the_same_key() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        assert!(problems(&desktop).is_empty(), "{:?}", desktop.validate());
        let mut first = BindingProfile {
            name: "one".to_string(),
            key: "ENTER".to_string(),
            win: true,
            ctrl: true,
            alt: true,
            ..BindingProfile::default()
        };
        first.enabled = true;
        let second = BindingProfile {
            name: "two".to_string(),
            key: "return".to_string(),
            ..first.clone()
        };
        assert_eq!(first.chord_key(), second.chord_key());
        desktop.bindings.push(first);
        desktop.bindings.push(second);
        assert!(!problems(&desktop).is_empty(), "two bindings on one key are told of");
        assert_eq!(crate::keys::canonical_key_name(" esc "), "ESCAPE");
        assert_eq!(crate::keys::canonical_key_name("oem_minus"), "MINUS");
        assert_eq!(crate::keys::canonical_key_name("X1"), "X1");
    }

    #[test]
    fn values_no_desktop_can_have_are_told_of() {
        let good = crate::examples::starting_configuration().desktop;
        let told = |change: &dyn Fn(&mut DesktopConfig), place: &str| {
            let mut desktop = good.clone();
            change(&mut desktop);
            assert!(
                problems(&desktop).iter().any(|path| path.contains(place)),
                "{place}: {:?}",
                problems(&desktop)
            );
        };
        told(&|desktop| desktop.workspaces[0].gaps_in = -4, "gaps_in");
        told(&|desktop| desktop.workspaces[0].gaps_out = 100_000, "gaps_out");
        told(&|desktop| desktop.appearance.effects.shadow.radius = u32::MAX, "shadow.radius");
        told(&|desktop| desktop.appearance.effects.shadow.offset_x = i32::MIN, "shadow.offset_x");
        told(&|desktop| desktop.appearance.effects.glow.radius = 5000, "glow.radius");
        told(&|desktop| desktop.animations.session.duration_ms = 600_000, "session.duration_ms");
        // (And none of them overflows where it is used.)
        let mut desktop = good.clone();
        desktop.appearance.effects.shadow.enabled = true;
        desktop.appearance.effects.shadow.radius = u32::MAX;
        desktop.appearance.effects.shadow.offset_x = i32::MIN;
        assert_eq!(desktop.appearance.effects.margin(), 160);
    }

    #[test]
    fn a_pattern_that_cannot_be_read_is_told_of_wherever_it_is_written() {
        let command: DispatchCommand = serde_json::from_value(serde_json::json!({
            "command": "focus_window",
            "selector": {"kind": "title", "value": "re:(unclosed"}
        }))
        .expect("a command");
        assert!(selector_pattern_problem(&command).is_some());
        let command: DispatchCommand = serde_json::from_value(serde_json::json!({
            "command": "focus_window",
            "selector": {"kind": "title", "value": "re:^Mail"}
        }))
        .expect("a command");
        assert!(selector_pattern_problem(&command).is_none());
    }
}

#[cfg(test)]
mod workspace_per_display_tests {
    use super::*;
    use crate::command::{DispatchCommand, WorkspaceSelector};

    fn labels(desktop: &DesktopConfig, display: &str) -> Vec<String> {
        desktop
            .workspaces_on(&DisplayRef::named(display))
            .iter()
            .map(|profile| profile.label().to_string())
            .collect()
    }

    #[test]
    fn a_workspace_titled_on_one_display_keeps_its_name_everywhere() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        let before = desktop.workspaces.len();
        let bindings = serde_json::to_value(&desktop.bindings).unwrap();
        let (one, two) = (DisplayRef::named("DP-1"), DisplayRef::named("DP-2"));
        assert_eq!(desktop.title_workspace_on(&one, "1", "web"), Titled::Changed);
        // The entry for every display is as it was; this display has one
        // of its own for the same workspace, right after it.
        assert_eq!(desktop.workspaces.len(), before + 1);
        let shared = desktop.workspaces.iter().position(|p| p.name == "1").unwrap();
        let own = &desktop.workspaces[shared + 1];
        assert_eq!((own.name.as_str(), own.title.as_deref()), ("1", Some("web")));
        assert_eq!(own.monitor, MonitorSelector::Name("DP-1".to_string()));
        assert_eq!(desktop.workspaces[shared].title, None);
        // Nothing that names the workspace has changed, and nothing has to.
        assert_eq!(serde_json::to_value(&desktop.bindings).unwrap(), bindings);
        assert!(desktop.bindings.iter().any(|binding| matches!(
            &binding.command,
            DispatchCommand::SwitchWorkspaceSelector { selector: WorkspaceSelector::Name(name) } if name == "1"
        )));
        assert_eq!(labels(&desktop, "DP-1")[0], "web");
        assert_eq!(labels(&desktop, "DP-2")[0], "1");
        assert_eq!(labels(&desktop, "DP-1").len(), labels(&desktop, "DP-2").len());
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());

        // Titled again there, as often as one likes: the same entry.
        assert_eq!(desktop.title_workspace_on(&one, "1", "mail"), Titled::Changed);
        assert_eq!(desktop.title_workspace_on(&one, "1", " mail "), Titled::Same);
        assert_eq!(desktop.workspaces.len(), before + 1);
        assert_eq!(labels(&desktop, "DP-1")[0], "mail");
        // Another display has a title of its own for it.
        assert_eq!(desktop.title_workspace_on(&two, "1", "chat"), Titled::Changed);
        assert_eq!(desktop.workspaces.len(), before + 2);
        assert_eq!((labels(&desktop, "DP-1")[0].as_str(), labels(&desktop, "DP-2")[0].as_str()), ("mail", "chat"));
        // The same title on two displays is nobody's trouble.
        assert_eq!(desktop.title_workspace_on(&two, "2", "mail"), Titled::Changed);
        // What another workspace of the display is shown by or named,
        // and a workspace nobody has.
        assert_eq!(desktop.title_workspace_on(&one, "2", "mail"), Titled::Taken);
        assert_eq!(desktop.title_workspace_on(&one, "2", "3"), Titled::Taken);
        assert_eq!(desktop.title_workspace_on(&one, "nothing", "x"), Titled::Unknown);
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());

        // The title given back: the display's own entry says nothing of
        // its own any more and is gone.
        assert_eq!(desktop.title_workspace_on(&one, "1", ""), Titled::Changed);
        assert_eq!(labels(&desktop, "DP-1")[0], "1");
        assert_eq!(desktop.workspaces.len(), before + 2);
        assert!(!desktop.workspaces.iter().any(|profile| {
            profile.name == "1" && profile.monitor == MonitorSelector::Name("DP-1".to_string())
        }));
    }

    #[test]
    fn a_title_for_every_display_is_written_in_every_entry() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        let before = desktop.workspaces.len();
        let one = DisplayRef::named("DP-1");
        assert!(desktop.title_workspace_on(&one, "1", "web").changed());
        assert_eq!(desktop.title_workspace("1", "Mail"), Titled::Changed);
        // (The display's own entry said only its title: it is not needed.)
        assert_eq!(desktop.workspaces.len(), before);
        assert_eq!(labels(&desktop, "DP-1")[0], "Mail");
        assert_eq!(labels(&desktop, "DP-9")[0], "Mail");
        assert_eq!(desktop.title_workspace("2", "mail"), Titled::Taken);
        assert_eq!(desktop.title_workspace("nothing", "x"), Titled::Unknown);
        assert_eq!(desktop.title_workspace("1", "Mail"), Titled::Same);
    }

    #[test]
    fn a_display_has_its_own_entry_before_the_one_for_every_display() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        let shared = desktop.workspaces.iter().position(|p| p.name == "2").unwrap();
        // DP-1 lays workspace 2 out another way, and does not have 3.
        let mut own = desktop.workspaces[shared].clone();
        own.monitor = MonitorSelector::Name("dp-1".to_string());
        own.layout = crate::layout::LayoutType::Monocle;
        desktop.workspaces.push(own);
        let mut off = desktop.workspaces.iter().find(|p| p.name == "3").unwrap().clone();
        off.monitor = MonitorSelector::Name("DP-1".to_string());
        off.enabled = false;
        desktop.workspaces.push(off);
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());

        let one = DisplayRef::named("DP-1");
        let there: Vec<&str> =
            desktop.workspaces_on(&one).iter().map(|profile| profile.name.as_str()).collect();
        // In the order they are written in, the display's own in the
        // place of the one it stands for.
        assert_eq!(&there[..3], ["1", "2", "4"]);
        assert_eq!(
            desktop.workspace_on(&one, "2").unwrap().layout,
            crate::layout::LayoutType::Monocle
        );
        let two = DisplayRef::named("DP-2");
        assert_ne!(
            desktop.workspace_on(&two, "2").unwrap().layout,
            crate::layout::LayoutType::Monocle
        );
        assert!(desktop.workspace_on(&two, "3").is_some());

        // Twice for the same display: told of.
        let again = desktop.workspaces[shared].clone();
        desktop.workspaces.push(again);
        assert!(!desktop.validate().is_empty());
    }

    #[test]
    fn a_workspace_of_the_moment_is_written_down_for_its_display() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        let one = DisplayRef::named("DP-1");
        assert!(desktop.keep_workspace_on(&one, "10", "music"));
        let kept = desktop.workspace_on(&one, "10").unwrap();
        assert_eq!(kept.label(), "music");
        assert!(kept.persistent && !kept.default);
        assert!(desktop.workspace_on(&DisplayRef::named("DP-2"), "10").is_none());
        // It is the display's last, and one it has is not written twice.
        assert_eq!(desktop.workspaces_on(&one).last().unwrap().name, "10");
        assert!(!desktop.keep_workspace_on(&one, "10", "again"));
        assert!(!desktop.keep_workspace_on(&one, "1", "again"));
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());
    }

    #[test]
    fn a_workspace_is_changed_for_one_display_or_for_all() {
        use crate::layout::LayoutType;
        let mut desktop = crate::examples::starting_configuration().desktop;
        let before = desktop.workspaces.len();
        let (one, two) = (DisplayRef::named("DP-1"), DisplayRef::named("DP-2"));
        let layout = |desktop: &DesktopConfig, display: &DisplayRef, name: &str| {
            desktop.workspace_on(display, name).map(|profile| profile.layout)
        };
        let shared = layout(&desktop, &one, "2").unwrap();
        let other = if shared == LayoutType::Monocle { LayoutType::BSP } else { LayoutType::Monocle };

        // For one display: an entry of its own, and gone again when it
        // says what every display has.
        assert!(desktop.edit_workspace_on(&one, "2", |profile| profile.layout = other));
        assert_eq!(desktop.workspaces.len(), before + 1);
        assert_eq!(layout(&desktop, &one, "2"), Some(other));
        assert_eq!(layout(&desktop, &two, "2"), Some(shared));
        assert!(desktop.own_workspace_entry(&one, "2").is_some());
        // (Its name and its display are not `change`'s to touch.)
        assert!(desktop.edit_workspace_on(&one, "2", |profile| {
            profile.name = "x".to_string();
            profile.monitor = MonitorSelector::Any;
        }));
        assert_eq!(layout(&desktop, &one, "2"), Some(other));
        assert!(desktop.edit_workspace_on(&one, "2", |profile| profile.layout = shared));
        assert_eq!(desktop.workspaces.len(), before);
        assert!(!desktop.edit_workspace_on(&one, "nothing", |_| {}));

        // For every display: in every entry.
        assert!(desktop.title_workspace_on(&one, "2", "mail").changed());
        assert!(desktop.edit_workspace("2", |profile| profile.layout = other));
        assert_eq!(layout(&desktop, &one, "2"), Some(other));
        assert_eq!(layout(&desktop, &two, "2"), Some(other));
        // As every display has it again.
        assert!(desktop.share_workspace_on(&one, "2"));
        assert_eq!(desktop.workspace_on(&one, "2").unwrap().label(), "2");
        assert!(!desktop.share_workspace_on(&one, "2"));

        // Not on this display, and back.
        assert!(desktop.have_workspace_on(&one, "3", false));
        assert!(desktop.workspace_on(&one, "3").is_none());
        assert!(desktop.workspace_on(&two, "3").is_some());
        assert!(desktop.have_workspace_on(&one, "3", true));
        assert!(desktop.workspace_on(&one, "3").is_some());
        assert_eq!(desktop.workspaces.len(), before);

        // The one a display starts with: no other is that there.
        desktop.start_with_workspace(Some(&one), "2");
        let starts = |desktop: &DesktopConfig, display: &DisplayRef| -> Vec<String> {
            desktop
                .workspaces_on(display)
                .iter()
                .filter(|profile| profile.default)
                .map(|profile| profile.name.clone())
                .collect()
        };
        assert_eq!(starts(&desktop, &one), ["2"]);
        assert_eq!(starts(&desktop, &two), ["1"]);
        desktop.start_with_workspace(None, "4");
        assert_eq!(starts(&desktop, &one), ["4"]);
        assert_eq!(starts(&desktop, &two), ["4"]);
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());
    }

    #[test]
    fn workspaces_are_added_taken_out_and_put_in_order() {
        let mut desktop = crate::examples::starting_configuration().desktop;
        let one = DisplayRef::named("DP-1");
        let names = |desktop: &DesktopConfig, display: &str| -> Vec<String> {
            desktop
                .workspaces_on(&DisplayRef::named(display))
                .iter()
                .map(|profile| profile.name.clone())
                .collect()
        };
        let before = names(&desktop, "DP-1");
        assert_eq!(&before[..4], ["1", "2", "3", "4"]);

        // A new one for every display, and one for a single display: the
        // lowest numbers that are free, after the others.
        let everywhere = desktop.add_workspace(None);
        let here = desktop.add_workspace(Some(&one));
        assert_eq!(everywhere, (before.len() + 1).to_string());
        assert_eq!(here, (before.len() + 2).to_string());
        assert_eq!(names(&desktop, "DP-1").len(), before.len() + 2);
        assert_eq!(names(&desktop, "DP-2").len(), before.len() + 1);
        let added = desktop.workspace_on(&one, &here).unwrap();
        assert!(added.persistent && !added.default && added.title.is_none());
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());

        // Put in another order, with every entry a workspace has.
        assert!(desktop.title_workspace_on(&one, "3", "mail").changed());
        assert!(desktop.move_workspace("3", Some("1")));
        assert_eq!(&names(&desktop, "DP-1")[..4], ["3", "1", "2", "4"]);
        assert_eq!(&names(&desktop, "DP-2")[..4], ["3", "1", "2", "4"]);
        assert_eq!(desktop.workspace_on(&one, "3").unwrap().label(), "mail");
        assert!(desktop.move_workspace("3", None));
        assert_eq!(names(&desktop, "DP-1").last().map(String::as_str), Some("3"));
        assert!(!desktop.move_workspace("3", None));
        assert!(!desktop.move_workspace("3", Some("3")));

        // Taken out: every entry.
        assert!(desktop.remove_workspace("3"));
        assert!(!names(&desktop, "DP-1").contains(&"3".to_string()));
        assert!(!desktop.workspaces.iter().any(|profile| profile.name == "3"));
        assert!(!desktop.remove_workspace("3"));
        // One that only this display has, not wanted there: gone.
        assert!(desktop.have_workspace_on(&one, &here, false));
        assert!(!desktop.workspaces.iter().any(|profile| profile.name == here));
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());
    }

    #[test]
    fn a_display_of_the_state_is_named_as_the_configuration_names_it() {
        let monitor = serde_json::json!({
            "name": "Monitor ACM2702", "connector": r"\\?\DISPLAY#ACM", "is_primary": true
        });
        let display = DisplayRef::of_state(&monitor, Some(1));
        assert_eq!(display.name(), r"\\?\DISPLAY#ACM");
        assert!(display.is_called("monitor acm2702"));
        assert!(display.is_meant_by(&MonitorSelector::Primary));
        assert!(display.is_meant_by(&MonitorSelector::Index(1)));
        assert!(!display.is_meant_by(&MonitorSelector::Index(0)));
        assert!(display.is_meant_by(&MonitorSelector::Name("Monitor ACM2702".to_string())));
        let bare = DisplayRef::of_state(&serde_json::json!({"name": "eDP-1", "connector": ""}), None);
        assert_eq!(bare.name(), "eDP-1");
    }

    #[test]
    fn an_entry_in_the_place_of_another_becomes_that_workspaces_entry() {
        // (As a panel's renaming wrote it before version 25.)
        let config = crate::Config::from_json(
            r#"{"desktop": {"schema_version": 24, "workspaces": [
                {"name": "Saturn", "monitor": "Any"},
                {"name": "1", "also_called": ["Saturn"], "monitor": {"Name": "DP-1"}},
                {"name": "3", "monitor": "Any"}
            ]}}"#,
        )
        .expect("a configuration");
        let desktop = config.desktop;
        assert_eq!(desktop.schema_version, DESKTOP_CONFIG_SCHEMA_VERSION);
        assert_eq!(desktop.workspaces.len(), 3);
        assert_eq!(labels(&desktop, "DP-1"), ["1", "3"]);
        assert_eq!(labels(&desktop, "DP-2"), ["Saturn", "3"]);
        let own = desktop.workspace_on(&DisplayRef::named("DP-1"), "saturn").unwrap();
        assert_eq!((own.name.as_str(), own.title.as_deref()), ("Saturn", Some("1")));
        assert!(desktop.validate().is_empty(), "{:?}", desktop.validate());
        // Written again, nothing of the old way is left.
        let text = serde_json::to_string(&desktop).unwrap();
        assert!(!text.contains("also_called"));
    }
}
