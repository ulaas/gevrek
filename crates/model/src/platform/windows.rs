//! Settings that only the Windows backend can apply (DWM).

use serde::{Deserialize, Serialize};

/// Windows 11 system backdrop types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[repr(i32)]
pub enum SystemBackdrop {
    /// Let the system decide
    Auto = 0,
    /// No backdrop effect
    None = 1,
    /// Mica effect (main material)
    Mica = 2,
    /// Acrylic blur effect
    Acrylic = 3,
    /// Mica Alt (tabbed variant)
    MicaAlt = 4,
}

/// Windows 11 window corner preferences
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[repr(i32)]
pub enum WindowCorner {
    /// System default
    Default = 0,
    /// Do not round corners
    DoNotRound = 1,
    /// Standard rounded corners
    Round = 2,
    /// Small rounded corners
    RoundSmall = 3,
}

/// What only the Windows compositor (DWM) can do for every window: these
/// were `desktop.appearance.backdrop`, `.corners` and `.shadow` up to
/// schema version 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct WindowsPlatformConfig {
    pub backdrop: SystemBackdrop,
    pub corners: WindowCorner,
    /// Whether DWM draws the frame shadow of windows.
    pub shadow: crate::settings::ShadowPolicy,
    /// Windows' taskbar is put away while gevrek's own panel is on the
    /// screen, and its strip is room for windows. It is shown again when
    /// the panel goes and when gevrek stops. On unless switched off: the
    /// panel has the tray's icons and the launcher, and the Windows key
    /// still opens the Start menu. (Where no panel of gevrek's runs, the
    /// taskbar stays.)
    pub hide_taskbar: bool,
    /// The Windows key is gevrek's while gevrek runs: the key alone opens
    /// no Start menu, and a key pressed with it does what gevrek binds it
    /// to or nothing; none of Windows' own shortcuts on the key is carried
    /// out (but the lock, Win+L, which no program can take).
    pub own_windows_key: bool,
    /// With the Windows key held, the left button on a window moves it and
    /// the right button resizes it, wherever on the window the pointer is.
    pub drag_with_windows_key: bool,
    /// The scrolling layout scrolls freely also where a display lies
    /// beside another: a column that reaches past its display's edge is
    /// cut there (the window is given a shape that ends at the edge), so
    /// that it is not seen on the display beside it. Off unless switched
    /// on: a window that is cut loses its round corners and its shadow
    /// while it is, and one whose title bar Windows draws has a plain
    /// bar meanwhile (the bar Windows draws takes no shape, so Windows
    /// is told not to draw it until the window is whole again). Without
    /// it, such displays show whole columns only.
    pub cut_columns_at_displays: bool,
    /// The windows of programs that run as an administrator are gevrek's
    /// windows as any other, where gevrek's helper for them runs
    /// (`gevrek elevated install`, once, or `gevrek elevated start`): a
    /// gevrek that does not run so itself may not move them, and asks the
    /// helper to. Off, and where no helper runs, they are left alone.
    pub manage_elevated_windows: bool,
    /// A window of a program that runs as an administrator is told from
    /// the others by its border: crimson, a lighter red while it has the
    /// keyboard, and with no glow, whatever the borders of the other
    /// windows look like. (What is typed and pressed in such a window is
    /// done with an administrator's rights.) On unless switched off.
    pub mark_elevated_windows: bool,
}

impl Default for WindowsPlatformConfig {
    fn default() -> Self {
        Self {
            backdrop: SystemBackdrop::Auto,
            corners: WindowCorner::Round,
            shadow: crate::settings::ShadowPolicy::Default,
            hide_taskbar: true,
            own_windows_key: true,
            drag_with_windows_key: true,
            cut_columns_at_displays: false,
            manage_elevated_windows: true,
            mark_elevated_windows: true,
        }
    }
}
