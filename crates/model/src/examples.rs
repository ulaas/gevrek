use crate::command::{
    Direction, DispatchCommand, MonitorTarget, WindowSelector, WorkspaceSelector,
};
use crate::config::Config;
use crate::graph::{LogLevel, NodeGraph, NodeType, SystemBackdropType, WindowCornerType};
use crate::layout::{DwindleDirection, LayoutType, MasterPosition, NewWindowPlacement};
use crate::platform::windows::{SystemBackdrop, WindowCorner};
use crate::rules::{MatchKind, MatchStrategy};
use crate::settings::{
    AnimationCurve, BindingDevice, BindingProfile, BindingTrigger, GestureDirection,
    GestureProfile, LaunchProfile, MonitorProfile, MonitorSelector, PredicateMode, ReservedArea,
    RuleColor, RuleEffect, RuleMeasure, RulePredicate, ScratchpadPosition, ScratchpadProfile,
    ShadowPolicy, StartupProfile, StartupRestartPolicy, WindowRuleProfile, WorkspaceProfile,
};
use crate::snap_zones::presets as snap_zone_presets;

#[derive(Debug, Clone)]
pub struct BuiltInExample {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub config: Config,
}

/// An example put on over a configuration of the person's own: the
/// example's look and way of working, with what is the person's and the
/// machine's kept — the keyboard, the mouse and the touchpad
/// (`desktop.input`: a layout is not a look), the displays
/// (`desktop.outputs`), the session's own settings, the night light, the
/// wallpaper, and the person's window and layer rules (after the
/// example's own, so that they are the ones that hold).
///
/// (Loading an example used to replace all of it; a person who tried a
/// look lost their keyboard layouts and their rules with it.)
pub fn put_on(example: &Config, own: &Config) -> Config {
    let mut config = example.clone();
    let (to, from) = (&mut config.desktop, &own.desktop);
    to.input = from.input.clone();
    to.outputs = from.outputs.clone();
    to.session = from.session.clone();
    to.night_light = from.night_light.clone();
    to.wallpaper = from.wallpaper.clone();
    for rule in &from.window_rules {
        // (By its name: a rule the example also has is the example's.)
        if !to.window_rules.iter().any(|known| known.name == rule.name) {
            to.window_rules.push(rule.clone());
        }
    }
    for rule in &from.layer_rules {
        if !to.layer_rules.contains(rule) {
            to.layer_rules.push(rule.clone());
        }
    }
    config
}

pub fn built_in_examples() -> Vec<BuiltInExample> {
    vec![
        example_aurora(),
        example_focus_stage(),
        example_neon_grid(),
        example_paper(),
        example_dwindle_desktop(),
        example_master_productivity(),
        example_multi_monitor(),
        example_dynamic_window_studio(),
        example_tagged_workflow(),
        example_scratchpad_terminal(),
        example_presentation_mode(),
        example_gaming_mode(),
        example_media_lounge(),
        example_accessible_focus(),
        example_automation_showcase(),
        example_snap_zone_workbench(),
        example_advanced_binding_lab(),
        example_minimal_monocle(),
    ]
}

fn example(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    mut config: Config,
) -> BuiltInExample {
    // Floating windows: the look that goes with the example's own (a
    // border a little thinner, less of a glow).
    config.desktop.appearance.floating =
        crate::settings::FloatingAppearance::softer_than_tiled(&config.desktop.appearance);
    BuiltInExample {
        id,
        name,
        description,
        config,
    }
}

fn workspace(name: &str, layout: LayoutType, default: bool) -> WorkspaceProfile {
    WorkspaceProfile {
        name: name.to_string(),
        layout,
        default,
        ..WorkspaceProfile::default()
    }
}

fn binding(name: &str, key: &str, command: DispatchCommand) -> BindingProfile {
    BindingProfile {
        name: name.to_string(),
        key: key.to_string(),
        command,
        ..BindingProfile::default()
    }
}

fn shifted_binding(name: &str, key: &str, command: DispatchCommand) -> BindingProfile {
    BindingProfile {
        shift: true,
        ..binding(name, key, command)
    }
}

fn predicate(kind: MatchKind, strategy: MatchStrategy, value: &str) -> RulePredicate {
    RulePredicate {
        extra: Default::default(),
        kind,
        strategy,
        value: value.to_string(),
        negate: false,
        case_sensitive: false,
    }
}

fn rule(
    name: &str,
    dynamic: bool,
    predicates: Vec<RulePredicate>,
    effects: Vec<RuleEffect>,
) -> WindowRuleProfile {
    WindowRuleProfile {
        extra: Default::default(),
        name: name.to_string(),
        enabled: true,
        dynamic,
        mode: PredicateMode::All,
        predicates,
        effects,
    }
}

fn startup_filter_graph() -> NodeGraph {
    let mut graph = NodeGraph::new();
    let startup = graph.add_node(NodeType::OnStartup, [80.0, 120.0]);
    let filters = graph.add_node(
        NodeType::ApplySystemFilters { enabled: true },
        [340.0, 120.0],
    );
    let log = graph.add_node(
        NodeType::LogMessage {
            message: "System filters and desktop profile are active".to_string(),
            level: LogLevel::Info,
        },
        [610.0, 120.0],
    );
    graph
        .add_connection_by_name(startup, "Event", filters, "Trigger")
        .expect("built-in startup graph must connect");
    graph
        .add_connection_by_name(filters, "Then", log, "Trigger")
        .expect("built-in startup graph must connect");
    graph
}

/// The graph of the showcase examples: new windows are tiled, and a hand
/// of keys does the things one does all day. Laid out for reading.
fn showcase_graph() -> NodeGraph {
    use crate::graph::WindowMode;
    let mut graph = NodeGraph::new();
    let at = [0.0, 0.0];
    let startup = graph.add_node(NodeType::OnStartup, at);
    let filters = graph.add_node(NodeType::ApplySystemFilters { enabled: true }, at);
    graph
        .add_connection_by_name(startup, "Event", filters, "Trigger")
        .expect("showcase graph must connect");

    let created = graph.add_node(NodeType::OnWindowCreated, at);
    let arrange = graph.add_node(
        NodeType::SetWindowArrangement {
            mode: WindowMode::Auto,
        },
        at,
    );
    graph
        .add_connection_by_name(created, "Event", arrange, "Trigger")
        .expect("showcase graph must connect");
    graph
        .add_connection_by_name(created, "Window", arrange, "Window")
        .expect("showcase graph must connect");

    let mut key = |name: &str, shift: bool, action: NodeType| {
        let on_key = graph.add_node(
            NodeType::OnKeyPress {
                key: name.to_string(),
                ctrl: true,
                alt: !shift,
                shift,
                win: false,
            },
            at,
        );
        let action = graph.add_node(action, at);
        graph
            .add_connection_by_name(on_key, "Event", action, "Trigger")
            .expect("showcase graph must connect");
    };
    key("E", false, NodeType::ToggleExposeMode);
    key("J", false, NodeType::FocusNextWindow);
    key("K", false, NodeType::FocusPreviousWindow);
    key("J", true, NodeType::SwapWithNext);
    key("K", true, NodeType::SwapWithPrevious);
    key("L", false, NodeType::NextWorkspace);
    key("H", false, NodeType::PreviousWorkspace);
    key("F", false, NodeType::ToggleFloating);
    key("Space", false, NodeType::CycleNextLayout);
    key("Space", true, NodeType::CyclePreviousLayout);
    crate::graph_layout::tidy(&mut graph);
    graph
}

/// The keys of the showcase examples and of the configuration gevrek starts
/// with. They follow the default keys of the niri compositor, with Super
/// (the Windows key) as the modifier, as far as gevrek has the action:
///
/// | Keys | What |
/// |---|---|
/// | Super+Shift+/ (and Super+F1) | the list of shortcuts |
/// | Super+O | overview |
/// | Super+Q | close the window |
/// | Super+T | a terminal |
/// | Super+arrows, Super+H J K L | focus the window in that direction |
/// | Super+Ctrl+the same | move the window in that direction |
/// | Super+Shift+the same | focus the monitor in that direction |
/// | Super+Shift+Ctrl+the same | move the window to that monitor |
/// | Super+PageDown / U, Super+PageUp / I, Super+wheel | next / previous workspace |
/// | Super+Ctrl+the same | move the window there and follow it |
/// | Super+1..9, Super+Ctrl+1..9 | workspace by number; move the window there |
/// | Super+Shift+wheel, Super+Ctrl+Shift+wheel | focus / move the window left and right |
/// | Super+[ ] , . W | tabs: join the neighbour, leave, tabs on or off |
/// | Super+F, Super+Shift+F, Super+M | one window fills the area; fullscreen; maximize |
/// | Super+C, Super+V | centre the window; floating or tiled |
/// | Super+A | a floating window always on top, or not (one a workspace) |
/// | Super+- =, Super+Shift+- = | narrower / wider; lower / taller |
///
/// | Super+D / Space, Super+S, Super+N | launcher, control centre, notifications |
/// | Super+Shift+D | show the desktop, or the windows again |
/// | Super+Alt+L, Super+Shift+P | lock the screen, displays off |
/// | Print, Ctrl+Print, Alt+Print | screenshot of a region, the screen, the window |
/// | the volume, media and brightness keys | what they say |
/// | Super+Shift+O | an empty workspace |
/// | Super+Shift+E | quit the session |
/// | Super+Escape | take the keys back from a program that asked for all |
/// | Super+Z, Super+Shift+Z | the zones above the windows, or back; hidden or shown |
///
/// The shell actions (launcher to brightness) are carried out by the shell
/// that is running; without one nothing happens. Not there: niri's column
/// widths. Any program can be put on a key with "Start a program" (`exec`).
pub fn standard_bindings() -> Vec<BindingProfile> {
    use crate::command::{Direction, MonitorTarget, WindowSelector};
    use crate::layout::{LayoutAxis, Sizing};
    use crate::settings::BindingDevice;

    // (ctrl, shift) with Super always held.
    fn chord(
        name: &str,
        key: &str,
        ctrl: bool,
        shift: bool,
        command: DispatchCommand,
    ) -> BindingProfile {
        BindingProfile {
            ctrl,
            shift,
            ..binding(name, key, command)
        }
    }
    fn wheel(
        name: &str,
        key: &str,
        ctrl: bool,
        shift: bool,
        command: DispatchCommand,
    ) -> BindingProfile {
        BindingProfile {
            device: BindingDevice::Mouse,
            ..chord(name, key, ctrl, shift, command)
        }
    }
    let switch =
        |selector: WorkspaceSelector| DispatchCommand::SwitchWorkspaceSelector { selector };
    let carry =
        |selector: WorkspaceSelector| DispatchCommand::MoveToWorkspaceAndFollow { selector };

    let mut list = vec![
        chord(
            "Show shortcuts",
            "SLASH",
            false,
            true,
            DispatchCommand::ShowShortcuts,
        ),
        binding("Show shortcuts", "F1", DispatchCommand::ShowShortcuts),
        binding("Overview", "O", DispatchCommand::ToggleExpose),
        chord(
            "Overview of every workspace",
            "O",
            true,
            false,
            DispatchCommand::WorkspaceOverview,
        ),
        binding("Close window", "Q", DispatchCommand::CloseActive),
        binding(
            "Terminal",
            "T",
            DispatchCommand::Exec {
                program: default_terminal().to_string(),
                args: Vec::new(),
            },
        ),
    ];

    // The shell: launcher, panels, lock, screenshots, the keys of the
    // keyboard for sound, brightness and media, and the end of the session.
    use crate::command::ShellAction;
    let shell = |action| DispatchCommand::Shell { action };
    // A key of its own, without Super: the media and brightness keys.
    let plain = |name: &str, key: &str, command: DispatchCommand| BindingProfile {
        win: false,
        ..binding(name, key, command)
    };
    list.extend([
        binding("Launcher", "D", shell(ShellAction::Launcher)),
        binding("Launcher", "SPACE", shell(ShellAction::Launcher)),
        // (Windows has it on Win+D; there D is the launcher's, as in niri.)
        chord(
            "Show the desktop",
            "D",
            false,
            true,
            DispatchCommand::ShowDesktop { on: None },
        ),
        binding("Control centre", "S", shell(ShellAction::ControlCenter)),
        binding("Notifications", "N", shell(ShellAction::NotificationCenter)),
        // Alt+Tab: held, with Tab on to the next window; let go, there.
        BindingProfile {
            alt: true,
            win: false,
            ..binding("Window switcher", "Tab", shell(ShellAction::WindowSwitcher))
        },
        BindingProfile {
            alt: true,
            ..binding("Lock the screen", "L", shell(ShellAction::Lock))
        },
        chord(
            "Displays off",
            "P",
            false,
            true,
            shell(ShellAction::DisplaysOff),
        ),
        chord(
            "Show an empty workspace",
            "O",
            false,
            true,
            switch(WorkspaceSelector::FirstEmpty),
        ),
        chord("Quit the session", "E", false, true, DispatchCommand::Quit),
        binding("Power menu", "X", shell(ShellAction::PowerMenu)),
        // The tutorial: begun or taken up again, and left (its card never
        // has the keyboard, so these are keys of the window manager).
        binding("The tutorial", "F2", shell(ShellAction::Tutorial)),
        shifted_binding("Leave the tutorial", "F2", shell(ShellAction::TutorialLeave)),
        plain(
            "Screenshot of a region",
            "PRINTSCREEN",
            shell(ShellAction::ScreenshotRegion),
        ),
        BindingProfile {
            ctrl: true,
            ..plain(
                "Screenshot of the screen",
                "PRINTSCREEN",
                shell(ShellAction::ScreenshotScreen),
            )
        },
        BindingProfile {
            alt: true,
            ..plain(
                "Screenshot of the window",
                "PRINTSCREEN",
                shell(ShellAction::ScreenshotWindow),
            )
        },
        plain("Volume up", "VOLUME_UP", shell(ShellAction::VolumeUp)),
        plain("Volume down", "VOLUME_DOWN", shell(ShellAction::VolumeDown)),
        plain("Mute", "VOLUME_MUTE", shell(ShellAction::VolumeMute)),
        plain(
            "Mute the microphone",
            "MIC_MUTE",
            shell(ShellAction::MicMute),
        ),
        plain(
            "Play / pause",
            "MEDIA_PLAY_PAUSE",
            shell(ShellAction::MediaPlayPause),
        ),
        plain("Stop playing", "MEDIA_STOP", shell(ShellAction::MediaStop)),
        plain(
            "Previous track",
            "MEDIA_PREVIOUS",
            shell(ShellAction::MediaPrevious),
        ),
        plain("Next track", "MEDIA_NEXT", shell(ShellAction::MediaNext)),
        plain(
            "Brightness up",
            "BRIGHTNESS_UP",
            shell(ShellAction::BrightnessUp),
        ),
        plain(
            "Brightness down",
            "BRIGHTNESS_DOWN",
            shell(ShellAction::BrightnessDown),
        ),
    ]);

    // The zones on the desktop (niri has none; Z is free in its keys and
    // in ours). Pressed, they are above the windows; pressed again (or
    // Escape, or opening something), back below. With Shift they are
    // hidden, and shown again. A press each, and nothing on the key's
    // release: a release is only heard as the same key while the
    // modifiers are still held, so "above while held" would stay above
    // for whoever lets go of Super first. Carried out by the shell that
    // runs the zones; without one, or with the zones switched off,
    // nothing happens.
    let zones = |action: serde_json::Value| DispatchCommand::Zones { action };
    list.extend([
        binding(
            "Zones above the windows, or back",
            "Z",
            zones(serde_json::json!({"command": "toggle_peek"})),
        ),
        chord(
            "Hide or show the zones",
            "Z",
            false,
            true,
            zones(serde_json::json!({"command": "quick_hide"})),
        ),
    ]);

    // Directions: the arrows and H J K L.
    let directions = [
        (
            "left",
            Direction::Left,
            ["LEFT", "H"],
            MonitorTarget::Previous,
        ),
        ("down", Direction::Down, ["DOWN", "J"], MonitorTarget::Next),
        ("up", Direction::Up, ["UP", "K"], MonitorTarget::Previous),
        (
            "right",
            Direction::Right,
            ["RIGHT", "L"],
            MonitorTarget::Next,
        ),
    ];
    for (word, direction, keys, monitor) in directions {
        for key in keys {
            list.push(binding(
                &format!("Focus {word}"),
                key,
                DispatchCommand::FocusDirection { direction },
            ));
            list.push(chord(
                &format!("Move window {word}"),
                key,
                true,
                false,
                DispatchCommand::SwapDirection { direction },
            ));
            list.push(chord(
                &format!("Focus monitor {word}"),
                key,
                false,
                true,
                DispatchCommand::FocusMonitor {
                    target: monitor.clone(),
                },
            ));
            list.push(chord(
                &format!("Move window to monitor {word}"),
                key,
                true,
                true,
                DispatchCommand::MoveWindowToMonitorAndFollow {
                    window: WindowSelector::Active,
                    target: monitor.clone(),
                },
            ));
        }
    }

    // Workspaces: next and previous, by number, with the wheel.
    for (key, selector, word) in [
        ("PAGEDOWN", WorkspaceSelector::Next, "Next"),
        ("U", WorkspaceSelector::Next, "Next"),
        ("PAGEUP", WorkspaceSelector::Previous, "Previous"),
        ("I", WorkspaceSelector::Previous, "Previous"),
    ] {
        list.push(binding(
            &format!("{word} workspace"),
            key,
            switch(selector.clone()),
        ));
        list.push(chord(
            &format!("Move window to {} workspace", word.to_lowercase()),
            key,
            true,
            false,
            carry(selector),
        ));
    }
    for (key, selector, word) in [
        ("WHEEL_DOWN", WorkspaceSelector::Next, "Next"),
        ("WHEEL_UP", WorkspaceSelector::Previous, "Previous"),
    ] {
        list.push(wheel(
            &format!("{word} workspace"),
            key,
            false,
            false,
            switch(selector.clone()),
        ));
        list.push(wheel(
            &format!("Move window to {} workspace", word.to_lowercase()),
            key,
            true,
            false,
            carry(selector),
        ));
    }
    for (key, direction, word) in [
        ("WHEEL_DOWN", Direction::Right, "right"),
        ("WHEEL_UP", Direction::Left, "left"),
    ] {
        list.push(wheel(
            &format!("Focus {word}"),
            key,
            false,
            true,
            DispatchCommand::FocusDirection { direction },
        ));
        list.push(wheel(
            &format!("Move window {word}"),
            key,
            true,
            true,
            DispatchCommand::SwapDirection { direction },
        ));
    }
    // Along the strip of the scrolling layout (elsewhere it does nothing).
    for (key, pixels, word) in [("WHEEL_DOWN", 240, "right"), ("WHEEL_UP", -240, "left")] {
        list.push(BindingProfile {
            alt: true,
            ..wheel(
                &format!("Scroll the strip {word}"),
                key,
                false,
                false,
                DispatchCommand::ScrollView { pixels },
            )
        });
    }
    for name in ["1", "2", "3", "4", "5", "6", "7", "8", "9"] {
        list.push(binding(
            &format!("Workspace {name}"),
            name,
            switch(WorkspaceSelector::Name(name.to_string())),
        ));
        list.push(chord(
            &format!("Move window to workspace {name}"),
            name,
            true,
            false,
            carry(WorkspaceSelector::Name(name.to_string())),
        ));
        // The window goes, and the workspace shown stays.
        list.push(BindingProfile {
            alt: true,
            ..binding(
                &format!("Send window to workspace {name}"),
                name,
                DispatchCommand::MoveToWorkspaceSelector {
                    selector: WorkspaceSelector::Name(name.to_string()),
                },
            )
        });
    }

    // Tabs (niri: windows of one column).
    list.extend([
        binding(
            "Join the window before as tabs",
            "LEFTBRACKET",
            DispatchCommand::MergeGroupPrevious,
        ),
        binding(
            "Join the window after as tabs",
            "RIGHTBRACKET",
            DispatchCommand::MergeGroupNext,
        ),
        binding(
            "Join the window after as tabs",
            "COMMA",
            DispatchCommand::MergeGroupNext,
        ),
        binding("Leave the tabs", "PERIOD", DispatchCommand::UngroupActive),
        binding("Tabs on or off", "W", DispatchCommand::ToggleGroup),
        // Among the windows that share a tile: the next, the one before.
        binding("Next tab", "TAB", DispatchCommand::CycleGroupNext),
        chord(
            "Previous tab",
            "TAB",
            false,
            true,
            DispatchCommand::CycleGroupPrevious,
        ),
    ]);

    // Sizes and states.
    list.extend([
        binding(
            "One window fills the area",
            "F",
            DispatchCommand::ToggleMonocle,
        ),
        chord(
            "Fullscreen",
            "F",
            false,
            true,
            DispatchCommand::ToggleFullscreen,
        ),
        binding("Maximize", "M", DispatchCommand::ToggleMaximized),
        binding("Centre window", "C", DispatchCommand::CenterActive),
        binding("Floating or tiled", "V", DispatchCommand::ToggleFloating),
        chord(
            "Floating windows out of the way, or back",
            "V",
            false,
            true,
            DispatchCommand::ToggleFloatingAside,
        ),
        // (A floating window only, one on each workspace.)
        binding("Always on top", "A", DispatchCommand::TogglePinned),
        binding(
            "Narrower",
            "MINUS",
            DispatchCommand::ResizeAxis {
                axis: LayoutAxis::Horizontal,
                sizing: Sizing::Decrease,
            },
        ),
        binding(
            "Wider",
            "EQUALS",
            DispatchCommand::ResizeAxis {
                axis: LayoutAxis::Horizontal,
                sizing: Sizing::Increase,
            },
        ),
        chord(
            "Lower",
            "MINUS",
            false,
            true,
            DispatchCommand::ResizeAxis {
                axis: LayoutAxis::Vertical,
                sizing: Sizing::Decrease,
            },
        ),
        chord(
            "Taller",
            "EQUALS",
            false,
            true,
            DispatchCommand::ResizeAxis {
                axis: LayoutAxis::Vertical,
                sizing: Sizing::Increase,
            },
        ),
    ]);
    list
}

/// The terminal Super+T starts: Windows Terminal on Windows, kitty elsewhere.
pub fn default_terminal() -> &'static str {
    if cfg!(windows) {
        "wt.exe"
    } else {
        "kitty"
    }
}

/// The configuration gevrek starts with when it has no configuration file:
/// the example Aurora.
pub fn starting_configuration() -> Config {
    example_aurora().config
}

/// What every showcase example has: the graph, the keys of
/// [`standard_bindings`], and workspaces `1` to `9` (5 to 9 only while used).
fn showcase_base(layout: LayoutType, gaps_in: i32, gaps_out: i32) -> Config {
    let mut config = Config::new();
    config.node_graph = showcase_graph();
    config.desktop.workspaces = ["1", "2", "3", "4"]
        .iter()
        .enumerate()
        .map(|(index, name)| WorkspaceProfile {
            gaps_in,
            gaps_out,
            // The look is the configuration's, not each workspace's own.
            appearance_override: false,
            ..workspace(name, layout, index == 0)
        })
        .collect();
    // Workspaces 5 to 9 come when they are first used and go when they are
    // left empty.
    for name in ["5", "6", "7", "8", "9"] {
        config.desktop.workspaces.push(WorkspaceProfile {
            gaps_in,
            gaps_out,
            appearance_override: false,
            persistent: false,
            ..workspace(name, layout, false)
        });
    }
    config.desktop.bindings = standard_bindings();
    // Windows joined as tabs (Super+[ and Super+]) show their strip of
    // tabs: without it they look like one window, and nothing says that
    // others lie behind it.
    config.desktop.appearance.stackbar.enabled = true;
    config
}

fn animate(config: &mut Config, ms: u32, curve: AnimationCurve) {
    let animations = &mut config.desktop.animations;
    animations.enabled = true;
    for channel in [
        &mut animations.layout,
        &mut animations.workspace,
        &mut animations.border,
        &mut animations.window,
    ] {
        channel.enabled = true;
        channel.duration_ms = ms;
        channel.curve = curve;
    }
}

fn example_aurora() -> BuiltInExample {
    use crate::settings::WindowTransitionStyle;
    let mut config = showcase_base(LayoutType::Dwindle, 10, 16);
    animate(&mut config, 220, AnimationCurve::CubicOut);
    config.desktop.animations.window.curve = AnimationCurve::BackOut;
    config.desktop.animations.window_enter_style = WindowTransitionStyle::Fade;
    config.desktop.animations.window_exit_style = WindowTransitionStyle::Fade;
    config.desktop.animations.workspace_style = crate::settings::WorkspaceTransitionStyle::Fade;
    let appearance = &mut config.desktop.appearance;
    appearance.border_width = 3;
    appearance.active_border_color = (181, 0, 125);
    appearance.inactive_border_color = (57, 44, 72);
    appearance.inactive_border_alpha = 200;
    let effects = &mut appearance.effects;
    effects.corner_radius.0 = 12;
    effects.gradient.enabled = true;
    effects.gradient.active_end = (168, 96, 255);
    effects.gradient.inactive_end = (30, 34, 48);
    effects.gradient.angle = 30.0;
    effects.gradient.spin_seconds = 8.0;
    effects.glow.enabled = true;
    effects.glow.radius = 22;
    effects.glow.opacity = 150;
    effects.shadow.enabled = true;
    effects.shadow.radius = 30;
    effects.shadow.offset_y = 10;
    effects.shadow.opacity = 150;
    effects.shadow.inactive_opacity = 100;
    let bar = &mut config.desktop.shell.panel;
    bar.background = (14, 17, 26);
    bar.opacity = 235;
    bar.margin = 0;
    bar.corner_radius = 0;
    bar.tasks_of_all_workspaces = true;
    // (As the person has it, 2026-11-17, and so what gevrek starts with:
    // a panel from edge to edge with the open windows of every
    // workspace at its right before the icons and the clock last; a
    // border from magenta to violet; windows and workspaces that fade;
    // the colours of a palette where none is set by hand, with the
    // accent the wallpaper's own; a night light that is less warm. The
    // person's own workspaces, displays and window rules are theirs and
    // are not here.)
    {
        use crate::settings::PanelModule;
        bar.left = vec![
            PanelModule::Launcher,
            PanelModule::Workspaces,
            PanelModule::Layout,
            PanelModule::Title,
            PanelModule::Spacer,
        ];
        bar.centre = Vec::new();
        bar.right = vec![
            PanelModule::Taskbar,
            PanelModule::Tray,
            PanelModule::Network,
            PanelModule::Sound,
            PanelModule::Battery,
            PanelModule::Clock,
        ];
    }
    config.desktop.appearance.theme.palette = Some("Default Dark".to_string());
    config.desktop.appearance.theme.accent = Some(crate::theme::WALLPAPER_ACCENT.to_string());
    config.desktop.night_light.temperature = 5000;
    example(
        "aurora",
        "Aurora",
        "The full look: a gradient border that slowly turns around the focused window, a glow in its colour, \
         soft shadows, generous gaps, windows and workspaces that fade in and out. Dwindle tiling, four workspaces, \
         the overview on Win+O (or Ctrl+Alt+E), the list of shortcuts on Win+F1 or Win+Shift+/.",
        config,
    )
}

fn example_focus_stage() -> BuiltInExample {
    use crate::settings::{PanelEdge, WindowTransitionStyle};
    let mut config = showcase_base(LayoutType::CenteredMaster, 12, 20);
    animate(&mut config, 260, AnimationCurve::EaseInOut);
    config.desktop.animations.window_enter_style = WindowTransitionStyle::Fade;
    config.desktop.animations.window_exit_style = WindowTransitionStyle::Fade;
    for profile in &mut config.desktop.workspaces {
        profile.master_ratio = 0.56;
    }
    let appearance = &mut config.desktop.appearance;
    appearance.border_width = 2;
    appearance.active_border_color = (236, 240, 248);
    appearance.inactive_border_color = (60, 66, 82);
    appearance.inactive_border_alpha = 120;
    appearance.dim_inactive = true;
    appearance.inactive_window_opacity = 235;
    let effects = &mut appearance.effects;
    effects.corner_radius.0 = 10;
    effects.tint.enabled = true;
    effects.tint.color = (8, 10, 22);
    effects.tint.inactive_opacity = 96;
    effects.shadow.enabled = true;
    effects.shadow.radius = 40;
    effects.shadow.offset_y = 14;
    effects.shadow.opacity = 190;
    effects.shadow.inactive_opacity = 40;
    let bar = &mut config.desktop.shell.panel;
    bar.edge = PanelEdge::Bottom;
    bar.height = 30;
    bar.margin = 0;
    bar.corner_radius = 0;
    bar.opacity = 255;
    bar.background = (10, 12, 18);
    bar.accent = Some((236, 240, 248));
    bar.clock_format = "%H:%M".to_string();
    example(
        "focus-stage",
        "Focus Stage",
        "One window in the light: the others are veiled with a dark tint and stand back, the focused one \
         casts a deep shadow. Centered master tiling, slow soft animations, a plain bar at the bottom.",
        config,
    )
}

fn example_neon_grid() -> BuiltInExample {
    use crate::settings::WindowTransitionStyle;
    let mut config = showcase_base(LayoutType::Grid, 6, 10);
    animate(&mut config, 110, AnimationCurve::EaseOut);
    config.desktop.animations.window_enter_style = WindowTransitionStyle::Slide;
    config.desktop.animations.window_exit_style = WindowTransitionStyle::Scale;
    let appearance = &mut config.desktop.appearance;
    appearance.border_width = 3;
    appearance.active_border_color = (255, 46, 136);
    appearance.inactive_border_color = (52, 34, 86);
    appearance.inactive_border_alpha = 255;
    let effects = &mut appearance.effects;
    effects.corner_radius.0 = 3;
    effects.gradient.enabled = true;
    effects.gradient.active_end = (255, 214, 10);
    effects.gradient.inactive_end = (20, 60, 110);
    effects.gradient.angle = 135.0;
    effects.glow.enabled = true;
    effects.glow.radius = 30;
    effects.glow.opacity = 210;
    effects.glow.inactive_opacity = 40;
    effects.glow.color = Some((255, 46, 136));
    let bar = &mut config.desktop.shell.panel;
    bar.background = (12, 6, 24);
    bar.text = (236, 226, 255);
    bar.accent = Some((255, 214, 10));
    bar.corner_radius = 3;
    bar.margin = 6;
    bar.set_shown(crate::settings::PanelModule::Stats, true);
    example(
        "neon-grid",
        "Neon Grid",
        "Loud and quick: a pink-to-yellow border with a strong glow, sharp corners, a grid of windows, \
         short snappy animations, windows that slide in. The bar shows the numbers too.",
        config,
    )
}

fn example_paper() -> BuiltInExample {
    use crate::settings::WindowTransitionStyle;
    let mut config = showcase_base(LayoutType::Columns, 14, 22);
    animate(&mut config, 180, AnimationCurve::EaseOut);
    config.desktop.animations.window_enter_style = WindowTransitionStyle::Fade;
    config.desktop.animations.window_exit_style = WindowTransitionStyle::Fade;
    let appearance = &mut config.desktop.appearance;
    appearance.border_width = 1;
    appearance.active_border_color = (120, 132, 150);
    appearance.inactive_border_color = (200, 205, 214);
    appearance.inactive_border_alpha = 255;
    let effects = &mut appearance.effects;
    effects.corner_radius.0 = 9;
    effects.shadow.enabled = true;
    effects.shadow.radius = 36;
    effects.shadow.offset_y = 12;
    effects.shadow.color = (40, 50, 70);
    effects.shadow.opacity = 120;
    effects.shadow.inactive_opacity = 60;
    let bar = &mut config.desktop.shell.panel;
    bar.background = (244, 245, 248);
    bar.text = (40, 46, 58);
    bar.accent = Some((58, 110, 220));
    bar.opacity = 245;
    bar.margin = 10;
    bar.corner_radius = 14;
    bar.height = 38;
    example(
        "paper",
        "Paper",
        "Quiet and light: hairline borders, large soft shadows that lift the focused window off the desk, \
         wide gaps, a light bar. Columns, gentle fades. Made for a light wallpaper.",
        config,
    )
}

fn example_dwindle_desktop() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![
        WorkspaceProfile {
            dwindle_direction: DwindleDirection::Alternate,
            dwindle_preserve_split: true,
            gaps_in: 7,
            gaps_out: 12,
            ..workspace("main", LayoutType::Dwindle, true)
        },
        workspace("web", LayoutType::Dwindle, false),
        workspace("chat", LayoutType::Grid, false),
    ];
    config.desktop.bindings = vec![
        binding(
            "Focus left",
            "H",
            DispatchCommand::FocusDirection {
                direction: Direction::Left,
            },
        ),
        binding(
            "Focus down",
            "J",
            DispatchCommand::FocusDirection {
                direction: Direction::Down,
            },
        ),
        binding(
            "Focus up",
            "K",
            DispatchCommand::FocusDirection {
                direction: Direction::Up,
            },
        ),
        binding(
            "Focus right",
            "L",
            DispatchCommand::FocusDirection {
                direction: Direction::Right,
            },
        ),
        binding("Toggle pseudotile", "P", DispatchCommand::TogglePseudotile),
        binding(
            "Cycle split direction",
            "D",
            DispatchCommand::CycleDwindleDirection,
        ),
        binding("Close active", "Q", DispatchCommand::CloseActive),
    ];
    example(
        "dwindle-desktop",
        "Dwindle Desktop",
        "Dwindle workspaces, directional focus, pseudotiling, split controls, gaps, and startup filters.",
        config,
    )
}

fn example_master_productivity() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![
        WorkspaceProfile {
            master_ratio: 0.62,
            master_count: 1,
            master_position: MasterPosition::Right,
            new_window_placement: NewWindowPlacement::AfterFocused,
            ..workspace("code", LayoutType::MasterStack, true)
        },
        workspace("web", LayoutType::CenteredMaster, false),
        workspace("chat", LayoutType::Columns, false),
        WorkspaceProfile {
            persistent: false,
            on_created_empty: Some(LaunchProfile {
                program: "notepad.exe".to_string(),
                environment: [("GEVREK_WORKSPACE".to_string(), "notes".to_string())]
                    .into_iter()
                    .collect(),
                ..LaunchProfile::default()
            }),
            ..workspace("notes", LayoutType::Monocle, false)
        },
    ];
    config.desktop.bindings = vec![
        binding(
            "Open code workspace",
            "1",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("code".to_string()),
            },
        ),
        binding(
            "Open web workspace",
            "2",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("web".to_string()),
            },
        ),
        binding(
            "Open chat workspace",
            "3",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("chat".to_string()),
            },
        ),
        binding(
            "Open ephemeral notes workspace",
            "4",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Name("notes".to_string()),
            },
        ),
        binding(
            "Grow master",
            "L",
            DispatchCommand::AdjustLayoutRatio { delta: 0.05 },
        ),
        binding(
            "Shrink master",
            "H",
            DispatchCommand::AdjustLayoutRatio { delta: -0.05 },
        ),
        binding("Promote to master", "M", DispatchCommand::PromoteToMaster),
    ];
    config.desktop.window_rules = vec![
        rule(
            "Route editors to code",
            false,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(code|devenv|idea64)\.exe",
            )],
            vec![RuleEffect::MoveToWorkspace {
                monitor_index: None,
                workspace_index: 0,
            }],
        ),
        rule(
            "Route browsers to web",
            false,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(chrome|msedge|firefox)\.exe",
            )],
            vec![RuleEffect::MoveToWorkspace {
                monitor_index: None,
                workspace_index: 1,
            }],
        ),
    ];
    example(
        "master-productivity",
        "Master-stack Productivity",
        "Named workspaces, app routing, master controls, plus an ephemeral notes workspace that launches Notepad when created empty.",
        config,
    )
}

fn example_multi_monitor() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.monitors = vec![
        MonitorProfile {
            monitor: MonitorSelector::Any,
            reserved: ReservedArea {
                top: 36,
                ..ReservedArea::default()
            },
            ..MonitorProfile::default()
        },
        MonitorProfile {
            monitor: MonitorSelector::Index(1),
            reserved: ReservedArea {
                top: 36,
                right: 280,
                ..ReservedArea::default()
            },
            ..MonitorProfile::default()
        },
    ];
    config.desktop.workspaces = vec![
        workspace("primary", LayoutType::CenteredMaster, true),
        WorkspaceProfile {
            monitor: MonitorSelector::Index(1),
            ..workspace("reference", LayoutType::Columns, true)
        },
        WorkspaceProfile {
            monitor: MonitorSelector::Index(1),
            ..workspace("communications", LayoutType::Grid, false)
        },
    ];
    config.desktop.window_rules = vec![
        rule(
            "Editors on primary",
            false,
            vec![predicate(MatchKind::Exe, MatchStrategy::Contains, "code")],
            vec![RuleEffect::MoveToWorkspace {
                monitor_index: Some(0),
                workspace_index: 0,
            }],
        ),
        rule(
            "Browsers on reference monitor",
            false,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(chrome|msedge|firefox)\.exe",
            )],
            vec![RuleEffect::MoveToWorkspace {
                monitor_index: Some(1),
                workspace_index: 0,
            }],
        ),
        rule(
            "Chat on secondary",
            false,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(slack|teams|discord)\.exe",
            )],
            vec![RuleEffect::MoveToWorkspace {
                monitor_index: Some(1),
                workspace_index: 1,
            }],
        ),
    ];
    config.desktop.bindings = vec![
        binding(
            "Previous visited workspace",
            "Tab",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::PreviousVisited,
            },
        ),
        binding(
            "Move window silently to first empty",
            "E",
            DispatchCommand::MoveToWorkspaceSelector {
                selector: WorkspaceSelector::FirstEmpty,
            },
        ),
        BindingProfile {
            ctrl: true,
            ..binding(
                "Move window and follow next workspace",
                "E",
                DispatchCommand::MoveToWorkspaceAndFollow {
                    selector: WorkspaceSelector::Next,
                },
            )
        },
        BindingProfile {
            ctrl: true,
            shift: true,
            ..binding(
                "Rename active workspace",
                "R",
                DispatchCommand::RenameWorkspace {
                    selector: WorkspaceSelector::Relative(0),
                    name: "renamed".to_string(),
                },
            )
        },
        binding(
            "Focus next monitor",
            "M",
            DispatchCommand::FocusMonitor {
                target: MonitorTarget::Next,
            },
        ),
        shifted_binding(
            "Move workspace to next monitor",
            "M",
            DispatchCommand::MoveWorkspaceToMonitor {
                target: MonitorTarget::Next,
            },
        ),
        BindingProfile {
            ctrl: true,
            ..binding(
                "Swap active workspaces",
                "M",
                DispatchCommand::SwapActiveWorkspaces {
                    target: MonitorTarget::Next,
                },
            )
        },
    ];
    example(
        "multi-monitor-workstation",
        "Multi-monitor Workstation",
        "Per-monitor workspaces and deterministic editor, browser, and communications routing.",
        config,
    )
}

fn example_dynamic_window_studio() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![workspace("studio", LayoutType::Dwindle, true)];
    config.desktop.window_rules = vec![
        rule(
            "Center dialogs",
            true,
            vec![predicate(
                MatchKind::Class,
                MatchStrategy::Regex,
                r"(?i)(dialog|#32770)",
            )],
            vec![
                RuleEffect::Float,
                RuleEffect::Resize {
                    width: RuleMeasure::Percent(48),
                    height: RuleMeasure::Percent(58),
                },
                RuleEffect::MinSize {
                    width: RuleMeasure::Pixels(360),
                    height: RuleMeasure::Pixels(240),
                },
                RuleEffect::MaxSize {
                    width: RuleMeasure::Percent(85),
                    height: RuleMeasure::Percent(85),
                },
                RuleEffect::Center,
                RuleEffect::Corners(WindowCorner::Round),
                RuleEffect::Backdrop(SystemBackdrop::Mica),
            ],
        ),
        rule(
            "Picture in picture",
            true,
            vec![predicate(
                MatchKind::Title,
                MatchStrategy::Contains,
                "Picture-in-Picture",
            )],
            vec![
                RuleEffect::Float,
                RuleEffect::AlwaysOnTop(true),
                RuleEffect::NoInitialFocus(true),
                RuleEffect::NoFollowMouse(true),
                RuleEffect::MinSize {
                    width: RuleMeasure::Pixels(320),
                    height: RuleMeasure::Pixels(180),
                },
                RuleEffect::MaxSize {
                    width: RuleMeasure::Percent(50),
                    height: RuleMeasure::Percent(50),
                },
                RuleEffect::KeepAspectRatio(true),
                RuleEffect::Resize {
                    width: RuleMeasure::Percent(28),
                    height: RuleMeasure::Percent(28),
                },
                RuleEffect::Move {
                    x: RuleMeasure::Percent(70),
                    y: RuleMeasure::Percent(66),
                },
                RuleEffect::Opacity(242),
                RuleEffect::BorderColor(RuleColor::Rgb {
                    r: 255,
                    g: 166,
                    b: 0,
                }),
            ],
        ),
        rule(
            "Keep credential prompts focused",
            true,
            vec![predicate(
                MatchKind::Class,
                MatchStrategy::Regex,
                r"(?i)(credential|pinentry|security prompt)",
            )],
            vec![
                RuleEffect::Float,
                RuleEffect::Center,
                RuleEffect::StayFocused(true),
                RuleEffect::PersistentSize(true),
            ],
        ),
        rule(
            "Passive notifications",
            true,
            vec![predicate(
                MatchKind::Class,
                MatchStrategy::Regex,
                r"(?i)(notification|toast)",
            )],
            vec![
                RuleEffect::Float,
                RuleEffect::AlwaysOnTop(true),
                RuleEffect::NoFocus(true),
            ],
        ),
    ];
    config.desktop.bindings = vec![
        binding(
            "Toggle PiP rule",
            "P",
            DispatchCommand::ToggleRule {
                name: "Picture in picture".to_string(),
            },
        ),
        binding("Toggle floating", "Space", DispatchCommand::ToggleFloating),
        binding(
            "Float active window",
            "F",
            DispatchCommand::SetFloating { enabled: true },
        ),
        binding(
            "Tile active window",
            "T",
            DispatchCommand::SetFloating { enabled: false },
        ),
        binding("Center active window", "C", DispatchCommand::CenterActive),
        binding("Raise active window", "R", DispatchCommand::RaiseActive),
        binding("Lower active window", "L", DispatchCommand::LowerActive),
        binding(
            "Pin active window",
            "I",
            DispatchCommand::SetPinned { enabled: true },
        ),
        binding(
            "Unpin active window",
            "O",
            DispatchCommand::SetPinned { enabled: false },
        ),
    ];
    example(
        "dynamic-window-studio",
        "Dynamic Geometry & Decorations",
        "Reversible dialog, credential-prompt, picture-in-picture, and passive-notification rules with complete focus policy, persistent and constrained floating sizes, percent sizing, placement, opacity, borders, corners, and backdrop.",
        config,
    )
}

fn example_tagged_workflow() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![
        workspace("work", LayoutType::Dwindle, true),
        workspace("communication", LayoutType::Columns, false),
    ];
    config.desktop.window_rules = vec![
        rule(
            "Tag communications apps",
            false,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(slack|teams|discord)\.exe",
            )],
            vec![RuleEffect::AddTag("communication".to_string())],
        ),
        rule(
            "Route communication tag",
            true,
            vec![predicate(
                MatchKind::Tag,
                MatchStrategy::Equals,
                "communication",
            )],
            vec![
                RuleEffect::MoveToWorkspace {
                    monitor_index: None,
                    workspace_index: 1,
                },
                RuleEffect::CaptionColor(RuleColor::Rgb {
                    r: 45,
                    g: 34,
                    b: 70,
                }),
            ],
        ),
        rule(
            "Urgent tag floats",
            true,
            vec![predicate(MatchKind::Tag, MatchStrategy::Equals, "urgent")],
            vec![
                RuleEffect::Float,
                RuleEffect::AlwaysOnTop(true),
                RuleEffect::BorderColor(RuleColor::Rgb {
                    r: 255,
                    g: 60,
                    b: 80,
                }),
            ],
        ),
    ];
    config.desktop.bindings = vec![
        binding(
            "Toggle urgent tag",
            "U",
            DispatchCommand::ToggleWindowTag {
                tag: "urgent".to_string(),
            },
        ),
        binding(
            "Toggle communication tag",
            "C",
            DispatchCommand::ToggleWindowTag {
                tag: "communication".to_string(),
            },
        ),
        binding("Clear manual tags", "X", DispatchCommand::ClearWindowTags),
        binding(
            "Focus communication window",
            "F",
            DispatchCommand::FocusWindow {
                selector: WindowSelector::Tag("communication".to_string()),
            },
        ),
        binding("Focus last window", "B", DispatchCommand::FocusLast),
        binding(
            "Float communication window",
            "V",
            DispatchCommand::SetWindowFloating {
                selector: WindowSelector::Tag("communication".to_string()),
                enabled: true,
            },
        ),
        binding(
            "Center urgent window",
            "M",
            DispatchCommand::CenterWindow {
                selector: WindowSelector::Tag("urgent".to_string()),
            },
        ),
        binding(
            "Pin urgent window",
            "P",
            DispatchCommand::SetWindowPinned {
                selector: WindowSelector::Tag("urgent".to_string()),
                enabled: true,
            },
        ),
        binding(
            "Fullscreen communication window",
            "H",
            DispatchCommand::ToggleWindowFullscreen {
                selector: WindowSelector::Tag("communication".to_string()),
            },
        ),
        binding(
            "Pseudotile terminal",
            "Y",
            DispatchCommand::ToggleWindowPseudotile {
                selector: WindowSelector::InitialClass(
                    "re:^(WindowsTerminal|CASCADIA_HOSTING_WINDOW_CLASS)$".to_string(),
                ),
            },
        ),
        binding(
            "Mark communication window urgent",
            "T",
            DispatchCommand::ToggleSelectedWindowTag {
                selector: WindowSelector::Tag("communication".to_string()),
                tag: "urgent".to_string(),
            },
        ),
        binding(
            "Bring communication window to work",
            "G",
            DispatchCommand::MoveWindowToWorkspaceAndFollow {
                window: WindowSelector::Tag("communication".to_string()),
                workspace: WorkspaceSelector::Name("work".to_string()),
            },
        ),
        binding(
            "Toggle communication group",
            "N",
            DispatchCommand::ToggleWindowGroup {
                selector: WindowSelector::Tag("communication".to_string()),
            },
        ),
        binding(
            "Next communication group tab",
            "I",
            DispatchCommand::CycleWindowGroup {
                selector: WindowSelector::Tag("communication".to_string()),
                forward: true,
            },
        ),
        binding(
            "Focus first communication group tab",
            "1",
            DispatchCommand::FocusWindowGroupIndex {
                selector: WindowSelector::Tag("communication".to_string()),
                index: 0,
            },
        ),
        binding(
            "Lock communication group",
            "O",
            DispatchCommand::ToggleWindowGroupLock {
                selector: WindowSelector::Tag("communication".to_string()),
            },
        ),
        binding(
            "Move communication tab forward",
            "R",
            DispatchCommand::MoveWindowInGroup {
                selector: WindowSelector::Tag("communication".to_string()),
                forward: true,
            },
        ),
    ];
    example(
        "tagged-workflow",
        "Tagged Workflow",
        "Static app tags feed dynamic routing and urgent-window decoration rules, with tag toggles on hotkeys.",
        config,
    )
}

fn example_scratchpad_terminal() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![workspace("main", LayoutType::Dwindle, true)];
    config.desktop.scratchpads = vec![ScratchpadProfile {
        id: "terminal".to_string(),
        match_kind: MatchKind::Exe,
        match_strategy: MatchStrategy::Regex,
        pattern: r"(wezterm|windowsTerminal|alacritty)\.exe".to_string(),
        launch: Some(LaunchProfile {
            program: "wt.exe".to_string(),
            ..LaunchProfile::default()
        }),
        monitor: MonitorSelector::Any,
        width_percent: 78,
        height_percent: 62,
        position: ScratchpadPosition::Top,
        ..ScratchpadProfile::default()
    }];
    config.desktop.bindings = vec![binding(
        "Toggle terminal scratchpad",
        "Oem3",
        DispatchCommand::ToggleScratchpad {
            id: "terminal".to_string(),
        },
    )];
    config.desktop.window_rules = vec![rule(
        "Scratchpad terminal styling",
        true,
        vec![predicate(
            MatchKind::Exe,
            MatchStrategy::Regex,
            r"(wezterm|windowsTerminal|alacritty)\.exe",
        )],
        vec![
            RuleEffect::Float,
            RuleEffect::Corners(WindowCorner::RoundSmall),
            RuleEffect::Backdrop(SystemBackdrop::Acrylic),
            RuleEffect::BorderColor(RuleColor::Rgb {
                r: 120,
                g: 90,
                b: 255,
            }),
        ],
    )];
    example(
        "scratchpad-terminal",
        "Dropdown Terminal Scratchpad",
        "A declarative Win+` special workspace launches, adopts, positions, and toggles a styled terminal on the active monitor.",
        config,
    )
}

fn example_presentation_mode() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![
        workspace("present", LayoutType::Monocle, true),
        workspace("notes", LayoutType::Rows, false),
    ];
    config.desktop.window_rules = vec![
        rule(
            "Presentation fullscreen",
            true,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(powerpnt|soffice)\.exe",
            )],
            vec![
                RuleEffect::MoveToWorkspace {
                    monitor_index: None,
                    workspace_index: 0,
                },
                RuleEffect::Fullscreen,
                RuleEffect::BorderColor(RuleColor::Hidden),
                RuleEffect::Corners(WindowCorner::DoNotRound),
            ],
        ),
        rule(
            "Speaker notes",
            true,
            vec![predicate(
                MatchKind::Title,
                MatchStrategy::Contains,
                "Presenter View",
            )],
            vec![
                RuleEffect::Float,
                RuleEffect::MoveToWorkspace {
                    monitor_index: None,
                    workspace_index: 1,
                },
                RuleEffect::Resize {
                    width: RuleMeasure::Percent(85),
                    height: RuleMeasure::Percent(80),
                },
                RuleEffect::Center,
            ],
        ),
    ];
    config.desktop.bindings = vec![
        binding(
            "Toggle presentation rule",
            "F9",
            DispatchCommand::ToggleRule {
                name: "Presentation fullscreen".to_string(),
            },
        ),
        binding("Overview", "F10", DispatchCommand::ToggleExpose),
    ];
    example(
        "presentation-mode",
        "Presentation Mode",
        "Fullscreen slides, separate speaker notes, rule toggling, and overview controls.",
        config,
    )
}

fn example_gaming_mode() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.animations.enabled = false;
    config.desktop.workspaces = vec![
        workspace("game", LayoutType::Monocle, true),
        workspace("social", LayoutType::Grid, false),
    ];
    config.desktop.window_rules = vec![
        rule(
            "Fullscreen games",
            true,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(?i)(game|steam_app|valorant|eldenring).*\.exe",
            )],
            vec![
                RuleEffect::Fullscreen,
                RuleEffect::AlwaysOnTop(true),
                RuleEffect::BorderColor(RuleColor::Hidden),
                RuleEffect::Corners(WindowCorner::DoNotRound),
                RuleEffect::Backdrop(SystemBackdrop::None),
                RuleEffect::AddTag("game".to_string()),
            ],
        ),
        rule(
            "Social apps off the game workspace",
            false,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(discord|steamwebhelper)\.exe",
            )],
            vec![RuleEffect::MoveToWorkspace {
                monitor_index: None,
                workspace_index: 1,
            }],
        ),
    ];
    config.desktop.bindings = vec![
        binding(
            "Toggle game rule",
            "G",
            DispatchCommand::ToggleRule {
                name: "Fullscreen games".to_string(),
            },
        ),
        binding(
            "Back to previous workspace",
            "Escape",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::PreviousVisited,
            },
        ),
    ];
    example(
        "gaming-mode",
        "Low-latency Gaming Mode",
        "Animation-free monocle gaming, fullscreen game rules, hidden decorations, and social-app isolation.",
        config,
    )
}

fn example_media_lounge() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![
        workspace("video", LayoutType::Monocle, true),
        workspace("music", LayoutType::CenteredMaster, false),
    ];
    config.desktop.window_rules = vec![
        rule(
            "Video fullscreen",
            true,
            vec![predicate(
                MatchKind::Title,
                MatchStrategy::Regex,
                r"(?i)(youtube|netflix|vlc|mpv)",
            )],
            vec![
                RuleEffect::Fullscreen,
                RuleEffect::BorderColor(RuleColor::Hidden),
                RuleEffect::Backdrop(SystemBackdrop::None),
            ],
        ),
        rule(
            "Music player card",
            true,
            vec![predicate(
                MatchKind::Exe,
                MatchStrategy::Regex,
                r"(spotify|musicbee|foobar2000)\.exe",
            )],
            vec![
                RuleEffect::Float,
                RuleEffect::MoveToWorkspace {
                    monitor_index: None,
                    workspace_index: 1,
                },
                RuleEffect::Resize {
                    width: RuleMeasure::Percent(54),
                    height: RuleMeasure::Percent(70),
                },
                RuleEffect::Center,
                RuleEffect::Backdrop(SystemBackdrop::MicaAlt),
                RuleEffect::Corners(WindowCorner::Round),
            ],
        ),
    ];
    config.desktop.bindings = vec![
        binding("Toggle fullscreen", "F", DispatchCommand::ToggleFullscreen),
        binding("Next media layout", "L", DispatchCommand::CycleNextLayout),
    ];
    example(
        "media-lounge",
        "Media Lounge",
        "Fullscreen video and a centered, Mica-styled music card on a dedicated workspace.",
        config,
    )
}

fn example_accessible_focus() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.focus.focus_follows_mouse = true;
    config.desktop.focus.focus_delay_ms = 250;
    config.desktop.appearance.border_width = 8;
    config.desktop.appearance.active_border_color = (255, 210, 0);
    config.desktop.appearance.inactive_border_color = (60, 60, 60);
    config.desktop.appearance.dim_inactive = true;
    config.desktop.appearance.inactive_window_opacity = 185;
    config.desktop.platform.windows.shadow = ShadowPolicy::Enabled;
    for channel in [
        &mut config.desktop.animations.layout,
        &mut config.desktop.animations.workspace,
        &mut config.desktop.animations.border,
        &mut config.desktop.animations.window,
    ] {
        channel.duration_ms = 320;
        channel.curve = AnimationCurve::EaseOut;
    }
    config.desktop.workspaces = vec![WorkspaceProfile {
        gaps_in: 14,
        gaps_out: 22,
        ..workspace("focus", LayoutType::Grid, true)
    }];
    config.desktop.window_rules = vec![rule(
        "Magnifier stays visible",
        true,
        vec![predicate(
            MatchKind::Exe,
            MatchStrategy::Equals,
            "magnify.exe",
        )],
        vec![
            RuleEffect::Float,
            RuleEffect::AlwaysOnTop(true),
            RuleEffect::DimInactive(false),
            RuleEffect::Shadow(true),
            RuleEffect::Resize {
                width: RuleMeasure::Percent(45),
                height: RuleMeasure::Percent(35),
            },
            RuleEffect::Move {
                x: RuleMeasure::Percent(52),
                y: RuleMeasure::Pixels(20),
            },
            RuleEffect::BorderColor(RuleColor::Rgb {
                r: 255,
                g: 210,
                b: 0,
            }),
        ],
    )];
    config.desktop.bindings = vec![
        binding("Focus next", "Tab", DispatchCommand::FocusNext),
        shifted_binding("Focus previous", "Tab", DispatchCommand::FocusPrevious),
        binding("Overview", "O", DispatchCommand::ToggleExpose),
    ];
    example(
        "accessible-focus",
        "Accessible Focus",
        "Focus-follows-mouse, inactive dimming, compositor shadows, larger high-contrast borders and gaps, slower motion, a dim-exempt magnifier, and overview.",
        config,
    )
}

fn example_automation_showcase() -> BuiltInExample {
    let mut config = Config::new();
    config.desktop.workspaces = vec![workspace("automation", LayoutType::BSP, true)];
    config.desktop.startup = vec![StartupProfile {
        id: "terminal".to_string(),
        name: "Windows Terminal".to_string(),
        launch: LaunchProfile {
            program: "wt.exe".to_string(),
            args: vec!["-w".to_string(), "0".to_string()],
            ..LaunchProfile::default()
        },
        delay_ms: 750,
        restart: StartupRestartPolicy::OnFailure,
        restart_delay_ms: 2_000,
        max_restarts: Some(2),
        ..StartupProfile::default()
    }];
    let mut graph = NodeGraph::new();
    let focused = graph.add_node(NodeType::OnWindowFocused, [60.0, 150.0]);
    let border = graph.add_node(
        NodeType::SetNativeBorderColor {
            r: 0,
            g: 200,
            b: 255,
        },
        [320.0, 80.0],
    );
    let caption = graph.add_node(
        NodeType::SetCaptionColor {
            r: 20,
            g: 35,
            b: 55,
        },
        [560.0, 80.0],
    );
    let backdrop = graph.add_node(
        NodeType::SetSystemBackdrop {
            backdrop_type: SystemBackdropType::Mica,
        },
        [800.0, 80.0],
    );
    let corners = graph.add_node(
        NodeType::SetWindowCorner {
            corner_type: WindowCornerType::RoundedSmall,
        },
        [1040.0, 80.0],
    );
    let log = graph.add_node(
        NodeType::LogMessage {
            message: "Focused window decorations refreshed".to_string(),
            level: LogLevel::Info,
        },
        [1280.0, 80.0],
    );
    for (from, to) in [
        (focused, border),
        (border, caption),
        (caption, backdrop),
        (backdrop, corners),
        (corners, log),
    ] {
        let output = if from == focused { "Event" } else { "Then" };
        graph
            .add_connection_by_name(from, output, to, "Trigger")
            .expect("built-in automation execution chain must connect");
    }
    graph
        .add_connection_by_name(focused, "Window", border, "Window")
        .expect("focused window must connect");
    graph
        .add_connection_by_name(border, "Window", caption, "Window")
        .expect("window pass-through must connect");
    graph
        .add_connection_by_name(caption, "Window", backdrop, "Window")
        .expect("window pass-through must connect");
    graph
        .add_connection_by_name(backdrop, "Window", corners, "Window")
        .expect("window pass-through must connect");
    let manual_launch = graph.add_node(NodeType::ManualTrigger, [60.0, 360.0]);
    let launch_notepad = graph.add_node(
        NodeType::LaunchProcess {
            program: "notepad.exe".to_string(),
            args: Vec::new(),
            working_directory: String::new(),
            create_no_window: false,
        },
        [320.0, 360.0],
    );
    graph
        .add_connection_by_name(manual_launch, "Trigger", launch_notepad, "Trigger")
        .expect("manual process-launch demonstration must connect");
    config.node_graph = graph;
    config.desktop.bindings = vec![binding(
        "Restart terminal service",
        "R",
        DispatchCommand::RestartStartup {
            id: "terminal".to_string(),
        },
    )];
    example(
        "automation-showcase",
        "Automation Graph Showcase",
        "A supervised delayed startup application with restart controls, a safe manual process-launch graph, and a focus-event chain for decorations and logging.",
        config,
    )
}

fn example_snap_zone_workbench() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![workspace("zone-lab", LayoutType::BSP, true)];
    config.desktop.snap_zones.default_layout = snap_zone_presets::priority_grid();
    config.desktop.snap_zones.behavior.snap_threshold = 0.2;
    config.desktop.bindings = vec![
        binding(
            "Snap to main zone",
            "1",
            DispatchCommand::SnapActiveToZone { index: 0 },
        ),
        binding(
            "Snap to upper side zone",
            "2",
            DispatchCommand::SnapActiveToZone { index: 1 },
        ),
        binding(
            "Snap to lower side zone",
            "3",
            DispatchCommand::SnapActiveToZone { index: 2 },
        ),
        binding(
            "Cycle zone forward",
            "N",
            DispatchCommand::CycleActiveZoneNext,
        ),
        binding(
            "Cycle zone backward",
            "P",
            DispatchCommand::CycleActiveZonePrevious,
        ),
    ];
    example(
        "snap-zone-workbench",
        "Snap Zone Workbench",
        "A priority-grid zone desktop with direct Win+1/2/3 placement, forward/backward cycling, and editable normalized geometry.",
        config,
    )
}

fn example_advanced_binding_lab() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    config.desktop.workspaces = vec![
        workspace("input-lab", LayoutType::Dwindle, true),
        workspace("secondary", LayoutType::MasterStack, false),
    ];

    let enter_resize = binding(
        "Enter resize submap",
        "R",
        DispatchCommand::SetBindingSubmap {
            name: "resize".to_string(),
        },
    );

    let mut release_float = binding("Float on key release", "F", DispatchCommand::ToggleFloating);
    release_float.trigger = BindingTrigger::Release;

    let mut long_close = binding("Long-press close", "Q", DispatchCommand::CloseActive);
    long_close.trigger = BindingTrigger::LongPress;
    long_close.long_press_ms = 750;

    let mut pass_through_pin = binding("Pass-through pin", "P", DispatchCommand::TogglePinned);
    pass_through_pin.win = false;
    pass_through_pin.ctrl = true;
    pass_through_pin.alt = true;
    pass_through_pin.consume = false;

    let mut mouse_overview = binding(
        "Mouse back release overview",
        "X1",
        DispatchCommand::ToggleExpose,
    );
    mouse_overview.device = BindingDevice::Mouse;
    mouse_overview.trigger = BindingTrigger::Release;
    mouse_overview.win = false;

    let mut wheel_workspace = binding(
        "Win+wheel next workspace",
        "WHEEL_DOWN",
        DispatchCommand::SwitchWorkspaceSelector {
            selector: WorkspaceSelector::Next,
        },
    );
    wheel_workspace.device = BindingDevice::Mouse;

    let repeat_move = |name: &str, key: &str, delta_x: i32, delta_y: i32| BindingProfile {
        name: name.to_string(),
        key: key.to_string(),
        submap: "resize".to_string(),
        trigger: BindingTrigger::Repeat,
        win: false,
        command: DispatchCommand::MoveActive { delta_x, delta_y },
        ..BindingProfile::default()
    };
    let press_move = |name: &str, key: &str, delta_x: i32, delta_y: i32| BindingProfile {
        name: name.to_string(),
        key: key.to_string(),
        submap: "resize".to_string(),
        trigger: BindingTrigger::Press,
        win: false,
        command: DispatchCommand::MoveActive { delta_x, delta_y },
        ..BindingProfile::default()
    };
    let exit_resize = BindingProfile {
        name: "Exit resize submap".to_string(),
        key: "ESCAPE".to_string(),
        submap: "resize".to_string(),
        win: false,
        command: DispatchCommand::ResetBindingSubmap,
        ..BindingProfile::default()
    };

    // The zones on the desktop by the keyboard, as a mode of its own:
    // Super+Ctrl+Z enters it, and inside no modifier is held. The arrows
    // go from zone to zone, H J K L walk the items of the zone, Shift
    // with an arrow carries the chosen items to the zone on that side.
    // "This zone" is the one that has the keyboard (`@focused`). Escape
    // lets the zones go back below the windows as it is pressed and
    // leaves the mode as it is let go: two things on one key, each on
    // its own half of the stroke.
    let in_zones = |name: &str, key: &str, action: serde_json::Value| BindingProfile {
        name: name.to_string(),
        key: key.to_string(),
        submap: "zones".to_string(),
        win: false,
        command: DispatchCommand::Zones { action },
        ..BindingProfile::default()
    };
    let mut zone_keys = vec![
        BindingProfile {
            ctrl: true,
            ..binding(
                "Enter the zones mode",
                "Z",
                DispatchCommand::SetBindingSubmap {
                    name: "zones".to_string(),
                },
            )
        },
        in_zones(
            "Zones above the windows, or back",
            "Z",
            serde_json::json!({"command": "toggle_peek"}),
        ),
        in_zones(
            "Open what is chosen",
            "RETURN",
            serde_json::json!({"command": "open_focused"}),
        ),
        in_zones(
            "Roll this zone up or down",
            "R",
            serde_json::json!({"command": "roll_up", "zone": "@focused"}),
        ),
        in_zones(
            "Next tab of this zone",
            "TAB",
            serde_json::json!({"command": "next_tab", "zone": "@focused"}),
        ),
        in_zones(
            "Hide or show the zones",
            "SPACE",
            serde_json::json!({"command": "quick_hide"}),
        ),
        in_zones(
            "Cover what the zones hold",
            "M",
            serde_json::json!({"command": "mask"}),
        ),
        in_zones(
            "Zones back below the windows",
            "ESCAPE",
            serde_json::json!({"command": "peek", "on": false}),
        ),
        BindingProfile {
            name: "Leave the zones mode".to_string(),
            key: "ESCAPE".to_string(),
            submap: "zones".to_string(),
            trigger: BindingTrigger::Release,
            win: false,
            command: DispatchCommand::ResetBindingSubmap,
            ..BindingProfile::default()
        },
    ];
    for (direction, arrow, letter) in [
        ("left", "LEFT", "H"),
        ("down", "DOWN", "J"),
        ("up", "UP", "K"),
        ("right", "RIGHT", "L"),
    ] {
        zone_keys.push(in_zones(
            &format!("The zone to the {direction}"),
            arrow,
            serde_json::json!({"command": "focus_neighbour", "direction": direction}),
        ));
        zone_keys.push(in_zones(
            &format!("The item to the {direction}"),
            letter,
            serde_json::json!({"command": "move_item_focus", "step": direction}),
        ));
        zone_keys.push(BindingProfile {
            shift: true,
            ..in_zones(
                &format!("Move the chosen items to the zone on the {direction}"),
                arrow,
                serde_json::json!({"command": "move_focused_items", "direction": direction}),
            )
        });
    }

    config.desktop.bindings = vec![
        enter_resize,
        release_float,
        long_close,
        pass_through_pin,
        mouse_overview,
        wheel_workspace,
        press_move("Move left immediately", "H", -24, 0),
        repeat_move("Keep moving left", "H", -24, 0),
        press_move("Move down immediately", "J", 0, 24),
        repeat_move("Keep moving down", "J", 0, 24),
        press_move("Move up immediately", "K", 0, -24),
        repeat_move("Keep moving up", "K", 0, -24),
        press_move("Move right immediately", "L", 24, 0),
        repeat_move("Keep moving right", "L", 24, 0),
        exit_resize,
    ];
    config.desktop.bindings.extend(zone_keys);
    config.desktop.gestures.profiles = vec![
        GestureProfile {
            name: "Three-finger next workspace".to_string(),
            fingers: 3,
            direction: GestureDirection::Left,
            distance_percent: 18.0,
            command: DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Next,
            },
            ..GestureProfile::default()
        },
        GestureProfile {
            name: "Three-finger previous workspace".to_string(),
            fingers: 3,
            direction: GestureDirection::Right,
            distance_percent: 18.0,
            command: DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Previous,
            },
            ..GestureProfile::default()
        },
        GestureProfile {
            name: "Four-finger overview".to_string(),
            fingers: 4,
            direction: GestureDirection::Up,
            distance_percent: 22.0,
            command: DispatchCommand::ToggleExpose,
            ..GestureProfile::default()
        },
    ];
    example(
        "advanced-binding-lab",
        "Advanced Binding Lab",
        "Press, release, repeat, long-press, mouse and pass-through bindings, global three/four-finger touchpad swipes, a modal resize submap with an explicit escape path, and a zones mode (Super+Ctrl+Z) that walks the desktop's zones and their items by the keyboard.",
        config,
    )
}

fn example_minimal_monocle() -> BuiltInExample {
    let mut config = Config::new();
    config.node_graph = startup_filter_graph();
    for channel in [
        &mut config.desktop.animations.layout,
        &mut config.desktop.animations.workspace,
        &mut config.desktop.animations.border,
        &mut config.desktop.animations.window,
    ] {
        channel.duration_ms = 120;
    }
    config.desktop.appearance.border_enabled = false;
    config.desktop.workspaces = vec![
        workspace("one", LayoutType::Monocle, true),
        workspace("two", LayoutType::Monocle, false),
    ];
    config.desktop.bindings = vec![
        binding("Next window", "J", DispatchCommand::FocusNext),
        binding("Previous window", "K", DispatchCommand::FocusPrevious),
        binding(
            "Next workspace",
            "L",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Next,
            },
        ),
        binding(
            "Previous workspace",
            "H",
            DispatchCommand::SwitchWorkspaceSelector {
                selector: WorkspaceSelector::Previous,
            },
        ),
        binding("Toggle floating", "Space", DispatchCommand::ToggleFloating),
    ];
    example(
        "minimal-monocle",
        "Minimal Monocle",
        "A distraction-free, borderless two-workspace setup with only focus, workspace, and floating controls.",
        config,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_example_is_put_on_over_what_is_the_persons_own() {
        let mut own = starting_configuration();
        own.desktop.input.keyboard.layouts = vec![
            crate::input::KeyboardLayout {
                layout: "us".to_string(),
                variant: String::new(),
                extra: Default::default(),
            },
            crate::input::KeyboardLayout {
                layout: "tr".to_string(),
                variant: String::new(),
                extra: Default::default(),
            },
        ];
        own.desktop.input.keyboard.numlock = true;
        own.desktop.night_light.enabled = true;
        own.desktop
            .window_rules
            .push(crate::settings::WindowRuleProfile {
                name: "Nested gevrek floats".to_string(),
                ..Default::default()
            });
        for example in built_in_examples() {
            let worn = put_on(&example.config, &own);
            // The person's keyboard, night light and rule are still there...
            assert_eq!(worn.desktop.input, own.desktop.input, "{}", example.id);
            assert!(worn.desktop.night_light.enabled, "{}", example.id);
            assert!(
                worn.desktop
                    .window_rules
                    .iter()
                    .any(|rule| rule.name == "Nested gevrek floats"),
                "{}",
                example.id
            );
            // ...and the look is the example's.
            assert_eq!(
                worn.desktop.appearance.active_border_color,
                example.config.desktop.appearance.active_border_color,
                "{}",
                example.id
            );
            assert_eq!(worn.desktop.shell.panel, example.config.desktop.shell.panel);
            assert_eq!(
                worn.desktop.bindings.len(),
                example.config.desktop.bindings.len()
            );
        }
    }
    use std::collections::HashSet;

    /// Super+Shift+D shows the desktop (Windows' own Win+D, with Shift: D
    /// alone is the launcher's), and in no example is that key, or the
    /// command, anything else.
    #[test]
    fn super_shift_d_shows_the_desktop_and_is_nobody_elses() {
        let is_it = |binding: &BindingProfile| {
            binding.normalized_key() == "D"
                && binding.win
                && binding.shift
                && !binding.ctrl
                && !binding.alt
                && binding.normalized_submap() == "default"
        };
        let standard = standard_bindings();
        let on_it: Vec<&BindingProfile> = standard.iter().filter(|binding| is_it(binding)).collect();
        assert_eq!(on_it.len(), 1);
        assert_eq!(on_it[0].command, DispatchCommand::ShowDesktop { on: None });
        assert_eq!(on_it[0].trigger, BindingTrigger::Press);
        for example in built_in_examples() {
            for binding in &example.config.desktop.bindings {
                if is_it(binding) {
                    assert_eq!(
                        binding.command,
                        DispatchCommand::ShowDesktop { on: None },
                        "{}: {}",
                        example.id,
                        binding.name
                    );
                }
                if matches!(binding.command, DispatchCommand::ShowDesktop { .. }) {
                    assert!(is_it(binding), "{}: {} shows the desktop", example.id, binding.name);
                }
            }
            // (Where an example has the standard keys, it has this one.)
            let standard_keys = example
                .config
                .desktop
                .bindings
                .iter()
                .any(|binding| binding.name == "Show shortcuts");
            if standard_keys {
                assert!(
                    example.config.desktop.bindings.iter().any(is_it),
                    "{} has the standard keys but not Super+Shift+D",
                    example.id
                );
            }
        }
    }

    #[test]
    fn the_keys_of_the_zones_are_standard_and_nobody_elses() {
        let standard = standard_bindings();
        // No two of the standard keys are one key pressed one way.
        let mut chords = HashSet::new();
        for binding in &standard {
            assert!(
                chords.insert((binding.normalized_submap(), binding.chord_key())),
                "{} is on a key that is taken: {}",
                binding.name,
                binding.chord_key()
            );
        }
        // Super+Z and Super+Shift+Z, each on the press alone.
        let on_z: Vec<(bool, BindingTrigger, String)> = standard
            .iter()
            .filter(|binding| binding.normalized_key() == "Z")
            .map(|binding| {
                assert!(binding.win && !binding.ctrl && !binding.alt, "{}", binding.name);
                let DispatchCommand::Zones { action } = &binding.command else {
                    panic!("{} is on Z and is not the zones'", binding.name);
                };
                (binding.shift, binding.trigger, action.to_string())
            })
            .collect();
        assert_eq!(
            on_z,
            [
                (false, BindingTrigger::Press, r#"{"command":"toggle_peek"}"#.to_string()),
                (true, BindingTrigger::Press, r#"{"command":"quick_hide"}"#.to_string()),
            ]
        );
        // The mode of the Advanced Binding Lab: entered, walked, left.
        let lab = built_in_examples()
            .into_iter()
            .find(|example| example.id == "advanced-binding-lab")
            .unwrap();
        let in_mode: Vec<&BindingProfile> = lab
            .config
            .desktop
            .bindings
            .iter()
            .filter(|binding| binding.normalized_submap() == "zones")
            .collect();
        assert_eq!(in_mode.len(), 8 + 12);
        assert!(in_mode.iter().all(|binding| !binding.win && !binding.ctrl && !binding.alt));
        assert!(in_mode.iter().any(|binding| {
            binding.normalized_key() == "ESCAPE"
                && binding.trigger == BindingTrigger::Release
                && binding.command == DispatchCommand::ResetBindingSubmap
        }));
        assert!(lab.config.desktop.bindings.iter().any(|binding| {
            binding.normalized_key() == "Z"
                && binding.ctrl
                && binding.win
                && matches!(&binding.command, DispatchCommand::SetBindingSubmap { name } if name == "zones")
        }));
        // In no example is Z with Super anything but the zones' (the
        // examples with keys of their own included).
        for example in built_in_examples() {
            let mut chords = HashSet::new();
            for binding in &example.config.desktop.bindings {
                assert!(
                    chords.insert((binding.normalized_submap(), binding.chord_key())),
                    "{}: {} is on a key that is taken",
                    example.id,
                    binding.name
                );
                if binding.normalized_key() == "Z" && binding.win && binding.normalized_submap() == "default" {
                    assert!(
                        matches!(
                            binding.command,
                            DispatchCommand::Zones { .. } | DispatchCommand::SetBindingSubmap { .. }
                        ),
                        "{}: {}",
                        example.id,
                        binding.name
                    );
                }
            }
        }
    }

    #[test]
    fn starter_library_has_at_least_ten_unique_valid_examples() {
        let examples = built_in_examples();
        assert!(examples.len() >= 10);
        let mut ids = HashSet::new();
        for example in examples {
            assert!(
                ids.insert(example.id),
                "duplicate example id {}",
                example.id
            );
            assert!(
                example.config.validate().is_empty(),
                "{} has validation errors: {:?}",
                example.name,
                example.config.validate()
            );
            assert!(
                !example.config.desktop.workspaces.is_empty()
                    || !example.config.node_graph.nodes.is_empty(),
                "{} demonstrates no functionality",
                example.name
            );
        }
    }

    #[test]
    fn the_showcase_examples_use_the_drawn_effects_the_bar_and_a_readable_graph() {
        let examples = built_in_examples();
        let of = |id: &str| {
            examples
                .iter()
                .find(|example| example.id == id)
                .unwrap_or_else(|| panic!("no example {id}"))
        };
        assert!(
            of("aurora")
                .config
                .desktop
                .appearance
                .effects
                .gradient
                .spin_seconds
                > 0.0
        );
        assert!(
            of("focus-stage")
                .config
                .desktop
                .appearance
                .effects
                .tint
                .enabled
        );
        assert!(
            of("neon-grid")
                .config
                .desktop
                .appearance
                .effects
                .glow
                .enabled
        );
        assert!(of("paper").config.desktop.appearance.effects.shadow.enabled);
        for id in ["aurora", "focus-stage", "neon-grid", "paper"] {
            let config = &of(id).config;
            assert!(
                config.desktop.appearance.effects.any(),
                "{id} draws nothing"
            );
            assert!(config.desktop.shell.panel.enabled, "{id} has no bar");
            assert!(config.desktop.animations.enabled, "{id} does not animate");
            // The border sits where gevrek puts it by default.
            assert_eq!(
                config.desktop.appearance.border_offset, -1,
                "{id} moves the border"
            );
            // New windows are tiled, and the overview and the list of
            // shortcuts have keys.
            let shortcuts = crate::shortcuts::shortcuts_of(config);
            for action in ["Show shortcuts", "Toggle overview", "Toggle Expose Mode"] {
                assert!(
                    shortcuts
                        .iter()
                        .any(|shortcut| shortcut.action.contains(action)),
                    "{id} has no key for {action}"
                );
            }
            assert!(config
                .node_graph
                .nodes
                .values()
                .any(|node| matches!(node.node_type, NodeType::SetWindowArrangement { .. })));
            // The graph is laid out: putting it in order again moves nothing.
            let mut graph = config.node_graph.clone();
            assert_eq!(
                crate::graph_layout::tidy(&mut graph),
                0,
                "{id}'s graph is not in order"
            );
        }
    }

    #[test]
    fn starter_library_covers_graphs_rules_and_geometry() {
        let examples = built_in_examples();
        assert!(examples
            .iter()
            .any(|example| !example.config.node_graph.connections.is_empty()));
        assert!(examples
            .iter()
            .any(|example| !example.config.desktop.window_rules.is_empty()));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| {
                    matches!(
                        effect,
                        RuleEffect::Move { .. } | RuleEffect::Resize { .. } | RuleEffect::Center
                    )
                })
        }));
        assert!(examples.iter().any(|example| {
            example.config.desktop.snap_zones.default_layout.zones.len() >= 3
                && example.config.desktop.bindings.iter().any(|binding| {
                    matches!(binding.command, DispatchCommand::SnapActiveToZone { .. })
                })
        }));
        assert!(examples.iter().any(|example| {
            let bindings = &example.config.desktop.bindings;
            bindings
                .iter()
                .any(|binding| binding.trigger == BindingTrigger::Release)
                && bindings
                    .iter()
                    .any(|binding| binding.trigger == BindingTrigger::Repeat)
                && bindings
                    .iter()
                    .any(|binding| binding.trigger == BindingTrigger::LongPress)
                && bindings
                    .iter()
                    .any(|binding| binding.device == BindingDevice::Mouse)
                && bindings
                    .iter()
                    .any(|binding| binding.normalized_submap() != "default")
                && bindings.iter().any(|binding| !binding.consume)
        }));
        assert!(examples.iter().any(|example| {
            let gestures = &example.config.desktop.gestures.profiles;
            gestures.iter().any(|gesture| gesture.fingers == 3)
                && gestures.iter().any(|gesture| gesture.fingers == 4)
                && gestures
                    .iter()
                    .any(|gesture| gesture.direction == GestureDirection::Left)
                && gestures
                    .iter()
                    .any(|gesture| gesture.direction == GestureDirection::Right)
        }));
        assert!(examples.iter().any(|example| {
            !example.config.desktop.monitors.is_empty()
                && example
                    .config
                    .desktop
                    .monitors
                    .iter()
                    .any(|profile| profile.reserved != ReservedArea::default())
        }));
        assert!(examples.iter().any(|example| {
            example.config.desktop.workspaces.iter().any(|workspace| {
                workspace.master_position != MasterPosition::Left
                    || workspace.new_window_placement != NewWindowPlacement::StackEnd
            })
        }));
        assert!(examples.iter().any(|example| {
            example.config.desktop.appearance.dim_inactive
                && example
                    .config
                    .desktop
                    .window_rules
                    .iter()
                    .flat_map(|rule| &rule.effects)
                    .any(|effect| matches!(effect, RuleEffect::DimInactive(false)))
        }));
        assert!(examples.iter().any(|example| {
            example.config.desktop.platform.windows.shadow != ShadowPolicy::Default
                && example
                    .config
                    .desktop
                    .window_rules
                    .iter()
                    .flat_map(|rule| &rule.effects)
                    .any(|effect| matches!(effect, RuleEffect::Shadow(_)))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::NoInitialFocus(true)))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::NoFocus(true)))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::NoFollowMouse(true)))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::StayFocused(true)))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::MinSize { .. }))
                && example
                    .config
                    .desktop
                    .window_rules
                    .iter()
                    .flat_map(|rule| &rule.effects)
                    .any(|effect| matches!(effect, RuleEffect::MaxSize { .. }))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::KeepAspectRatio(true)))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .window_rules
                .iter()
                .flat_map(|rule| &rule.effects)
                .any(|effect| matches!(effect, RuleEffect::PersistentSize(true)))
        }));
        assert!(examples.iter().any(|example| {
            !example.config.desktop.startup.is_empty()
                && example.config.desktop.startup.iter().any(|profile| {
                    profile.restart != StartupRestartPolicy::Never
                        && !profile.launch.program.trim().is_empty()
                })
                && example.config.desktop.bindings.iter().any(|binding| {
                    matches!(
                        binding.command,
                        DispatchCommand::LaunchStartup { .. }
                            | DispatchCommand::StopStartup { .. }
                            | DispatchCommand::RestartStartup { .. }
                    )
                })
        }));
        assert!(examples.iter().any(|example| {
            let commands = example
                .config
                .desktop
                .bindings
                .iter()
                .map(|binding| &binding.command)
                .collect::<Vec<_>>();
            commands
                .iter()
                .any(|command| matches!(command, DispatchCommand::FocusMonitor { .. }))
                && commands.iter().any(|command| {
                    matches!(command, DispatchCommand::MoveWorkspaceToMonitor { .. })
                })
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::SwapActiveWorkspaces { .. }))
        }));
        assert!(examples.iter().any(|example| {
            let commands = example
                .config
                .desktop
                .bindings
                .iter()
                .map(|binding| &binding.command)
                .collect::<Vec<_>>();
            commands
                .iter()
                .any(|command| matches!(command, DispatchCommand::MoveToWorkspaceAndFollow { .. }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::RenameWorkspace { .. }))
        }));
        assert!(examples.iter().any(|example| {
            let commands = example
                .config
                .desktop
                .bindings
                .iter()
                .map(|binding| &binding.command)
                .collect::<Vec<_>>();
            commands
                .iter()
                .any(|command| matches!(command, DispatchCommand::SetFloating { enabled: true }))
                && commands.iter().any(|command| {
                    matches!(command, DispatchCommand::SetFloating { enabled: false })
                })
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::CenterActive))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::RaiseActive))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::LowerActive))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::SetPinned { enabled: true }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::SetPinned { enabled: false }))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .desktop
                .bindings
                .iter()
                .any(|binding| matches!(binding.command, DispatchCommand::ClearWindowTags))
        }));
        assert!(examples.iter().any(|example| {
            let commands = example
                .config
                .desktop
                .bindings
                .iter()
                .map(|binding| &binding.command)
                .collect::<Vec<_>>();
            commands
                .iter()
                .any(|command| matches!(command, DispatchCommand::FocusWindow { .. }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::FocusLast))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::CenterWindow { .. }))
                && commands.iter().any(|command| {
                    matches!(
                        command,
                        DispatchCommand::SetWindowFloating { enabled: true, .. }
                    )
                })
                && commands.iter().any(|command| {
                    matches!(
                        command,
                        DispatchCommand::SetWindowPinned { enabled: true, .. }
                    )
                })
                && commands.iter().any(|command| {
                    matches!(command, DispatchCommand::ToggleWindowFullscreen { .. })
                })
                && commands.iter().any(|command| {
                    matches!(command, DispatchCommand::ToggleWindowPseudotile { .. })
                })
                && commands.iter().any(|command| {
                    matches!(command, DispatchCommand::ToggleSelectedWindowTag { .. })
                })
                && commands.iter().any(|command| {
                    matches!(
                        command,
                        DispatchCommand::MoveWindowToWorkspaceAndFollow { .. }
                    )
                })
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::ToggleWindowGroup { .. }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::CycleWindowGroup { .. }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::FocusWindowGroupIndex { .. }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::ToggleWindowGroupLock { .. }))
                && commands
                    .iter()
                    .any(|command| matches!(command, DispatchCommand::MoveWindowInGroup { .. }))
        }));
        assert!(examples.iter().any(|example| {
            example
                .config
                .node_graph
                .nodes
                .values()
                .any(|node| matches!(node.node_type, NodeType::LaunchProcess { .. }))
        }));
        assert!(examples.iter().any(|example| {
            let animations = &example.config.desktop.animations;
            animations.enabled
                && animations.layout.enabled
                && animations.workspace.enabled
                && animations.border.enabled
                && animations.window.enabled
                && animations.layout.duration_ms != animations.workspace.duration_ms
                && animations.border.curve != animations.window.curve
                && animations.window_enter_style != crate::settings::WindowTransitionStyle::None
                && animations.window_exit_style != crate::settings::WindowTransitionStyle::None
        }));
        assert!(examples.iter().any(|example| {
            example.config.desktop.workspaces.iter().any(|workspace| {
                !workspace.persistent
                    && workspace
                        .on_created_empty
                        .as_ref()
                        .is_some_and(|launch| !launch.program.trim().is_empty())
            })
        }));
    }
}
