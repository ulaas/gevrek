//! The pages, and which settings each one shows.
//!
//! A page is a list of cards; a card shows places of the configuration
//! through the form. Which place is on which page is written here once,
//! and a test holds the list against the schema: a setting no page shows
//! fails it, so a setting added to the model cannot be forgotten.

use gevrek_ui::theme::icons;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Overview,
    Displays,
    Workspaces,
    Appearance,
    Animations,
    Wallpaper,
    Bar,
    Zones,
    Bindings,
    Gestures,
    Rules,
    Scratchpads,
    SnapZones,
    Focus,
    Input,
    Startup,
    Session,
    System,
    Graph,
    Commands,
    Windows,
    Events,
    Log,
    Raw,
    History,
    Problems,
    Diagnostics,
}

/// What a card shows of the object at its place.
#[derive(Debug, Clone, Copy)]
pub enum Show {
    /// The place itself, whatever it holds (a list, a value).
    Place,
    /// Every field of the object, except these.
    AllBut(&'static [&'static str]),
    /// These fields of the object, in this order.
    Only(&'static [&'static str]),
    /// Every field of the object; these of them are drawn by a card of
    /// the page's own (dragged, chosen by looking), and the form leaves
    /// them out. One thing is set in one place.
    Rest(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
pub struct Card {
    pub title: &'static str,
    pub pointer: &'static str,
    pub show: Show,
}

const fn card(title: &'static str, pointer: &'static str, show: Show) -> Card {
    Card {
        title,
        pointer,
        show,
    }
}

impl Page {
    /// What is long on the page moves in a room of its own (the list of
    /// the bindings, the events, the log, the text): the page itself is
    /// as high as the window and does not move.
    pub fn scrolls_by_itself(self) -> bool {
        matches!(self, Page::Bindings | Page::Events | Page::Log | Page::Raw)
    }

    pub const ALL: [Page; 27] = [
        Page::Overview,
        Page::Displays,
        Page::Workspaces,
        Page::Appearance,
        Page::Animations,
        Page::Wallpaper,
        Page::Bar,
        Page::Zones,
        Page::Bindings,
        Page::Gestures,
        Page::Rules,
        Page::Scratchpads,
        Page::SnapZones,
        Page::Focus,
        Page::Input,
        Page::Startup,
        Page::Session,
        Page::System,
        Page::Graph,
        Page::Commands,
        Page::Windows,
        Page::Events,
        Page::Log,
        Page::Raw,
        Page::History,
        Page::Problems,
        Page::Diagnostics,
    ];

    /// The name in the address of the page (`--page appearance`).
    pub fn key(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Displays => "displays",
            Page::Workspaces => "workspaces",
            Page::Appearance => "appearance",
            Page::Animations => "animations",
            Page::Wallpaper => "wallpaper",
            Page::Bar => "bar",
            Page::Zones => "zones",
            Page::Bindings => "bindings",
            Page::Gestures => "gestures",
            Page::Rules => "rules",
            Page::Scratchpads => "scratchpads",
            Page::SnapZones => "snap-zones",
            Page::Focus => "focus",
            Page::Input => "input",
            Page::Startup => "startup",
            Page::Session => "session",
            Page::System => "system",
            Page::Graph => "graph",
            Page::Commands => "commands",
            Page::Windows => "windows",
            Page::Events => "events",
            Page::Log => "log",
            Page::Raw => "raw",
            Page::History => "history",
            Page::Problems => "problems",
            Page::Diagnostics => "diagnostics",
        }
    }

    pub fn from_key(key: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|page| page.key() == key)
    }

    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Displays => "Displays",
            Page::Workspaces => "Workspaces and layout",
            Page::Appearance => "Appearance",
            Page::Animations => "Animations",
            Page::Wallpaper => "Wallpaper",
            Page::Bar => "Panel",
            Page::Zones => "Zones on the desktop",
            Page::Bindings => "Key bindings",
            Page::Gestures => "Mouse and gestures",
            Page::Rules => "Window rules",
            Page::Scratchpads => "Scratchpads",
            Page::SnapZones => "Snap zones",
            Page::Focus => "Focus",
            Page::Input => "Input",
            Page::Startup => "Startup",
            Page::Session => "Session",
            Page::System => "This system",
            Page::Graph => "Automation graph",
            Page::Commands => "Command centre",
            Page::Windows => "Windows and outputs",
            Page::Events => "Events",
            Page::Log => "Log",
            Page::Raw => "Raw configuration",
            Page::History => "History",
            Page::Problems => "Problems",
            Page::Diagnostics => "Diagnostics",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Page::Overview => "The window manager this window talks to, and where to start.",
            Page::Displays => "Mode, place and scale of each display. A new mode is on trial for 15 seconds.",
            Page::Workspaces => "The workspaces each display has: what they are called, how they are laid out, and which stay when they are empty.",
            Page::Appearance => "Borders, opacity, and the drawn effects around windows.",
            Page::Animations => "How windows and workspaces move.",
            Page::Wallpaper => "The picture, or the colour, behind the windows. A display can have its own.",
            Page::Bar => "The panel at the edge of each display, and what stands on it.",
            Page::Zones => "Containers on the desktop that hold its icons: the zones there are, the rules that sort items into them, and how they look. (Not the places windows snap to.)",
            Page::Bindings => "What the keys do.",
            Page::Gestures => "What the corners of a display do, and touchpad gestures.",
            Page::Rules => "What happens to a window when it opens, by what it is.",
            Page::Scratchpads => "Programs that come and go on a key.",
            Page::SnapZones => "Places on a display that windows snap to. (Not the zones that hold the desktop's icons: those are Zones on the desktop.)",
            Page::Focus => "Which window has the keyboard, and when that changes.",
            Page::Input => "Keyboard layouts, mouse and touchpad.",
            Page::Startup => "Programs started with the session.",
            Page::Session => "The environment of the session's programs, and how gevrek starts.",
            Page::System => "Settings that only one system applies.",
            Page::Graph => "What gevrek does by itself when something happens: nodes, wired from pin to pin.",
            Page::Commands => "Everything gevrek can be told to do. Run one now, or copy it for a key or a script.",
            Page::Windows => "What the window manager has now.",
            Page::Events => "What the window manager says is happening.",
            Page::Log => "The window manager's own log.",
            Page::Raw => "The configuration as it is written in the file.",
            Page::History => "What is not applied yet, and the versions of the file that were kept.",
            Page::Problems => "What the configuration's own check says.",
            Page::Diagnostics => "Versions, paths, and what this window manager can do.",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Page::Overview => icons::HOUSE,
            Page::Displays => icons::MONITOR,
            Page::Workspaces => icons::SQUARES_FOUR,
            Page::Appearance => icons::PALETTE,
            Page::Animations => icons::WAVE_SINE,
            Page::Wallpaper => icons::IMAGE,
            Page::Bar => icons::ROWS,
            Page::Zones => icons::FOLDERS,
            Page::Bindings => icons::KEYBOARD,
            Page::Gestures => icons::HAND_SWIPE_RIGHT,
            Page::Rules => icons::FUNNEL,
            Page::Scratchpads => icons::NOTE_PENCIL,
            Page::SnapZones => icons::GRID_FOUR,
            Page::Focus => icons::CROSSHAIR,
            Page::Input => icons::MOUSE,
            Page::Startup => icons::ROCKET_LAUNCH,
            Page::Session => icons::SIGN_IN,
            Page::System => icons::DESKTOP,
            Page::Graph => icons::TREE_STRUCTURE,
            Page::Commands => icons::TERMINAL_WINDOW,
            Page::Windows => icons::APP_WINDOW,
            Page::Events => icons::LIGHTNING,
            Page::Log => icons::SCROLL,
            Page::Raw => icons::BRACKETS_CURLY,
            Page::History => icons::CLOCK_COUNTER_CLOCKWISE,
            Page::Problems => icons::WARNING,
            Page::Diagnostics => icons::STETHOSCOPE,
        }
    }

    /// The heading the page stands under in the list of pages.
    pub fn group(self) -> &'static str {
        match self {
            Page::Overview => "",
            Page::Displays
            | Page::Workspaces
            | Page::Appearance
            | Page::Animations
            | Page::Wallpaper
            | Page::Bar
            | Page::Zones => "Desktop",
            Page::Bindings
            | Page::Gestures
            | Page::Rules
            | Page::Scratchpads
            | Page::SnapZones
            | Page::Focus
            | Page::Input => "Behaviour",
            Page::Startup | Page::Session | Page::System => "Session",
            Page::Graph | Page::Commands => "Automation",
            Page::Windows | Page::Events | Page::Log => "Inspect",
            Page::Raw | Page::History | Page::Problems | Page::Diagnostics => "Advanced",
        }
    }

    /// The cards of settings the page shows through the form. Pages drawn
    /// by hand (the overview, the inspectors) have none.
    pub fn cards(self) -> &'static [Card] {
        // (The theme first: it decides the colours that are not set by
        // hand below. Then what is seen most: borders, floating windows,
        // how solid windows are; then the effects, the tabs' strip, and
        // last what is for few: eyes that need more contrast, and
        // borders that are room and nothing else.)
        const APPEARANCE: &[Card] = &[
            card("Theme", "/desktop/appearance/theme", Show::AllBut(&[])),
            card(
                "Borders",
                "/desktop/appearance",
                Show::Only(&[
                    "border_enabled",
                    "border_width",
                    "border_offset",
                    "border_style",
                    "active_border_color",
                    "inactive_border_color",
                    "active_border_alpha",
                    "inactive_border_alpha",
                ]),
            ),
            card(
                "Floating windows",
                "/desktop/appearance/floating",
                Show::AllBut(&[]),
            ),
            card(
                "Opacity",
                "/desktop/appearance",
                Show::Only(&[
                    "transparency_enabled",
                    "window_opacity",
                    "dim_inactive",
                    "inactive_window_opacity",
                    "dim_fullscreen",
                ]),
            ),
            card("Effects", "/desktop/appearance/effects", Show::AllBut(&[])),
            card(
                "Stack bar",
                "/desktop/appearance/stackbar",
                Show::AllBut(&[]),
            ),
            card(
                "For eyes that need it",
                "/desktop/appearance",
                Show::Only(&["high_contrast"]),
            ),
            card(
                "Invisible borders",
                "/desktop/appearance/invisible_borders",
                Show::AllBut(&[]),
            ),
        ];
        // (What a display is first, then what it looks like in the
        // evening; the room kept free at its edges is set once, if ever.)
        const DISPLAYS: &[Card] = &[
            card("Displays", "/desktop/outputs", Show::Place),
            card(
                "Scale",
                "/desktop/platform/wayland",
                Show::Only(&["output_scale", "output_scales"]),
            ),
            card("Night light", "/desktop/night_light", Show::AllBut(&[])),
            card(
                "Space kept free",
                "/desktop/work_area_offset",
                Show::AllBut(&[]),
            ),
            // (Room kept free at one display's edges, and its place
            // among the displays: of a display, not of its workspaces.)
            card("Space kept free, display by display", "/desktop/monitors", Show::Place),
        ];
        // (Drawn by the page of its own, `crate::workspaces`.)
        const WORKSPACES: &[Card] =
            &[card("Workspaces", "/desktop/workspaces", Show::Place)];
        const ANIMATIONS: &[Card] = &[card("Animations", "/desktop/animations", Show::AllBut(&[]))];
        const BAR: &[Card] = &[
            // (What stands on it is placed by dragging, in the card above.)
            card(
                "Panel",
                "/desktop/shell/panel",
                Show::Rest(&["left", "centre", "right"]),
            ),
            card(
                "Control centre and notifications",
                "/desktop/shell/glass",
                Show::AllBut(&[]),
            ),
            card(
                "Notifications",
                "/desktop/shell/notifications",
                Show::AllBut(&[]),
            ),
            // (What the shell shows, together; then what it does, and
            // last the rules for other programs' surfaces.)
            card("On-screen display", "/desktop/shell/osd", Show::AllBut(&[])),
            card(
                "Screenshots",
                "/desktop/shell/screenshots",
                Show::AllBut(&[]),
            ),
            card("Layer rules", "/desktop/layer_rules", Show::Place),
        ];
        const WALLPAPER: &[Card] = &[card("Wallpaper", "/desktop/wallpaper", Show::AllBut(&[]))];
        const BINDINGS: &[Card] = &[card("Bindings", "/desktop/bindings", Show::Place)];
        const GESTURES: &[Card] = &[
            card("Hot corners", "/desktop/hot_corners", Show::AllBut(&[])),
            card("Gestures", "/desktop/gestures", Show::AllBut(&[])),
        ];
        const RULES: &[Card] = &[
            card("Rules", "/desktop/window_rules", Show::Place),
            card(
                "The system's own",
                "/desktop",
                Show::Only(&["apply_system_rules"]),
            ),
        ];
        const SCRATCHPADS: &[Card] = &[card("Scratchpads", "/desktop/scratchpads", Show::Place)];
        // (The zones themselves are drawn and dragged in the card above.)
        const SNAP_ZONES: &[Card] = &[card(
            "How they behave",
            "/desktop/snap_zones",
            Show::Rest(&["default_layout", "monitor_layouts"]),
        )];
        // (The zones of the desktop, which hold its icons. The switch
        // first, then what there is to make: the zones and the rules, each
        // drawn by the page of its own, `crate::zones_page`. Then how all
        // of them look and behave. The last card takes whatever the
        // section holds that the others do not name, so that a setting
        // the model gains is on the page at once.)
        const ZONES: &[Card] = &[
            card("Zones on the desktop", "/desktop/zones", Show::Only(&["enabled"])),
            card("The zones", "/desktop/zones/zones", Show::Place),
            card(
                "Rules that sort the desktop",
                "/desktop/zones",
                Show::Only(&["rules", "catch_all", "rules_on_arrival", "timed_rules"]),
            ),
            card("How every zone looks", "/desktop/zones/look", Show::AllBut(&[])),
            card("Moving and sizing a zone", "/desktop/zones/snap", Show::AllBut(&[])),
            card(
                "How they behave",
                "/desktop/zones",
                Show::AllBut(&[
                    "enabled",
                    "zones",
                    "rules",
                    "catch_all",
                    "rules_on_arrival",
                    "timed_rules",
                    "look",
                    "snap",
                ]),
            ),
        ];
        const FOCUS: &[Card] = &[card("Focus", "/desktop/focus", Show::AllBut(&[]))];
        const INPUT: &[Card] = &[
            // (Its layouts are chosen by name in the card above.)
            card(
                "Keyboard",
                "/desktop/input/keyboard",
                Show::Rest(&["layouts"]),
            ),
            card("Mouse", "/desktop/input/mouse", Show::AllBut(&[])),
            card("Touchpad", "/desktop/input/touchpad", Show::AllBut(&[])),
        ];
        const STARTUP: &[Card] = &[card("Programs", "/desktop/startup", Show::Place)];
        // (Three switches anybody may want before a list of variables
        // few write.)
        const SESSION: &[Card] = &[
            card(
                "When gevrek starts",
                "/desktop",
                Show::Only(&[
                    "manage_existing_on_startup",
                    "restore_session_on_startup",
                    "watch_configuration",
                ]),
            ),
            card("Environment", "/desktop/session", Show::AllBut(&[])),
        ];
        const LINUX: Card = card(
            "Linux (Wayland)",
            "/desktop/platform/wayland",
            Show::AllBut(&["output_scale", "output_scales"]),
        );
        const WINDOWS: Card = card("Windows", "/desktop/platform/windows", Show::AllBut(&[]));
        // (The system this window runs on first: its settings are the
        // ones that do something here.)
        #[cfg(windows)]
        const SYSTEM: &[Card] = &[WINDOWS, LINUX];
        #[cfg(not(windows))]
        const SYSTEM: &[Card] = &[LINUX, WINDOWS];
        match self {
            Page::Displays => DISPLAYS,
            Page::Workspaces => WORKSPACES,
            Page::Appearance => APPEARANCE,
            Page::Animations => ANIMATIONS,
            Page::Wallpaper => WALLPAPER,
            Page::Bar => BAR,
            Page::Bindings => BINDINGS,
            Page::Gestures => GESTURES,
            Page::Rules => RULES,
            Page::Scratchpads => SCRATCHPADS,
            Page::SnapZones => SNAP_ZONES,
            Page::Zones => ZONES,
            Page::Focus => FOCUS,
            Page::Input => INPUT,
            Page::Startup => STARTUP,
            Page::Session => SESSION,
            Page::System => SYSTEM,
            _ => &[],
        }
    }

    /// Whether the page shows the setting at `pointer`.
    pub fn shows(self, pointer: &str) -> bool {
        self.cards().iter().any(|card| card.shows(pointer))
    }

    /// The page that shows the setting at `pointer`.
    pub fn of(pointer: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|page| page.shows(pointer))
    }

    /// The places whose problems count for the page's badge.
    pub fn places(self) -> Vec<String> {
        let mut places = Vec::new();
        for card in self.cards() {
            match card.show {
                Show::Place | Show::AllBut(_) | Show::Rest(_) => {
                    places.push(card.pointer.to_string())
                }
                Show::Only(names) => {
                    places.extend(names.iter().map(|name| format!("{}/{name}", card.pointer)))
                }
            }
        }
        places
    }
}

impl Card {
    pub fn shows(&self, pointer: &str) -> bool {
        let Some(rest) = pointer.strip_prefix(self.pointer) else {
            return false;
        };
        if rest.is_empty() {
            return matches!(self.show, Show::Place);
        }
        let Some(rest) = rest.strip_prefix('/') else {
            return false;
        };
        let first = rest.split('/').next().unwrap_or(rest);
        match self.show {
            Show::Place | Show::Rest(_) => true,
            Show::AllBut(skipped) => !skipped.contains(&first),
            Show::Only(names) => names.contains(&first),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_is_on_exactly_one_page() {
        let settings = gevrek_schema::catalogue();
        let mut seen = 0;
        for setting in settings
            .iter()
            .filter(|setting| setting.pointer.starts_with("/desktop/"))
        {
            // Written by gevrek, not set by a person.
            if setting.pointer == "/desktop/schema_version" {
                continue;
            }
            let pages: Vec<Page> = Page::ALL
                .into_iter()
                .filter(|page| page.shows(&setting.pointer))
                .collect();
            assert_eq!(pages.len(), 1, "{} is shown by {pages:?}", setting.pointer);
            seen += 1;
        }
        assert!(seen > 100, "{seen} settings were looked at");
    }

    #[test]
    fn every_card_names_a_place_the_schema_has() {
        for page in Page::ALL {
            for card in page.cards() {
                assert!(
                    gevrek_schema::schema().at(card.pointer).is_some(),
                    "{:?}: {} is not in the schema",
                    page,
                    card.pointer
                );
                if let Show::Only(names) | Show::AllBut(names) | Show::Rest(names) = card.show {
                    for name in names {
                        let place = format!("{}/{name}", card.pointer);
                        assert!(
                            gevrek_schema::schema().at(&place).is_some(),
                            "{page:?}: {place} is not in the schema"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_page_is_found_by_its_key() {
        for page in Page::ALL {
            assert_eq!(Page::from_key(page.key()), Some(page));
            assert!(!page.title().is_empty() && !page.subtitle().is_empty());
        }
    }
}
