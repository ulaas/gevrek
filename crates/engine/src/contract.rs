//! The contract between the engine and a platform backend.
//!
//! A backend does three things for the core (`core::WmCore`):
//!
//! - it answers `WindowQueries`: facts about windows read from the live
//!   desktop (frame, title, class, states, the look a window has now);
//! - it carries out `Effect`s: descriptions of what should be (place this
//!   window, show or hide it, focus it, its border, its state), and it
//!   decides how (Win32 overlay windows, compositor render elements);
//! - it offers the `Desktop` services that are not effects: animation,
//!   decoration setup, stackbar drawing, output and pointer queries,
//!   launching programs.
//!
//! In the other direction the backend's host feeds the core
//! `events::WindowEvent`s (a window appeared, was destroyed, got the focus,
//! moved, was minimized, ...), commands, and the configuration.
//!
//! Coordinates are logical, visible-frame pixels. A backend adds or removes
//! its own frame insets (Win32 invisible DWM borders, server-side borders).

use crate::outputs::MonitorInfo;
use gevrek_model::geometry::{Point, Rect};
use gevrek_model::ids::{ContainerId, OutputId, WindowId};
use gevrek_model::platform::windows::{SystemBackdrop, WindowCorner};
use gevrek_model::settings::AnimationCurve;
use std::time::Duration;

/// A display as the backend enumerated it. Stage 1 reuses the engine's
/// monitor record; it carries the output id, name, stable device path,
/// bounds, work area and scale.
pub type OutputInfo = MonitorInfo;

/// What the engine asks the backend to do. Effects are descriptions; the
/// backend decides how (Win32 overlay windows, compositor render elements).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Place a window and set its states.
    Configure {
        id: WindowId,
        rect: Rect,
        states: WindowStates,
    },
    /// Show or hide a window (workspace switches). The backend picks the
    /// hiding strategy and keeps it crash-safe.
    SetVisible {
        id: WindowId,
        visible: bool,
    },
    Focus(Option<WindowId>),
    Stack {
        id: WindowId,
        op: StackOp,
    },
    /// Ask the window to close.
    Close(WindowId),
    /// Make the whole window translucent: 255 is opaque.
    Opacity {
        id: WindowId,
        alpha: u8,
    },
    /// Put the window into, or take it out of, a state without placing it
    /// (a rule or a command maximizes; a tiled window that maximized itself
    /// is taken back).
    State {
        id: WindowId,
        change: StateChange,
    },
    Decorate {
        id: WindowId,
        deco: Decoration,
    },
    /// Change the border the backend draws around a window, without
    /// describing it again (`Decorate` does that).
    Border {
        id: WindowId,
        change: BorderChange,
    },
    /// Show or hide every border and stackbar at once (hidden while the
    /// overview of a workspace is shown).
    DecorationsVisible(bool),
    /// Hide the stackbar of the container this window is the key of (the
    /// window left the layout).
    HideStackbar(WindowId),
    /// Tabs for a container holding several windows.
    Stackbar {
        container: ContainerId,
        tabs: Vec<TabInfo>,
        rect: Rect,
    },
    /// Only sent when `Capabilities::render_time_animation` is set.
    Animate {
        id: WindowId,
        from: Rect,
        to: Rect,
        curve: AnimationCurve,
        duration: Duration,
    },
    WarpPointer(Point),
    Launch(LaunchSpec),
    /// Escape hatch for features only one platform has; gated by
    /// `Capabilities`.
    Platform(PlatformEffect),
}

/// One change to a window's border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderChange {
    /// Show it, or hide it and keep it for later (a window on a hidden
    /// workspace, a fullscreen window).
    Visible(bool),
    /// The window is no longer decorated.
    Remove,
    /// The window moved or resized: put the border around it again. A
    /// backend that draws borders at render time has nothing to do.
    Follow,
    /// Draw it in the focused or the unfocused colour.
    Focused(bool),
}

/// How far and how strongly the glow reaches around one window, where
/// that is not as the configuration says for every window (its colour, and
/// whether there is a glow at all, stay the configuration's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlowStrength {
    pub radius: u32,
    pub opacity: u8,
    pub inactive_opacity: u8,
}

impl GlowStrength {
    /// The configuration's glow with this reach and strength.
    pub fn of(
        &self,
        glow: &gevrek_model::settings::GlowEffect,
    ) -> gevrek_model::settings::GlowEffect {
        gevrek_model::settings::GlowEffect {
            radius: self.radius,
            opacity: self.opacity,
            inactive_opacity: self.inactive_opacity,
            ..glow.clone()
        }
    }
}

/// One change of a window's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    Maximized(bool),
    Minimized(bool),
    Fullscreen(bool),
}

/// The states a window is configured with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowStates {
    pub tiled: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub activated: bool,
}

impl WindowStates {
    pub const TILED: Self = Self {
        tiled: true,
        maximized: false,
        fullscreen: false,
        activated: false,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackOp {
    Raise,
    Lower,
    AlwaysOnTop(bool),
}

/// How a window is decorated: border and opacity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Decoration {
    /// `None` removes the border.
    pub border: Option<BorderDecoration>,
    /// 0-255; `None` leaves the window opaque.
    pub opacity: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderDecoration {
    pub width: i32,
    pub offset: i32,
    pub radius: i32,
    pub color: (u8, u8, u8),
    pub alpha: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabInfo {
    pub window: WindowId,
    pub title: String,
    pub focused: bool,
}

/// A program to start.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformEffect {
    Windows(WindowsEffect),
}

/// DWM features (Windows 11).
#[derive(Debug, Clone, PartialEq)]
pub enum WindowsEffect {
    Backdrop {
        id: WindowId,
        backdrop: SystemBackdrop,
    },
    Corner {
        id: WindowId,
        corner: WindowCorner,
    },
    /// Whether the compositor draws the window's frame shadow.
    Shadow {
        id: WindowId,
        policy: gevrek_model::settings::ShadowPolicy,
    },
    /// Show or remove the minimize and maximize buttons of the title bar
    /// (a tiled window has no use for them).
    CaptionButtons {
        id: WindowId,
        shown: bool,
    },
    /// Replace the window's title text.
    Title {
        id: WindowId,
        title: String,
    },
    /// Hide the window from the screen without minimizing it (DWM cloak),
    /// or show it again.
    Cloak {
        id: WindowId,
        cloaked: bool,
    },
    /// The title bar colour: the system's, none, or a fixed one.
    CaptionColor {
        id: WindowId,
        color: gevrek_model::settings::RuleColor,
    },
    /// The DWM-drawn border (`BorderStyle::Native`).
    NativeBorderColor {
        id: WindowId,
        color: gevrek_model::settings::RuleColor,
    },
}

/// What the graph's query nodes may ask about windows. The interpreter
/// reads through this, so it does not depend on a particular backend; a
/// backend answers from the live desktop.
pub trait WindowQueries {
    /// The window that has the keyboard focus.
    fn active_window(&self) -> Option<WindowId>;
    fn exists(&self, id: WindowId) -> bool;
    /// Whether the window's title contains `title`.
    fn has_title(&self, id: WindowId, title: &str) -> Result<bool, String>;
    fn position(&self, id: WindowId) -> Option<(i32, i32)>;
    fn size(&self, id: WindowId) -> Option<(i32, i32)>;
    /// The window's title now.
    fn title(&self, id: WindowId) -> String;
    fn class(&self, id: WindowId) -> String;
    fn exe(&self, id: WindowId) -> String;
    /// The picture the window's program gave this window itself, where
    /// the system has a way for that (xdg-toplevel-icon): a name out of
    /// the icon theme, or the path of a file of its pixels. Empty where
    /// it gave none: the window's picture is then its program's. (On
    /// Windows, which has no such way: the program's file, by its whole
    /// path, whose icon the system draws.)
    fn icon(&self, _id: WindowId) -> String {
        String::new()
    }
    fn path(&self, id: WindowId) -> String;
    /// The application id. A backend whose windows have none of their own
    /// (Windows) gets the program name without its extension.
    fn app_id(&self, id: WindowId) -> String {
        app_id_of_exe(&self.exe(id))
    }
    /// Every window the backend tracks.
    fn windows(&self) -> Vec<gevrek_model::window::WindowInfo>;
    /// The window's frame on the desktop.
    fn rect(&self, id: WindowId) -> Result<Rect, String>;
    fn is_minimized(&self, id: WindowId) -> bool;
    fn is_maximized(&self, id: WindowId) -> bool;
    fn is_fullscreen(&self, id: WindowId) -> bool;
    fn is_visible(&self, id: WindowId) -> bool;
    fn is_always_on_top(&self, id: WindowId) -> bool;
    /// The smallest size the window's program lets it have, a width and
    /// a height in the desktop's pixels (0 where it names none for that
    /// side); `None` where the program names no smallest size, or the
    /// backend cannot know it. A layout gives such a window the room its
    /// neighbours can spare (`layouts::make_room`).
    fn smallest_size(&self, _id: WindowId) -> Option<(i32, i32)> {
        None
    }
    /// The window this one belongs to: the window a dialog asks about, the
    /// one a tool's window is there for (the parent of a Wayland toplevel,
    /// the window an X11 window is transient for, the owner of a window
    /// on Windows). `None` for a window of its own. The window named need
    /// not be one gevrek has in a workspace: the engine looks.
    fn parent(&self, _id: WindowId) -> Option<WindowId> {
        None
    }
    /// The window stands where its program put it (an X11 window that
    /// names its place), or where the desktop put it for what it belongs
    /// to (a dialog over its window): a place that means something, which
    /// the Floating layout leaves it at. False where the place is only
    /// where windows happen to open.
    fn placed_itself(&self, _id: WindowId) -> bool {
        false
    }
    /// The backend knows this window (it has seen it appear).
    fn is_tracked(&self, id: WindowId) -> bool;
    /// The process the window belongs to.
    fn pid(&self, id: WindowId) -> Option<u32>;
    /// Title and class as they were when the window appeared.
    fn initial_title(&self, id: WindowId) -> String;
    fn initial_class(&self, id: WindowId) -> String;
    /// The application hid this window itself (for example to the tray).
    fn hidden_by_app(&self, id: WindowId) -> bool;
    /// The window is one the backend made itself to draw a decoration.
    fn is_decoration(&self, id: WindowId) -> bool;
    /// The backend draws a border around this window.
    fn has_border(&self, id: WindowId) -> bool;
    /// The look the window has now: opacity (0-255), border and title bar
    /// colour, backdrop and corner style.
    fn opacity(&self, id: WindowId) -> u8;
    fn border_color(&self, id: WindowId) -> gevrek_model::settings::RuleColor;
    fn caption_color(&self, id: WindowId) -> gevrek_model::settings::RuleColor;
    fn backdrop(&self, id: WindowId) -> SystemBackdrop;
    fn corners(&self, id: WindowId) -> WindowCorner;
    fn shadow_policy(&self, id: WindowId) -> gevrek_model::settings::ShadowPolicy;
    /// What the window rules match on, read from the live window (without
    /// tags and workspace, which the engine side adds). `None` for a window
    /// that cannot be read (gone, or not responding).
    fn rule_facts(&self, id: WindowId) -> Option<crate::rules::WindowInfo>;
    /// The window rules say to leave this window alone.
    fn ignored_by_rules(&self, id: WindowId) -> bool;
    /// The window rules say this window floats.
    fn floats_by_rules(&self, id: WindowId) -> bool;
}

/// The application id of a program that does not name itself: its file
/// name without the extension, in lower case (`Code.exe` is `code`).
pub fn app_id_of_exe(exe: &str) -> String {
    let name = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    stem.to_ascii_lowercase()
}

/// What the stackbar of one grouped container should show, built by the
/// engine side and handed to `Desktop::sync_stackbars`.
pub struct StackbarContainerState {
    /// The container's focused window. The bar is keyed by it, follows its
    /// visibility, and serves as the focus anchor for tab clicks.
    pub key: WindowId,
    /// Container layout rect `(x, y, width, height)`; the bar occupies the
    /// top strip of `StackbarProfile::height` pixels.
    pub rect: (i32, i32, i32, i32),
    /// Tab windows in tab order.
    pub tabs: Vec<WindowId>,
    /// Index of the focused tab.
    pub focused_tab: usize,
}

/// The desktop as the engine side works with it: it answers
/// `WindowQueries`, carries out `Effect`s, and offers the services that are
/// not effects yet (animation, stackbars, decoration setup, matching
/// windows against patterns, rule resolution against live window facts).
///
/// The executor and the core take `&mut dyn Desktop`, so nothing in them
/// names a particular backend. The service methods are the ones the Win32
/// window manager already had; a second backend implements them, and most
/// of them should become `Effect`s or engine logic over time.
pub trait Desktop: WindowQueries {
    /// Carry out one effect and report whether the desktop accepted it.
    fn apply(&mut self, effect: &Effect) -> anyhow::Result<()>;

    // --- outputs and input ---------------------------------------------------
    /// The output most of the window is on.
    fn output_of(&self, id: WindowId) -> Option<OutputId>;
    /// Whether a window that has just opened and is to be tiled goes to
    /// the output with the keyboard, whatever output it opened on. For a
    /// system that opens a program's window where the program last had
    /// one (Windows); where the desktop itself decides where a window
    /// opens (a Wayland compositor), that place stands.
    fn new_windows_go_where_the_keyboard_is(&self) -> bool {
        false
    }
    /// The output the pointer is on.
    fn output_at_cursor(&self) -> Option<OutputId>;
    /// The primary pointer button is held down (a drag may be going on).
    fn primary_button_down(&self) -> bool;

    // --- placement and animation -------------------------------------------
    /// Place several windows, animated according to the configuration.
    fn apply_animated_rectangles(
        &mut self,
        operations: &[(WindowId, Rect)],
        scope: crate::animation_scheduler::TransitionScope,
    ) -> anyhow::Result<()>;
    /// The windows that are hidden next are those of a workspace that
    /// gives way to another, and `begin_workspace_transition` follows. A
    /// desktop on which a hidden window is gone at once (Windows) keeps
    /// them until then, so that they can go out the way
    /// `desktop.animations.workspace_style` says; a desktop that draws
    /// its windows itself needs nothing of this.
    fn workspace_gives_way(&mut self) {}
    /// Show these windows as live pictures of themselves, small, in
    /// `area`, to choose one of (`expose::arrange` says where); or put
    /// that away where it is shown. The windows themselves stay as they
    /// are. `None`: this desktop cannot, and the windows themselves are
    /// arranged instead (`expose::activate`).
    fn toggle_window_overview(&mut self, _windows: &[WindowId], _area: Rect) -> Option<bool> {
        None
    }
    fn begin_workspace_transition(
        &mut self,
        operations: &[(WindowId, Rect)],
        offset_x: i32,
        offset_y: i32,
    ) -> anyhow::Result<bool>;
    fn start_configured_window_enter(&mut self, id: WindowId) -> anyhow::Result<bool>;
    fn start_window_animation(
        &mut self,
        id: WindowId,
        animation_type: crate::animations::AnimationType,
        duration_ms: u32,
    ) -> anyhow::Result<bool>;
    /// The desktop moved this window itself for an animation: the move
    /// events it causes are not the user's.
    fn is_animation_event_suppressed(&self, id: WindowId) -> bool;
    fn preferred_window_size(&self, id: WindowId) -> Option<(i32, i32)>;

    // --- decoration ----------------------------------------------------------
    /// Give a window its border, as `look` has it.
    fn decorate_window(
        &mut self,
        id: WindowId,
        look: &crate::workspace::BorderLook,
        is_focused: bool,
    ) -> anyhow::Result<()>;
    /// The glow around this window is another than the configuration's
    /// for every window (a floating window's, where floating windows have
    /// a look of their own); `None`: the configuration's again. A backend
    /// that draws no glow of its own choosing has nothing to do.
    fn set_window_glow(&mut self, _id: WindowId, _glow: Option<GlowStrength>) {}
    fn decorate_window_at(
        &mut self,
        id: WindowId,
        at: Rect,
        look: &crate::workspace::BorderLook,
        is_focused: bool,
    ) -> anyhow::Result<()>;
    fn update_border_position_explicit(
        &mut self,
        id: WindowId,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> anyhow::Result<()>;
    fn toggle_title_bar(&mut self, id: WindowId) -> anyhow::Result<bool>;

    // --- stackbars -----------------------------------------------------------
    fn sync_stackbars(
        &mut self,
        desired: &[StackbarContainerState],
        config: &gevrek_model::settings::StackbarProfile,
    ) -> anyhow::Result<()>;

    // --- programs ------------------------------------------------------------
    /// Start a program. Returns its process id.
    fn launch(&mut self, launch: &gevrek_model::settings::LaunchProfile) -> anyhow::Result<u32>;

    // --- windows and rules ---------------------------------------------------
    /// The backend is hiding this window for the WM (it is on a workspace
    /// that is not shown).
    fn is_hidden_by_backend(&self, id: WindowId) -> bool;
    /// The window a native handle value names now (on Windows an HWND; a
    /// reused HWND is a different window id than the destroyed one had).
    fn window_for_native_handle(&self, raw: isize) -> WindowId;
    /// Stop tracking a window that is gone.
    fn untrack_window(&mut self, id: WindowId);
    /// Show again the windows the WM had hidden when it last stopped.
    fn restore_journaled_windows(&mut self) -> usize;
}

#[cfg(test)]
mod tests {
    use super::app_id_of_exe;

    #[test]
    fn a_program_without_an_id_of_its_own_is_known_by_its_file_name() {
        assert_eq!(app_id_of_exe("Code.exe"), "code");
        assert_eq!(
            app_id_of_exe(r"C:\Program Files\Mozilla Firefox\firefox.exe"),
            "firefox"
        );
        assert_eq!(app_id_of_exe("/usr/bin/foot"), "foot");
        assert_eq!(app_id_of_exe("my.app.exe"), "my.app");
        assert_eq!(app_id_of_exe(""), "");
    }
}
