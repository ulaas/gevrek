//! How rules match windows.

use serde::{Deserialize, Serialize};

/// Matching strategy for window rules
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum MatchStrategy {
    /// Exact match
    Equals,
    /// Contains substring
    Contains,
    /// Starts with
    StartsWith,
    /// Ends with
    EndsWith,
    /// Regular expression
    Regex,
}

/// What property to match against
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum MatchKind {
    /// Match by executable name (e.g., "chrome.exe")
    Exe,
    /// Match by window class
    Class,
    /// Match by window title
    Title,
    /// Match by full executable path
    Path,
    /// Match by application id: what a Wayland program calls itself
    /// (`org.mozilla.firefox`); on Windows the program name without `.exe`.
    /// A rule written with it works on both systems.
    AppId,
    /// Match any runtime-owned window tag.
    Tag,
    /// Match the current workspace name or numeric index.
    Workspace,
    /// Match windows that have a parent (child windows)
    HasParent,
    /// Match by visibility state
    IsVisible,
    /// Match tool windows (WS_EX_TOOLWINDOW)
    IsToolWindow,
    /// Match non-activatable windows (WS_EX_NOACTIVATE)
    IsNoActivate,
    /// Match windows that have an owner
    HasOwner,
    /// Match cloaked windows (Windows 8+)
    IsCloaked,
    /// Match layered windows (WS_EX_LAYERED)
    IsLayered,
}
