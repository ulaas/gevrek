use crate::WindowHandle;
pub use gevrek_model::rules::{MatchKind, MatchStrategy};
use gevrek_model::settings::{
    PredicateMode, RuleColor, RuleEffect, RuleMeasure, RulePredicate, WindowRuleProfile,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A rule for identifying windows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowRule {
    /// Identifier/pattern to match
    pub id: String,
    /// What to match against
    pub kind: MatchKind,
    /// How to match
    pub strategy: MatchStrategy,
}

impl WindowRule {
    pub fn new(id: String, kind: MatchKind, strategy: MatchStrategy) -> Self {
        Self { id, kind, strategy }
    }

    /// Check if a string matches this rule
    pub fn matches(&self, value: &str) -> bool {
        match self.strategy {
            MatchStrategy::Equals => value == self.id,
            MatchStrategy::Contains => value.contains(&self.id),
            MatchStrategy::StartsWith => value.starts_with(&self.id),
            MatchStrategy::EndsWith => value.ends_with(&self.id),
            MatchStrategy::Regex => regex::Regex::new(&self.id)
                .map(|regex| regex.is_match(value))
                .unwrap_or(false),
        }
    }
}

/// Rule for windows that should float (not be tiled)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatRule {
    pub rule: WindowRule,
    /// Optional comment explaining why this should float
    pub comment: Option<String>,
}

impl FloatRule {
    pub fn new(rule: WindowRule) -> Self {
        Self {
            rule,
            comment: None,
        }
    }

    pub fn with_comment(mut self, comment: String) -> Self {
        self.comment = Some(comment);
        self
    }
}

/// Rule for windows that should be ignored by the window manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IgnoreRule {
    pub rule: WindowRule,
    /// Optional comment explaining why this should be ignored
    pub comment: Option<String>,
}

impl IgnoreRule {
    pub fn new(rule: WindowRule) -> Self {
        Self {
            rule,
            comment: None,
        }
    }

    pub fn with_comment(mut self, comment: String) -> Self {
        self.comment = Some(comment);
        self
    }
}

/// Rule for windows that should be explicitly managed (override ignore)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManageRule {
    pub rule: WindowRule,
}

impl ManageRule {
    pub fn new(rule: WindowRule) -> Self {
        Self { rule }
    }
}

/// Rule for assigning windows to specific workspaces
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRule {
    pub rule: WindowRule,
    /// Monitor index (None = any monitor)
    pub monitor_idx: Option<usize>,
    /// Workspace index on the monitor
    pub workspace_idx: usize,
}

impl WorkspaceRule {
    pub fn new(rule: WindowRule, workspace_idx: usize) -> Self {
        Self {
            rule,
            monitor_idx: None,
            workspace_idx,
        }
    }

    pub fn with_monitor(mut self, monitor_idx: usize) -> Self {
        self.monitor_idx = Some(monitor_idx);
        self
    }
}

/// Information about a window for rule matching
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub handle: WindowHandle,
    pub title: String,
    pub class: String,
    pub exe: String,
    pub path: String,
    /// The application id: a Wayland program's own name for itself; on
    /// Windows the program name without `.exe`.
    pub app_id: String,
    /// Window has a parent (is a child window)
    pub has_parent: bool,
    /// Window is visible
    pub is_visible: bool,
    /// Window has WS_EX_TOOLWINDOW style
    pub is_tool_window: bool,
    /// Window has WS_EX_NOACTIVATE style
    pub is_noactivate: bool,
    /// Window has an owner
    pub has_owner: bool,
    /// Window is cloaked (Windows 8+)
    pub is_cloaked: bool,
    /// Window has WS_EX_LAYERED
    pub is_layered: bool,
    pub tags: BTreeSet<String>,
    pub workspace: Option<String>,
    pub workspace_index: Option<usize>,
}

/// Rule engine for managing and applying window rules
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleEngine {
    pub float_rules: Vec<FloatRule>,
    pub ignore_rules: Vec<IgnoreRule>,
    pub manage_rules: Vec<ManageRule>,
    pub workspace_rules: Vec<WorkspaceRule>,
    #[serde(default)]
    pub declarative_rules: Vec<WindowRuleProfile>,
}

/// The combined result of every matching declarative rule.
///
/// Rules are evaluated in configuration order. Later effects override earlier
/// effects for the same property, matching the behavior users expect from a
/// cascading desktop configuration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleResolution {
    pub managed: Option<bool>,
    pub floating: Option<bool>,
    pub workspace: Option<(Option<usize>, usize)>,
    pub always_on_top: Option<bool>,
    pub no_initial_focus: Option<bool>,
    pub no_focus: Option<bool>,
    pub no_follow_mouse: Option<bool>,
    pub stay_focused: Option<bool>,
    pub persistent_size: Option<bool>,
    pub opacity: Option<u8>,
    pub dim_inactive: Option<bool>,
    pub shadow: Option<bool>,
    pub position: Option<RulePosition>,
    pub size: Option<(RuleMeasure, RuleMeasure)>,
    pub min_size: Option<(RuleMeasure, RuleMeasure)>,
    pub max_size: Option<(RuleMeasure, RuleMeasure)>,
    pub keep_aspect_ratio: Option<bool>,
    pub maximize: bool,
    pub fullscreen: bool,
    pub border_color: Option<RuleColor>,
    pub caption_color: Option<RuleColor>,
    pub backdrop: Option<gevrek_model::platform::windows::SystemBackdrop>,
    pub corners: Option<gevrek_model::platform::windows::WindowCorner>,
    pub tags: BTreeSet<String>,
    pub matched_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RulePosition {
    Coordinates { x: RuleMeasure, y: RuleMeasure },
    Centered,
}

impl RuleResolution {
    pub fn effect_descriptions(&self) -> Vec<String> {
        fn measure(value: &RuleMeasure) -> String {
            match value {
                RuleMeasure::Pixels(value) => format!("{value}px"),
                RuleMeasure::Percent(value) => format!("{value}%"),
            }
        }

        let mut effects = Vec::new();
        if let Some(managed) = self.managed {
            effects.push(if managed { "managed" } else { "ignored" }.to_string());
        }
        if let Some(floating) = self.floating {
            effects.push(if floating { "floating" } else { "tiled" }.to_string());
        }
        if let Some((monitor, workspace)) = self.workspace {
            effects.push(match monitor {
                Some(monitor) => format!("workspace monitor:{monitor} index:{workspace}"),
                None => format!("workspace {workspace}"),
            });
        }
        if let Some(topmost) = self.always_on_top {
            effects.push(format!("topmost {topmost}"));
        }
        if let Some(no_initial_focus) = self.no_initial_focus {
            effects.push(format!("no initial focus {no_initial_focus}"));
        }
        if let Some(no_focus) = self.no_focus {
            effects.push(format!("no focus {no_focus}"));
        }
        if let Some(no_follow_mouse) = self.no_follow_mouse {
            effects.push(format!("no follow mouse {no_follow_mouse}"));
        }
        if let Some(stay_focused) = self.stay_focused {
            effects.push(format!("stay focused {stay_focused}"));
        }
        if let Some(persistent_size) = self.persistent_size {
            effects.push(format!("persistent size {persistent_size}"));
        }
        if let Some(opacity) = self.opacity {
            effects.push(format!("opacity {opacity}"));
        }
        if let Some(dimmed) = self.dim_inactive {
            effects.push(format!("inactive dim {dimmed}"));
        }
        if let Some(shadow) = self.shadow {
            effects.push(format!("shadow {shadow}"));
        }
        if let Some(position) = &self.position {
            effects.push(match position {
                RulePosition::Coordinates { x, y } => {
                    format!("move {}, {}", measure(x), measure(y))
                }
                RulePosition::Centered => "centered".to_string(),
            });
        }
        if let Some((width, height)) = &self.size {
            effects.push(format!("resize {} × {}", measure(width), measure(height)));
        }
        if let Some((width, height)) = &self.min_size {
            effects.push(format!("minimum {} × {}", measure(width), measure(height)));
        }
        if let Some((width, height)) = &self.max_size {
            effects.push(format!("maximum {} × {}", measure(width), measure(height)));
        }
        if let Some(keep_aspect_ratio) = self.keep_aspect_ratio {
            effects.push(format!("keep aspect ratio {keep_aspect_ratio}"));
        }
        if self.maximize {
            effects.push("maximized".to_string());
        }
        if self.fullscreen {
            effects.push("fullscreen".to_string());
        }
        if let Some(color) = self.border_color {
            effects.push(format!("border {color:?}"));
        }
        if let Some(color) = self.caption_color {
            effects.push(format!("caption {color:?}"));
        }
        if let Some(backdrop) = self.backdrop {
            effects.push(format!("backdrop {backdrop:?}"));
        }
        if let Some(corners) = self.corners {
            effects.push(format!("corners {corners:?}"));
        }
        effects
    }
}

impl RuleEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a float rule
    pub fn add_float_rule(&mut self, rule: FloatRule) {
        self.float_rules.push(rule);
    }

    /// Add an ignore rule
    pub fn add_ignore_rule(&mut self, rule: IgnoreRule) {
        self.ignore_rules.push(rule);
    }

    /// Add a manage rule
    pub fn add_manage_rule(&mut self, rule: ManageRule) {
        self.manage_rules.push(rule);
    }

    /// Add a workspace rule
    pub fn add_workspace_rule(&mut self, rule: WorkspaceRule) {
        self.workspace_rules.push(rule);
    }

    pub fn set_declarative_rules(&mut self, rules: Vec<WindowRuleProfile>) {
        self.declarative_rules = rules;
    }

    pub fn resolve(&self, window: &WindowInfo) -> RuleResolution {
        self.resolve_filtered(window, |_| true)
    }

    pub fn resolve_dynamic(&self, window: &WindowInfo) -> RuleResolution {
        self.resolve_filtered(window, |rule| rule.dynamic)
    }

    pub fn resolve_static(&self, window: &WindowInfo) -> RuleResolution {
        self.resolve_filtered(window, |rule| !rule.dynamic)
    }

    fn resolve_filtered(
        &self,
        window: &WindowInfo,
        include: impl Fn(&WindowRuleProfile) -> bool,
    ) -> RuleResolution {
        let mut effective_window = window.clone();
        let mut resolution = RuleResolution {
            tags: window.tags.clone(),
            ..RuleResolution::default()
        };
        for rule in self
            .declarative_rules
            .iter()
            .filter(|rule| rule.enabled && include(rule))
        {
            let matched = match rule.mode {
                PredicateMode::All => rule
                    .predicates
                    .iter()
                    .all(|predicate| self.predicate_matches(predicate, &effective_window)),
                PredicateMode::Any => rule
                    .predicates
                    .iter()
                    .any(|predicate| self.predicate_matches(predicate, &effective_window)),
            };
            if !matched {
                continue;
            }
            resolution.matched_rules.push(rule.name.clone());

            for effect in &rule.effects {
                match effect {
                    // From a newer gevrek: kept in the file, not applied.
                    RuleEffect::Unknown(_) => {}
                    RuleEffect::Float => resolution.floating = Some(true),
                    RuleEffect::Tile => resolution.floating = Some(false),
                    RuleEffect::Ignore => resolution.managed = Some(false),
                    RuleEffect::Manage => resolution.managed = Some(true),
                    RuleEffect::MoveToWorkspace {
                        monitor_index,
                        workspace_index,
                    } => resolution.workspace = Some((*monitor_index, *workspace_index)),
                    RuleEffect::AlwaysOnTop(value) => {
                        resolution.always_on_top = Some(*value);
                    }
                    RuleEffect::NoInitialFocus(value) => {
                        resolution.no_initial_focus = Some(*value);
                    }
                    RuleEffect::NoFocus(value) => {
                        resolution.no_focus = Some(*value);
                    }
                    RuleEffect::NoFollowMouse(value) => {
                        resolution.no_follow_mouse = Some(*value);
                    }
                    RuleEffect::StayFocused(value) => {
                        resolution.stay_focused = Some(*value);
                    }
                    RuleEffect::PersistentSize(value) => {
                        resolution.persistent_size = Some(*value);
                    }
                    RuleEffect::Opacity(value) => resolution.opacity = Some(*value),
                    RuleEffect::DimInactive(value) => {
                        resolution.dim_inactive = Some(*value);
                    }
                    RuleEffect::Shadow(value) => {
                        resolution.shadow = Some(*value);
                    }
                    RuleEffect::Move { x, y } => {
                        resolution.position = Some(RulePosition::Coordinates {
                            x: x.clone(),
                            y: y.clone(),
                        });
                    }
                    RuleEffect::Resize { width, height } => {
                        resolution.size = Some((width.clone(), height.clone()));
                    }
                    RuleEffect::MinSize { width, height } => {
                        resolution.min_size = Some((width.clone(), height.clone()));
                    }
                    RuleEffect::MaxSize { width, height } => {
                        resolution.max_size = Some((width.clone(), height.clone()));
                    }
                    RuleEffect::KeepAspectRatio(value) => {
                        resolution.keep_aspect_ratio = Some(*value);
                    }
                    RuleEffect::Center => {
                        resolution.position = Some(RulePosition::Centered);
                    }
                    RuleEffect::Maximize => resolution.maximize = true,
                    RuleEffect::Fullscreen => resolution.fullscreen = true,
                    RuleEffect::BorderColor(color) => {
                        resolution.border_color = Some(*color);
                    }
                    RuleEffect::CaptionColor(color) => {
                        resolution.caption_color = Some(*color);
                    }
                    RuleEffect::Backdrop(backdrop) => {
                        resolution.backdrop = Some(*backdrop);
                    }
                    RuleEffect::Corners(corners) => {
                        resolution.corners = Some(*corners);
                    }
                    RuleEffect::AddTag(tag) => {
                        let tag = normalize_tag(tag);
                        if !tag.is_empty() {
                            effective_window.tags.insert(tag.clone());
                            resolution.tags.insert(tag);
                        }
                    }
                    RuleEffect::RemoveTag(tag) => {
                        let tag = normalize_tag(tag);
                        effective_window.tags.remove(&tag);
                        resolution.tags.remove(&tag);
                    }
                }
            }
        }
        resolution
    }

    pub fn set_rule_enabled(&mut self, name: &str, enabled: bool) -> Result<bool, String> {
        let matches = self
            .declarative_rules
            .iter_mut()
            .filter(|rule| rule.name.eq_ignore_ascii_case(name))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(format!("No rule named '{name}' exists"));
        }
        if matches.len() > 1 {
            return Err(format!("Rule name '{name}' is ambiguous"));
        }
        matches.into_iter().next().unwrap().enabled = enabled;
        Ok(enabled)
    }

    pub fn toggle_rule(&mut self, name: &str) -> Result<bool, String> {
        let enabled = self
            .declarative_rules
            .iter()
            .find(|rule| rule.name.eq_ignore_ascii_case(name))
            .map(|rule| !rule.enabled)
            .ok_or_else(|| format!("No rule named '{name}' exists"))?;
        self.set_rule_enabled(name, enabled)
    }

    pub fn rule_states(&self) -> Vec<(String, bool, bool)> {
        self.declarative_rules
            .iter()
            .map(|rule| (rule.name.clone(), rule.enabled, rule.dynamic))
            .collect()
    }

    /// Check if a window should float
    pub fn should_float(&self, window: &WindowInfo) -> bool {
        if let Some(floating) = self.resolve_static(window).floating {
            return floating;
        }
        self.float_rules
            .iter()
            .any(|fr| self.rule_matches(&fr.rule, window))
    }

    /// Check if a window should be ignored
    /// Returns false if there's an explicit manage rule
    pub fn should_ignore(&self, window: &WindowInfo) -> bool {
        if let Some(managed) = self.resolve_static(window).managed {
            return !managed;
        }
        // Check manage rules first (they override ignore)
        if self
            .manage_rules
            .iter()
            .any(|mr| self.rule_matches(&mr.rule, window))
        {
            return false;
        }

        // Check ignore rules
        if let Some(matched) = self
            .ignore_rules
            .iter()
            .find(|ir| self.rule_matches(&ir.rule, window))
        {
            log::debug!(
                "Window ignored by rule {:?} ({:?}): title={:?} class={:?} exe={:?}",
                matched.rule.id,
                matched.rule.kind,
                window.title,
                window.class,
                window.exe,
            );
            return true;
        }
        false
    }

    /// Get workspace assignment for a window
    pub fn get_workspace_assignment(&self, window: &WindowInfo) -> Option<&WorkspaceRule> {
        self.workspace_rules
            .iter()
            .find(|wr| self.rule_matches(&wr.rule, window))
    }

    /// Check if a rule matches a window
    fn rule_matches(&self, rule: &WindowRule, window: &WindowInfo) -> bool {
        match rule.kind {
            // String-based matching
            MatchKind::Exe => rule.matches(&window.exe),
            MatchKind::Class => rule.matches(&window.class),
            MatchKind::Title => rule.matches(&window.title),
            MatchKind::Path => rule.matches(&window.path),
            MatchKind::AppId => rule.matches(&window.app_id),
            MatchKind::Tag => window.tags.iter().any(|tag| rule.matches(tag)),
            MatchKind::Workspace => {
                window
                    .workspace
                    .as_deref()
                    .map(|workspace| rule.matches(workspace))
                    .unwrap_or(false)
                    || window
                        .workspace_index
                        .map(|index| rule.matches(&index.to_string()))
                        .unwrap_or(false)
            }

            // Boolean property matching - pattern "true" or "false"
            MatchKind::HasParent => {
                let expected = rule.id.to_lowercase() == "true";
                window.has_parent == expected
            }
            MatchKind::IsVisible => {
                let expected = rule.id.to_lowercase() == "true";
                window.is_visible == expected
            }
            MatchKind::IsToolWindow => {
                let expected = rule.id.to_lowercase() == "true";
                window.is_tool_window == expected
            }
            MatchKind::IsNoActivate => {
                let expected = rule.id.to_lowercase() == "true";
                window.is_noactivate == expected
            }
            MatchKind::HasOwner => {
                let expected = rule.id.to_lowercase() == "true";
                window.has_owner == expected
            }
            MatchKind::IsCloaked => {
                let expected = rule.id.to_lowercase() == "true";
                window.is_cloaked == expected
            }
            MatchKind::IsLayered => {
                let expected = rule.id.to_lowercase() == "true";
                window.is_layered == expected
            }
        }
    }

    fn predicate_matches(&self, predicate: &RulePredicate, window: &WindowInfo) -> bool {
        let matched = match predicate.kind {
            MatchKind::Exe => Self::string_matches(
                &window.exe,
                &predicate.value,
                &predicate.strategy,
                predicate.case_sensitive,
            ),
            MatchKind::Class => Self::string_matches(
                &window.class,
                &predicate.value,
                &predicate.strategy,
                predicate.case_sensitive,
            ),
            MatchKind::AppId => Self::string_matches(
                &window.app_id,
                &predicate.value,
                &predicate.strategy,
                predicate.case_sensitive,
            ),
            MatchKind::Title => Self::string_matches(
                &window.title,
                &predicate.value,
                &predicate.strategy,
                predicate.case_sensitive,
            ),
            MatchKind::Path => Self::string_matches(
                &window.path,
                &predicate.value,
                &predicate.strategy,
                predicate.case_sensitive,
            ),
            MatchKind::Tag => window.tags.iter().any(|tag| {
                Self::string_matches(
                    tag,
                    &predicate.value,
                    &predicate.strategy,
                    predicate.case_sensitive,
                )
            }),
            MatchKind::Workspace => {
                Self::string_matches(
                    window.workspace.as_deref().unwrap_or_default(),
                    &predicate.value,
                    &predicate.strategy,
                    predicate.case_sensitive,
                ) || window
                    .workspace_index
                    .map(|index| {
                        Self::string_matches(
                            &index.to_string(),
                            &predicate.value,
                            &predicate.strategy,
                            predicate.case_sensitive,
                        )
                    })
                    .unwrap_or(false)
            }
            MatchKind::HasParent => window.has_parent == Self::expected_boolean(&predicate.value),
            MatchKind::IsVisible => window.is_visible == Self::expected_boolean(&predicate.value),
            MatchKind::IsToolWindow => {
                window.is_tool_window == Self::expected_boolean(&predicate.value)
            }
            MatchKind::IsNoActivate => {
                window.is_noactivate == Self::expected_boolean(&predicate.value)
            }
            MatchKind::HasOwner => window.has_owner == Self::expected_boolean(&predicate.value),
            MatchKind::IsCloaked => window.is_cloaked == Self::expected_boolean(&predicate.value),
            MatchKind::IsLayered => window.is_layered == Self::expected_boolean(&predicate.value),
        };
        matched != predicate.negate
    }

    fn expected_boolean(value: &str) -> bool {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        )
    }

    fn string_matches(
        actual: &str,
        expected: &str,
        strategy: &MatchStrategy,
        case_sensitive: bool,
    ) -> bool {
        if matches!(strategy, MatchStrategy::Regex) {
            return regex::RegexBuilder::new(expected)
                .case_insensitive(!case_sensitive)
                .build()
                .map(|regex| regex.is_match(actual))
                .unwrap_or(false);
        }

        let (actual, expected) = if case_sensitive {
            (actual.to_string(), expected.to_string())
        } else {
            (actual.to_lowercase(), expected.to_lowercase())
        };
        match strategy {
            MatchStrategy::Equals => actual == expected,
            MatchStrategy::Contains => actual.contains(&expected),
            MatchStrategy::StartsWith => actual.starts_with(&expected),
            MatchStrategy::EndsWith => actual.ends_with(&expected),
            MatchStrategy::Regex => unreachable!(),
        }
    }

    /// Create common float rules for typical floating windows
    pub fn add_common_float_rules(&mut self) {
        let common_floats = vec![
            // gevrek's own settings program: a window to have beside what
            // it sets, not a tile among the others. (A rule of the
            // person's own with the effect Tile takes it back, and the
            // command that floats or tiles a window does for the moment.)
            (
                "gevrek-studio",
                MatchKind::AppId,
                MatchStrategy::Equals,
                "gevrek-studio",
            ),
            (
                "gevrek-studio",
                MatchKind::Class,
                MatchStrategy::Equals,
                "gevrek-studio",
            ),
            (
                "Steam",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Steam's windows, where the name begins with a capital",
            ),
            // Steam, and what it starts: its own windows (the library,
            // the friends list, the small ones that come and go), games
            // from its library and what they run through (Proton). Asked
            // for by the person; a rule of the person's own with the
            // effect Tile takes any of them back.
            (
                "steam",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Steam's windows, and games through Proton (steam_app_...)",
            ),
            (
                "steam",
                MatchKind::AppId,
                MatchStrategy::StartsWith,
                "Steam's windows, by what they call themselves",
            ),
            ("steam", MatchKind::Exe, MatchStrategy::Equals, "Steam"),
            (
                "steamwebhelper",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Steam's own pages",
            ),
            (
                "/steamapps/",
                MatchKind::Path,
                MatchStrategy::Contains,
                "Programs of Steam's library",
            ),
            (
                "/compatibilitytools.d/",
                MatchKind::Path,
                MatchStrategy::Contains,
                "Programs run through a compatibility tool of Steam's",
            ),
            // Whoever asks for a password or for leave to do something:
            // the keyring's question, the agents that ask on behalf of
            // polkit, the askers of GnuPG and ssh, the wallet, and the
            // file choosers of the desktop portals. (On Wayland a window
            // that says it is a dialog floats anyway; these are for the
            // ones that do not say so.)
            (
                "gcr-prompter",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "The keyring asks for a password",
            ),
            (
                "polkit",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "An agent asks for leave on behalf of polkit",
            ),
            (
                "policykit",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "An agent asks for leave on behalf of polkit",
            ),
            (
                "pinentry",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "GnuPG asks for a passphrase",
            ),
            (
                "askpass",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "ssh or sudo asks for a password",
            ),
            (
                "kwallet",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "The wallet asks for its password",
            ),
            (
                "ksecretd",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "The wallet asks for its password",
            ),
            (
                "xdg-desktop-portal",
                MatchKind::AppId,
                MatchStrategy::Contains,
                "A file chooser or question of a desktop portal",
            ),
            // Dialog boxes
            (
                "dialog",
                MatchKind::Class,
                MatchStrategy::Contains,
                "Dialog boxes",
            ),
            (
                "#32770",
                MatchKind::Class,
                MatchStrategy::Equals,
                "System dialogs",
            ),
            // Popups
            (
                "popup",
                MatchKind::Class,
                MatchStrategy::Contains,
                "Popup windows",
            ),
            (
                "Menu",
                MatchKind::Class,
                MatchStrategy::Contains,
                "Menu windows",
            ),
            // Tool windows
            (
                "ToolWindow",
                MatchKind::Class,
                MatchStrategy::Contains,
                "Tool windows",
            ),
            // Splash screens
            (
                "SplashScreen",
                MatchKind::Class,
                MatchStrategy::Contains,
                "Splash screens",
            ),
            // System tools (often unmanageable/protected)
            (
                "Taskmgr.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Task Manager",
            ),
            (
                "msedge.exe",
                MatchKind::Exe,
                MatchStrategy::Contains,
                "Edge browser",
            ), // Edge has special window styles
            (
                "SystemSettings.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Windows Settings",
            ),
            (
                "ShellExperienceHost.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Windows Shell Experience",
            ),
            (
                "SearchHost.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Windows Search",
            ),
            (
                "StartMenuExperienceHost.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Start Menu",
            ),
            (
                "TextInputHost.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Touch Keyboard",
            ),
            // Common applications that should float
            (
                "Notepad.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Notepad",
            ),
            (
                "calc.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Calculator",
            ),
            (
                "mspaint.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Paint",
            ),
            (
                "SnippingTool.exe",
                MatchKind::Exe,
                MatchStrategy::Equals,
                "Snipping Tool",
            ),
        ];

        for (id, kind, strategy, comment) in common_floats {
            let rule = WindowRule::new(id.to_string(), kind, strategy);
            let float_rule = FloatRule::new(rule).with_comment(comment.to_string());
            self.add_float_rule(float_rule);
        }
    }

    /// Create system ignore rules for windows that should never be managed
    /// These are filters that were previously hardcoded in window_monitor and window_manager
    pub fn add_system_ignore_rules(&mut self) {
        let mut system_ignores: Vec<(&str, MatchKind, MatchStrategy, &str)> = vec![
            // === Gevrek internal windows (our own creations) ===
            (
                "Gevrek_Border",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Gevrek border windows",
            ),
            (
                "Gevrek_Stackbar",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Gevrek stackbar tab strips",
            ),
            (
                "Gevrek_Window",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Gevrek general windows",
            ),
            (
                "GevrekHotkeyWindow",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Gevrek hotkey listener",
            ),
            (
                "Gevrek_Overlay",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Gevrek overlay windows",
            ),
            // The shell's surfaces (crates/shellkit): a panel, a launcher,
            // a notification, and their popups. The surface's name follows
            // the prefix.
            (
                "Gevrek_Shell",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Gevrek shell surfaces",
            ),
            // The list of shortcuts the settings program shows
            // (`gevrek-studio --shortcuts`): by what it calls itself where
            // a window has such a name (Wayland), by its title where it
            // has not (Windows, where the shell's own list is used unless
            // no shell runs).
            (
                "gevrek-shortcuts",
                MatchKind::AppId,
                MatchStrategy::Equals,
                "gevrek's list of shortcuts",
            ),
            // === Windows shell and taskbar ===
            (
                "Shell_TrayWnd",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Taskbar",
            ),
            (
                "Shell_SecondaryTrayWnd",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Secondary taskbar",
            ),
            (
                "TaskListThumbnailWnd",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Taskbar thumbnails",
            ),
            (
                "MSTaskListWClass",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Taskbar buttons",
            ),
            (
                "MSTaskSwWClass",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Task switcher",
            ),
            (
                "Progman",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Program Manager (desktop)",
            ),
            (
                "WorkerW",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Desktop worker windows",
            ),
            // === UWP and Modern UI ===
            // `ApplicationFrameWindow` is deliberately not ignored: it is the
            // top-level frame of every UWP app (Settings, Calculator, ...) and
            // is managed like any window. Frames of suspended apps or apps on
            // other virtual desktops are cloaked and filtered as such. A
            // top-level CoreWindow, by contrast, is shell UI (Start, search).
            (
                "Windows.UI.Core.CoreWindow",
                MatchKind::Class,
                MatchStrategy::Equals,
                "UWP core window",
            ),
            (
                "Windows.UI.",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Windows UI components",
            ),
            (
                "XamlExplorerHostIslandWindow",
                MatchKind::Class,
                MatchStrategy::Equals,
                "XAML islands",
            ),
            // === Hung-app ghost ===
            // Shown by Windows in place of a window whose app stopped
            // responding (owned by a system process). The app's own window
            // comes back when it recovers.
            (
                "Ghost",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Hung-app ghost window",
            ),
            // === Tooltips and popups ===
            (
                "tooltips_class32",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Tooltips",
            ),
            (
                "SysShadow",
                MatchKind::Class,
                MatchStrategy::Equals,
                "Window shadows",
            ),
            (
                "DropShadow",
                MatchKind::Class,
                MatchStrategy::Contains,
                "Drop shadow windows",
            ),
            // === IME and input ===
            ("IME", MatchKind::Class, MatchStrategy::Equals, "IME window"),
            (
                "MSCTFIME",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "Text input framework",
            ),
            // === DWM and system notifications ===
            (
                "Dwm",
                MatchKind::Class,
                MatchStrategy::StartsWith,
                "DWM internal windows",
            ),
            (
                "ForegroundStaging",
                MatchKind::Class,
                MatchStrategy::Equals,
                "DWM foreground staging",
            ),
            // === Title-based filters for system windows ===
            (
                "DWM Notification Window",
                MatchKind::Title,
                MatchStrategy::Equals,
                "DWM internal notification",
            ),
            (
                "MSCTFIME UI",
                MatchKind::Title,
                MatchStrategy::Equals,
                "IME UI window",
            ),
            (
                "Default IME",
                MatchKind::Title,
                MatchStrategy::Equals,
                "Default IME window",
            ),
            (
                "Program Manager",
                MatchKind::Title,
                MatchStrategy::Equals,
                "Desktop",
            ),
            (
                "Workspace Overlay",
                MatchKind::Title,
                MatchStrategy::Equals,
                "Gevrek workspace overlay",
            ),
            (
                "GDI+ Window",
                MatchKind::Title,
                MatchStrategy::StartsWith,
                "GDI+ rendering windows",
            ),
        ];
        if cfg!(windows) {
            system_ignores.push((
                "Gevrek Shortcuts",
                MatchKind::Title,
                MatchStrategy::Equals,
                "gevrek's list of shortcuts",
            ));
        }

        // Property-based filters
        let property_ignores: Vec<(&str, MatchKind, &str)> = vec![
            ("true", MatchKind::HasParent, "Child windows"),
            (
                "true",
                MatchKind::IsToolWindow,
                "Tool windows (WS_EX_TOOLWINDOW)",
            ),
            (
                "true",
                MatchKind::IsNoActivate,
                "Non-activatable windows (WS_EX_NOACTIVATE)",
            ),
            ("true", MatchKind::IsCloaked, "Cloaked windows"),
        ];

        // Add class-based ignore rules
        for (id, kind, strategy, comment) in system_ignores {
            let rule = WindowRule::new(id.to_string(), kind, strategy);
            let ignore_rule =
                IgnoreRule::new(rule).with_comment(format!("System filter: {}", comment));
            self.add_ignore_rule(ignore_rule);
        }

        // Add property-based ignore rules
        for (id, kind, comment) in property_ignores {
            let rule = WindowRule::new(id.to_string(), kind, MatchStrategy::Equals);
            let ignore_rule =
                IgnoreRule::new(rule).with_comment(format!("System filter: {}", comment));
            self.add_ignore_rule(ignore_rule);
        }
    }
}

/// Which rules a resolution runs: all of them, only the static ones
/// (decided once, when the window appears), or only the dynamic ones
/// (re-evaluated when the window's title, state, workspace or tags change).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleResolutionScope {
    All,
    Static,
    Dynamic,
}

pub fn normalize_tag(tag: &str) -> String {
    tag.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_rule_equals() {
        let rule = WindowRule::new(
            "chrome.exe".to_string(),
            MatchKind::Exe,
            MatchStrategy::Equals,
        );
        assert!(rule.matches("chrome.exe"));
        assert!(!rule.matches("firefox.exe"));
    }

    #[test]
    fn test_window_rule_contains() {
        let rule = WindowRule::new(
            "Visual Studio".to_string(),
            MatchKind::Title,
            MatchStrategy::Contains,
        );
        assert!(rule.matches("Visual Studio Code - Untitled"));
        assert!(rule.matches("Microsoft Visual Studio 2022"));
        assert!(!rule.matches("VS Code"));
    }

    #[test]
    fn steam_and_what_it_starts_float_unless_a_rule_says_otherwise() {
        let mut engine = RuleEngine::new();
        engine.add_common_float_rules();
        let window = |class: &str, exe: &str, path: &str| WindowInfo {
            handle: WindowHandle(1),
            title: "a window".to_string(),
            class: class.to_string(),
            exe: exe.to_string(),
            path: path.to_string(),
            app_id: class.to_string(),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };
        // Steam itself, a game through Proton, a game of the library.
        assert!(engine.should_float(&window(
            "steam",
            "steam",
            "/home/a/.local/share/Steam/ubuntu12_32/steam"
        )));
        assert!(engine.should_float(&window("steamwebhelper", "steamwebhelper", "")));
        assert!(engine.should_float(&window("steam_app_620", "wine64-preloader", "")));
        assert!(engine.should_float(&window(
            "hl2_linux",
            "hl2_linux",
            "/home/a/.local/share/Steam/steamapps/common/Portal 2/hl2_linux"
        )));
        // gevrek's own settings program floats too.
        assert!(engine.should_float(&window(
            "gevrek-studio",
            "gevrek-studio",
            "/usr/bin/gevrek-studio"
        )));
        assert!(engine.should_float(&window("Steam", "steam", "")));
        // A terminal is not Steam's.
        assert!(!engine.should_float(&window("foot", "foot", "/usr/bin/foot")));
        // A rule of the person's own takes one back.
        engine.set_declarative_rules(vec![gevrek_model::settings::WindowRuleProfile {
            name: "Portal tiles".to_string(),
            predicates: vec![gevrek_model::settings::RulePredicate {
                kind: MatchKind::Class,
                strategy: MatchStrategy::Equals,
                value: "hl2_linux".to_string(),
                ..Default::default()
            }],
            effects: vec![gevrek_model::settings::RuleEffect::Tile],
            ..Default::default()
        }]);
        // (A rule is looked at again whenever the window changes, as
        // rules are unless they say otherwise: it says "not floating"
        // and that is what holds.)
        let portal = window(
            "hl2_linux",
            "hl2_linux",
            "/home/a/.local/share/Steam/steamapps/common/Portal 2/hl2_linux",
        );
        assert_eq!(engine.resolve_dynamic(&portal).floating, Some(false));
        assert_eq!(
            engine
                .resolve_dynamic(&window("steam", "steam", ""))
                .floating,
            None
        );
        assert!(engine.should_float(&window("steam", "steam", "")));
    }

    #[test]
    fn hung_app_ghost_windows_are_ignored() {
        let mut engine = RuleEngine::new();
        engine.add_system_ignore_rules();
        let ghost = WindowInfo {
            handle: WindowHandle(1),
            title: "Editor (Not Responding)".to_string(),
            class: "Ghost".to_string(),
            exe: "dwm.exe".to_string(),
            path: String::new(),
            app_id: crate::contract::app_id_of_exe("dwm.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };
        assert!(engine.should_ignore(&ghost));
    }

    #[test]
    fn uwp_frames_are_managed_but_shell_core_windows_are_not() {
        let mut engine = RuleEngine::new();
        engine.add_system_ignore_rules();
        let window = |class: &str, exe: &str| WindowInfo {
            handle: WindowHandle(1),
            title: "Calculator".to_string(),
            class: class.to_string(),
            exe: exe.to_string(),
            path: format!("C:\\Program Files\\WindowsApps\\{exe}"),
            app_id: crate::contract::app_id_of_exe(exe),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };
        assert!(!engine.should_ignore(&window("ApplicationFrameWindow", "CalculatorApp.exe")));
        assert!(engine.should_ignore(&window(
            "Windows.UI.Core.CoreWindow",
            "StartMenuExperienceHost.exe"
        )));
    }

    #[test]
    fn the_shells_surfaces_are_ignored_by_their_class_whatever_their_title() {
        let mut engine = RuleEngine::new();
        engine.add_system_ignore_rules();
        let window = |class: &str| WindowInfo {
            handle: WindowHandle(1),
            title: "Any title".to_string(),
            class: class.to_string(),
            exe: "gevrek-shell.exe".to_string(),
            path: String::new(),
            app_id: crate::contract::app_id_of_exe("gevrek-shell.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };
        // A panel, and a menu opened from it.
        assert!(engine.should_ignore(&window("Gevrek_Shell_gevrek-panel")));
        assert!(engine.should_ignore(&window("Gevrek_ShellPopup_gevrek-panel")));
        // Another program's window that only has the word in its class.
        assert!(!engine.should_ignore(&window("My_Gevrek_Shell")));
    }

    #[test]
    fn test_rule_engine_should_float() {
        let mut engine = RuleEngine::new();
        let rule = WindowRule::new(
            "dialog".to_string(),
            MatchKind::Class,
            MatchStrategy::Contains,
        );
        engine.add_float_rule(FloatRule::new(rule));

        let window = WindowInfo {
            handle: WindowHandle(1),
            title: "Open File".to_string(),
            class: "dialog_box".to_string(),
            exe: "app.exe".to_string(),
            path: "C:\\app.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("app.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };

        assert!(engine.should_float(&window));
    }

    #[test]
    fn test_manage_rule_overrides_ignore() {
        let mut engine = RuleEngine::new();

        let ignore_rule = WindowRule::new(
            "test.exe".to_string(),
            MatchKind::Exe,
            MatchStrategy::Equals,
        );
        engine.add_ignore_rule(IgnoreRule::new(ignore_rule));

        let manage_rule = WindowRule::new(
            "test.exe".to_string(),
            MatchKind::Exe,
            MatchStrategy::Equals,
        );
        engine.add_manage_rule(ManageRule::new(manage_rule));

        let window = WindowInfo {
            handle: WindowHandle(1),
            title: "Test".to_string(),
            class: "test".to_string(),
            exe: "test.exe".to_string(),
            path: "C:\\test.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("test.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };

        // Should not ignore because manage rule overrides
        assert!(!engine.should_ignore(&window));
    }

    #[test]
    fn declarative_rules_support_conditions_negation_and_ordered_overrides() {
        let window = WindowInfo {
            handle: WindowHandle(7),
            title: "Picture in Picture".to_string(),
            class: "Chrome_WidgetWin_1".to_string(),
            exe: "chrome.exe".to_string(),
            path: "C:\\Chrome\\chrome.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("chrome.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: Some("media".to_string()),
            workspace_index: Some(2),
        };
        let matching = RulePredicate {
            kind: MatchKind::Exe,
            strategy: MatchStrategy::Equals,
            value: "CHROME.EXE".to_string(),
            ..RulePredicate::default()
        };
        let pip = RulePredicate {
            kind: MatchKind::Title,
            strategy: MatchStrategy::Contains,
            value: "picture".to_string(),
            ..RulePredicate::default()
        };
        let not_dialog = RulePredicate {
            kind: MatchKind::Class,
            strategy: MatchStrategy::Contains,
            value: "dialog".to_string(),
            negate: true,
            ..RulePredicate::default()
        };

        let mut engine = RuleEngine::new();
        engine.set_declarative_rules(vec![
            WindowRuleProfile {
                name: "Chrome defaults to tiled".to_string(),
                dynamic: false,
                predicates: vec![matching.clone()],
                effects: vec![RuleEffect::Tile],
                ..WindowRuleProfile::default()
            },
            WindowRuleProfile {
                name: "Chrome picture-in-picture floats".to_string(),
                dynamic: true,
                predicates: vec![matching, pip, not_dialog],
                effects: vec![
                    RuleEffect::Float,
                    RuleEffect::AlwaysOnTop(true),
                    RuleEffect::NoInitialFocus(true),
                    RuleEffect::NoFocus(false),
                    RuleEffect::NoFollowMouse(true),
                    RuleEffect::StayFocused(true),
                    RuleEffect::PersistentSize(true),
                    RuleEffect::Opacity(230),
                ],
                ..WindowRuleProfile::default()
            },
        ]);

        let resolution = engine.resolve(&window);
        assert_eq!(resolution.floating, Some(true));
        assert_eq!(resolution.always_on_top, Some(true));
        assert_eq!(resolution.no_initial_focus, Some(true));
        assert_eq!(resolution.no_focus, Some(false));
        assert_eq!(resolution.no_follow_mouse, Some(true));
        assert_eq!(resolution.stay_focused, Some(true));
        assert_eq!(resolution.persistent_size, Some(true));
        assert_eq!(resolution.opacity, Some(230));
        assert!(!engine.should_float(&window));
        assert_eq!(engine.resolve_dynamic(&window).floating, Some(true));
    }

    #[test]
    fn geometry_and_decoration_effects_cascade_and_describe_the_effective_state() {
        let window = WindowInfo {
            handle: WindowHandle(8),
            title: "Picture-in-Picture".to_string(),
            class: "MediaWindow".to_string(),
            exe: "player.exe".to_string(),
            path: "C:\\player.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("player.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: Some("media".to_string()),
            workspace_index: Some(1),
        };
        let mut engine = RuleEngine::new();
        engine.set_declarative_rules(vec![
            WindowRuleProfile {
                name: "Media defaults".to_string(),
                effects: vec![
                    RuleEffect::Move {
                        x: RuleMeasure::Pixels(20),
                        y: RuleMeasure::Pixels(30),
                    },
                    RuleEffect::Resize {
                        width: RuleMeasure::Percent(50),
                        height: RuleMeasure::Percent(60),
                    },
                    RuleEffect::MinSize {
                        width: RuleMeasure::Pixels(320),
                        height: RuleMeasure::Pixels(180),
                    },
                    RuleEffect::MaxSize {
                        width: RuleMeasure::Percent(90),
                        height: RuleMeasure::Percent(85),
                    },
                    RuleEffect::KeepAspectRatio(true),
                    RuleEffect::Backdrop(gevrek_model::platform::windows::SystemBackdrop::Mica),
                    RuleEffect::Corners(gevrek_model::platform::windows::WindowCorner::Round),
                    RuleEffect::DimInactive(true),
                    RuleEffect::Shadow(false),
                ],
                ..WindowRuleProfile::default()
            },
            WindowRuleProfile {
                name: "PiP overrides".to_string(),
                effects: vec![
                    RuleEffect::Center,
                    RuleEffect::BorderColor(RuleColor::Hidden),
                    RuleEffect::CaptionColor(RuleColor::Rgb {
                        r: 10,
                        g: 20,
                        b: 30,
                    }),
                ],
                ..WindowRuleProfile::default()
            },
        ]);

        let resolution = engine.resolve(&window);
        assert_eq!(resolution.position, Some(RulePosition::Centered));
        assert_eq!(
            resolution.size,
            Some((RuleMeasure::Percent(50), RuleMeasure::Percent(60)))
        );
        assert_eq!(
            resolution.min_size,
            Some((RuleMeasure::Pixels(320), RuleMeasure::Pixels(180)))
        );
        assert_eq!(
            resolution.max_size,
            Some((RuleMeasure::Percent(90), RuleMeasure::Percent(85)))
        );
        assert_eq!(resolution.keep_aspect_ratio, Some(true));
        assert_eq!(resolution.border_color, Some(RuleColor::Hidden));
        assert_eq!(resolution.dim_inactive, Some(true));
        assert_eq!(resolution.shadow, Some(false));
        assert_eq!(
            resolution.caption_color,
            Some(RuleColor::Rgb {
                r: 10,
                g: 20,
                b: 30
            })
        );
        assert!(resolution
            .effect_descriptions()
            .contains(&"resize 50% × 60%".to_string()));
        assert!(resolution
            .effect_descriptions()
            .contains(&"minimum 320px × 180px".to_string()));
        assert!(resolution
            .effect_descriptions()
            .contains(&"maximum 90% × 85%".to_string()));
        assert!(resolution
            .effect_descriptions()
            .contains(&"keep aspect ratio true".to_string()));
        assert!(resolution
            .effect_descriptions()
            .contains(&"centered".to_string()));
        assert!(resolution
            .effect_descriptions()
            .contains(&"inactive dim true".to_string()));
        assert!(resolution
            .effect_descriptions()
            .contains(&"shadow false".to_string()));
    }

    #[test]
    fn tag_effects_cascade_into_later_predicates() {
        let window = WindowInfo {
            handle: WindowHandle(9),
            title: "Inbox".to_string(),
            class: "Mail".to_string(),
            exe: "mail.exe".to_string(),
            path: "C:\\mail.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("mail.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: Some("communications".to_string()),
            workspace_index: Some(3),
        };
        let mut engine = RuleEngine::new();
        engine.set_declarative_rules(vec![
            WindowRuleProfile {
                name: "Classify mail".to_string(),
                predicates: vec![RulePredicate {
                    kind: MatchKind::Exe,
                    strategy: MatchStrategy::Equals,
                    value: "mail.exe".to_string(),
                    ..RulePredicate::default()
                }],
                effects: vec![RuleEffect::AddTag(" Communication ".to_string())],
                ..WindowRuleProfile::default()
            },
            WindowRuleProfile {
                name: "Pin communication".to_string(),
                predicates: vec![RulePredicate {
                    kind: MatchKind::Tag,
                    strategy: MatchStrategy::Equals,
                    value: "communication".to_string(),
                    ..RulePredicate::default()
                }],
                effects: vec![RuleEffect::AlwaysOnTop(true)],
                ..WindowRuleProfile::default()
            },
        ]);

        let resolution = engine.resolve(&window);
        assert!(resolution.tags.contains("communication"));
        assert_eq!(resolution.always_on_top, Some(true));
        assert_eq!(
            resolution.matched_rules,
            vec!["Classify mail", "Pin communication"]
        );
    }

    #[test]
    fn dynamic_resolution_excludes_static_rules_and_named_toggle_is_exact() {
        let window = WindowInfo {
            handle: WindowHandle(10),
            title: "Editor".to_string(),
            class: "Editor".to_string(),
            exe: "editor.exe".to_string(),
            path: "C:\\editor.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("editor.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };
        let mut engine = RuleEngine::new();
        engine.set_declarative_rules(vec![
            WindowRuleProfile {
                name: "Static classify".to_string(),
                dynamic: false,
                effects: vec![RuleEffect::AddTag("static".to_string())],
                ..WindowRuleProfile::default()
            },
            WindowRuleProfile {
                name: "Dynamic float".to_string(),
                dynamic: true,
                effects: vec![RuleEffect::Float],
                ..WindowRuleProfile::default()
            },
        ]);

        let resolution = engine.resolve_dynamic(&window);
        assert_eq!(resolution.floating, Some(true));
        assert!(!resolution.tags.contains("static"));
        assert!(!engine.toggle_rule("dynamic float").unwrap());
        assert_eq!(engine.resolve_dynamic(&window).floating, None);
        assert!(engine.set_rule_enabled("missing", true).is_err());
    }

    #[test]
    fn workspace_predicates_match_name_or_numeric_index() {
        let window = WindowInfo {
            handle: WindowHandle(77),
            title: "Editor".to_string(),
            class: "EditorWindow".to_string(),
            exe: "editor.exe".to_string(),
            path: "C:\\editor.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("editor.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered: false,
            tags: BTreeSet::new(),
            workspace: Some("code".to_string()),
            workspace_index: Some(2),
        };
        let rule = |name: &str, value: &str| WindowRuleProfile {
            name: name.to_string(),
            predicates: vec![RulePredicate {
                extra: Default::default(),
                kind: MatchKind::Workspace,
                strategy: MatchStrategy::Equals,
                value: value.to_string(),
                negate: false,
                case_sensitive: false,
            }],
            effects: vec![RuleEffect::AddTag(name.to_string())],
            ..WindowRuleProfile::default()
        };
        let mut engine = RuleEngine::new();
        engine.set_declarative_rules(vec![rule("by-name", "code"), rule("by-index", "2")]);

        let resolution = engine.resolve_dynamic(&window);
        assert!(resolution.tags.contains("by-name"));
        assert!(resolution.tags.contains("by-index"));
    }

    #[test]
    fn layered_predicate_matches_ws_ex_layered_windows() {
        let window = |is_layered: bool| WindowInfo {
            handle: WindowHandle(88),
            title: "Overlay".to_string(),
            class: "OverlayWindow".to_string(),
            exe: "overlay.exe".to_string(),
            path: "C:\\overlay.exe".to_string(),
            app_id: crate::contract::app_id_of_exe("overlay.exe"),
            has_parent: false,
            is_visible: true,
            is_tool_window: false,
            is_noactivate: false,
            has_owner: false,
            is_cloaked: false,
            is_layered,
            tags: BTreeSet::new(),
            workspace: None,
            workspace_index: None,
        };
        let mut engine = RuleEngine::new();
        engine.set_declarative_rules(vec![WindowRuleProfile {
            name: "Float layered".to_string(),
            predicates: vec![RulePredicate {
                extra: Default::default(),
                kind: MatchKind::IsLayered,
                strategy: MatchStrategy::Equals,
                value: "true".to_string(),
                negate: false,
                case_sensitive: false,
            }],
            effects: vec![RuleEffect::Float],
            ..WindowRuleProfile::default()
        }]);

        assert_eq!(engine.resolve(&window(true)).floating, Some(true));
        assert_eq!(engine.resolve(&window(false)).floating, None);

        // The legacy WindowRule path honors the same boolean kind.
        let legacy = WindowRule::new(
            "false".to_string(),
            MatchKind::IsLayered,
            MatchStrategy::Equals,
        );
        let mut legacy_engine = RuleEngine::new();
        legacy_engine.add_float_rule(FloatRule::new(legacy));
        assert!(legacy_engine.should_float(&window(false)));
        assert!(!legacy_engine.should_float(&window(true)));
    }
}
