//! Built-in Base16 palettes for theme-driven border and stackbar colors
//! (`palette` + `accent`).
//!
//! A theme resolves the active border color (and stackbar accent) from the
//! chosen accent slot and the inactive border color from `base03` of the
//! palette. Explicit per-color settings win: a color that still holds its
//! built-in default is treated as unset and replaced by the theme-derived
//! value at apply time, while any user-configured color is left untouched.

use crate::settings::{AppearanceProfile, StackbarProfile, ThemeProfile};

/// A named Base16 palette: 16 colors indexed by the `base00`..`base0F` slots.
pub struct Base16Palette {
    pub name: &'static str,
    pub colors: [(u8, u8, u8); 16],
}

const fn rgb(hex: u32) -> (u8, u8, u8) {
    (
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

/// The built-in palette set: a small, well-known subset of the Base16
/// collection.
pub static BASE16_PALETTES: &[Base16Palette] = &[
    Base16Palette {
        name: "Default Dark",
        colors: [
            rgb(0x181818),
            rgb(0x282828),
            rgb(0x383838),
            rgb(0x585858),
            rgb(0xb8b8b8),
            rgb(0xd8d8d8),
            rgb(0xe8e8e8),
            rgb(0xf8f8f8),
            rgb(0xab4642),
            rgb(0xdc9656),
            rgb(0xf7ca88),
            rgb(0xa1b56c),
            rgb(0x86c1b9),
            rgb(0x7cafc2),
            rgb(0xba8baf),
            rgb(0xa16946),
        ],
    },
    Base16Palette {
        name: "Ocean",
        colors: [
            rgb(0x2b303b),
            rgb(0x343d46),
            rgb(0x4f5b66),
            rgb(0x65737e),
            rgb(0xa7adba),
            rgb(0xc0c5ce),
            rgb(0xdfe1e8),
            rgb(0xeff1f5),
            rgb(0xbf616a),
            rgb(0xd08770),
            rgb(0xebcb8b),
            rgb(0xa3be8c),
            rgb(0x96b5b4),
            rgb(0x8fa1b3),
            rgb(0xb48ead),
            rgb(0xab7967),
        ],
    },
    Base16Palette {
        name: "Monokai",
        colors: [
            rgb(0x272822),
            rgb(0x383830),
            rgb(0x49483e),
            rgb(0x75715e),
            rgb(0xa59f85),
            rgb(0xf8f8f2),
            rgb(0xf5f4f1),
            rgb(0xf9f8f5),
            rgb(0xf92672),
            rgb(0xfd971f),
            rgb(0xf4bf75),
            rgb(0xa6e22e),
            rgb(0xa1efe4),
            rgb(0x66d9ef),
            rgb(0xae81ff),
            rgb(0xcc6633),
        ],
    },
    Base16Palette {
        name: "Gruvbox Dark",
        colors: [
            rgb(0x282828),
            rgb(0x3c3836),
            rgb(0x504945),
            rgb(0x665c54),
            rgb(0xbdae93),
            rgb(0xd5c4a1),
            rgb(0xebdbb2),
            rgb(0xfbf1c7),
            rgb(0xfb4934),
            rgb(0xfe8019),
            rgb(0xfabd2f),
            rgb(0xb8bb26),
            rgb(0x8ec07c),
            rgb(0x83a598),
            rgb(0xd3869b),
            rgb(0xd65d0e),
        ],
    },
    Base16Palette {
        name: "Solarized Dark",
        colors: [
            rgb(0x002b36),
            rgb(0x073642),
            rgb(0x586e75),
            rgb(0x657b83),
            rgb(0x839496),
            rgb(0x93a1a1),
            rgb(0xeee8d5),
            rgb(0xfdf6e3),
            rgb(0xdc322f),
            rgb(0xcb4b16),
            rgb(0xb58900),
            rgb(0x859900),
            rgb(0x2aa198),
            rgb(0x268bd2),
            rgb(0x6c71c4),
            rgb(0xd33682),
        ],
    },
];

/// Default accent slot when a palette is configured without an explicit
/// accent (Base16 blue, `base0D`).
pub const DEFAULT_ACCENT_INDEX: usize = 0x0D;
/// Slot used for the inactive border color (Base16 comment gray, `base03`).
pub const INACTIVE_BORDER_INDEX: usize = 0x03;

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Look up a built-in palette by name, tolerating case, spaces, hyphens, and
/// underscores ("ocean", "Ocean", "gruvbox-dark", "solarized dark", ...).
pub fn find_palette(name: &str) -> Option<&'static Base16Palette> {
    let normalized = normalize_name(name);
    BASE16_PALETTES
        .iter()
        .find(|palette| normalize_name(palette.name) == normalized)
}

/// Comma-separated list of the built-in palette names for error messages.
pub fn palette_names() -> String {
    BASE16_PALETTES
        .iter()
        .map(|palette| palette.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Parse a Base16 accent slot name (`base00`..`base0F`, case-insensitive).
pub fn accent_index(name: &str) -> Option<usize> {
    let normalized = normalize_name(name);
    let slot = normalized.strip_prefix("base")?;
    if slot.len() != 2 {
        return None;
    }
    let index = usize::from_str_radix(slot, 16).ok()?;
    (index < 16).then_some(index)
}

/// The word that, as a theme's accent, means: the colour that stands out
/// in the wallpaper.
pub const WALLPAPER_ACCENT: &str = "wallpaper";

/// Whether the theme's accent is taken from the wallpaper.
pub fn follows_wallpaper(theme: &ThemeProfile) -> bool {
    theme
        .accent
        .as_deref()
        .is_some_and(|accent| normalize_name(accent) == WALLPAPER_ACCENT)
}

/// The colour that stands out in a picture, as an accent: the hue that
/// the picture's coloured parts have most of, made bright and strong
/// enough to be read on a dark surface. `None` for a picture without
/// colour to speak of (grey, black, white). The same pixels give the same
/// colour on every system; a small copy of the picture is enough.
pub fn accent_of_pixels(pixels: impl IntoIterator<Item = (u8, u8, u8)>) -> Option<(u8, u8, u8)> {
    /// Hues are counted in parts of fifteen degrees.
    const PARTS: usize = 24;
    let mut weight = [0.0f64; PARTS];
    // Per part: the hue as a direction (so that the reds on both sides of
    // nought come together), and how strong the colour is.
    let mut across = [0.0f64; PARTS];
    let mut up = [0.0f64; PARTS];
    let mut strength = [0.0f64; PARTS];
    let mut seen = 0usize;
    for (red, green, blue) in pixels {
        seen += 1;
        let (red, green, blue) = (
            f64::from(red) / 255.0,
            f64::from(green) / 255.0,
            f64::from(blue) / 255.0,
        );
        let most = red.max(green).max(blue);
        let least = red.min(green).min(blue);
        let spread = most - least;
        let strong = if most > 0.0 { spread / most } else { 0.0 };
        // Nearly grey or nearly black says nothing of a colour.
        if strong < 0.2 || most < 0.2 {
            continue;
        }
        let hue = 60.0
            * if most == red {
                ((green - blue) / spread).rem_euclid(6.0)
            } else if most == green {
                (blue - red) / spread + 2.0
            } else {
                (red - green) / spread + 4.0
            };
        let part = ((hue / 15.0) as usize).min(PARTS - 1);
        let counts = strong * most;
        weight[part] += counts;
        across[part] += counts * hue.to_radians().cos();
        up[part] += counts * hue.to_radians().sin();
        strength[part] += counts * strong;
    }
    let all: f64 = weight.iter().sum();
    // Colour on less than a fiftieth of the picture is no colour of it.
    if seen == 0 || all < seen as f64 * 0.02 * 0.2 {
        return None;
    }
    // The part with the most, its neighbours counted half.
    let with_neighbours = |part: usize| {
        weight[part] + 0.5 * weight[(part + PARTS - 1) % PARTS] + 0.5 * weight[(part + 1) % PARTS]
    };
    let best = (0..PARTS).max_by(|one, other| {
        with_neighbours(*one)
            .partial_cmp(&with_neighbours(*other))
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    if weight[best] <= 0.0 {
        return None;
    }
    let hue = up[best].atan2(across[best]).to_degrees().rem_euclid(360.0);
    let strong = (strength[best] / weight[best]).clamp(0.55, 0.85);
    Some(from_hue(hue, strong, 0.9))
}

/// A colour by its hue (degrees), how strong and how bright it is.
fn from_hue(hue: f64, strong: f64, bright: f64) -> (u8, u8, u8) {
    let sixth = (hue / 60.0).rem_euclid(6.0);
    let chroma = bright * strong;
    let other = chroma * (1.0 - (sixth % 2.0 - 1.0).abs());
    let (red, green, blue) = match sixth as u32 {
        0 => (chroma, other, 0.0),
        1 => (other, chroma, 0.0),
        2 => (0.0, chroma, other),
        3 => (0.0, other, chroma),
        4 => (other, 0.0, chroma),
        _ => (chroma, 0.0, other),
    };
    let lift = bright - chroma;
    let byte = |part: f64| ((part + lift) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(red), byte(green), byte(blue))
}

/// Colors derived from a configured theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColors {
    pub active_border: (u8, u8, u8),
    pub inactive_border: (u8, u8, u8),
    pub stackbar_accent: (u8, u8, u8),
}

/// Resolve a theme profile to concrete colors. Returns `None` when no
/// palette is configured (or the configured names are invalid — config
/// validation reports those loudly).
pub fn resolve_theme_colors(theme: &ThemeProfile) -> Option<ThemeColors> {
    let palette = find_palette(theme.palette.as_deref()?)?;
    let accent = theme
        .accent
        .as_deref()
        .and_then(accent_index)
        .unwrap_or(DEFAULT_ACCENT_INDEX);
    Some(ThemeColors {
        active_border: palette.colors[accent],
        inactive_border: palette.colors[INACTIVE_BORDER_INDEX],
        stackbar_accent: palette.colors[accent],
    })
}

/// An appearance as it is carried out: its theme applied (with the
/// wallpaper's colour where that is known and asked for), then high
/// contrast where that is asked for. Every place that gives workspaces
/// their borders goes through here, so that they agree.
pub fn dressed(
    appearance: &AppearanceProfile,
    wallpaper: Option<(u8, u8, u8)>,
) -> AppearanceProfile {
    let mut dressed = appearance.clone();
    apply_theme_with_wallpaper(&mut dressed, wallpaper);
    apply_high_contrast(&mut dressed);
    dressed
}

/// The borders of a desktop in high contrast: drawn, at least so wide,
/// white around the window with the keyboard and grey around the others.
pub const HIGH_CONTRAST_WIDTH: i32 = 3;
pub const HIGH_CONTRAST_ACTIVE: (u8, u8, u8) = (255, 255, 255);
pub const HIGH_CONTRAST_INACTIVE: (u8, u8, u8) = (128, 128, 128);

/// Where the configuration asks for high contrast: the borders it means,
/// in place of whatever the appearance and its theme say. (Last of what
/// is applied to an appearance.)
pub fn apply_high_contrast(appearance: &mut AppearanceProfile) {
    if !appearance.high_contrast {
        return;
    }
    appearance.border_enabled = true;
    appearance.border_width = appearance.border_width.max(HIGH_CONTRAST_WIDTH);
    appearance.active_border_color = HIGH_CONTRAST_ACTIVE;
    appearance.inactive_border_color = HIGH_CONTRAST_INACTIVE;
    appearance.active_border_alpha = 255;
    appearance.inactive_border_alpha = 255;
    appearance.stackbar.active_text_color = HIGH_CONTRAST_ACTIVE;
}

/// Apply theme-derived colors to an appearance profile in place. Colors that
/// still hold their built-in defaults are replaced; explicitly configured
/// colors (anything else) override the theme.
pub fn apply_theme_to_appearance(appearance: &mut AppearanceProfile) {
    apply_theme_with_wallpaper(appearance, None);
}

/// The same, where the wallpaper's colour is known: a theme whose accent
/// is `wallpaper` takes it for the focused border and the stackbar (with
/// or without a palette for the rest). Without a known colour such a
/// theme leaves the accent as it is.
pub fn apply_theme_with_wallpaper(
    appearance: &mut AppearanceProfile,
    wallpaper: Option<(u8, u8, u8)>,
) {
    let from_wallpaper = wallpaper.filter(|_| follows_wallpaper(&appearance.theme));
    let palette = resolve_theme_colors(&appearance.theme);
    let colors = match (palette, from_wallpaper) {
        (Some(colors), Some(accent)) => ThemeColors {
            active_border: accent,
            stackbar_accent: accent,
            ..colors
        },
        (Some(colors), None) => colors,
        (None, Some(accent)) => ThemeColors {
            active_border: accent,
            stackbar_accent: accent,
            inactive_border: appearance.inactive_border_color,
        },
        (None, None) => return,
    };
    let appearance_defaults = AppearanceProfile::default();
    let stackbar_defaults = StackbarProfile::default();
    if appearance.active_border_color == appearance_defaults.active_border_color {
        appearance.active_border_color = colors.active_border;
    }
    if appearance.inactive_border_color == appearance_defaults.inactive_border_color {
        appearance.inactive_border_color = colors.inactive_border;
    }
    if appearance.stackbar.active_text_color == stackbar_defaults.active_text_color {
        appearance.stackbar.active_text_color = colors.stackbar_accent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wallpapers_colour_is_the_hue_it_has_most_of() {
        let red_of = |(red, green, blue): (u8, u8, u8)| red > 200 && green < 120 && blue < 120;
        // Mostly a dull red, some grey, a little strong green.
        let picture = std::iter::repeat_n((150, 40, 45), 600)
            .chain(std::iter::repeat_n((128, 128, 128), 300))
            .chain(std::iter::repeat_n((20, 240, 30), 60));
        let accent = accent_of_pixels(picture).expect("a colour");
        assert!(red_of(accent), "{accent:?}");
        // The reds on both sides of nought are one hue.
        let both =
            std::iter::repeat_n((200, 30, 40), 100).chain(std::iter::repeat_n((200, 40, 30), 100));
        assert!(red_of(accent_of_pixels(both).expect("a colour")));
        // Blue.
        let (red, green, blue) =
            accent_of_pixels(std::iter::repeat_n((20, 40, 160), 50)).expect("a colour");
        assert!(
            blue > 200 && red < 120 && green < 140,
            "{:?}",
            (red, green, blue)
        );
        // Bright and strong enough whatever the picture's own colour is.
        let (red, green, blue) =
            accent_of_pixels(std::iter::repeat_n((60, 50, 40), 50)).expect("a colour");
        assert_eq!(red.max(green).max(blue), 230);
        assert!(red.min(green).min(blue) <= 104);
        // Nothing with colour: no accent.
        assert_eq!(
            accent_of_pixels(std::iter::repeat_n((90, 90, 90), 50)),
            None
        );
        assert_eq!(accent_of_pixels(std::iter::repeat_n((5, 0, 0), 50)), None);
        assert_eq!(accent_of_pixels(std::iter::empty()), None);
        // A speck of colour on a grey picture is not the picture's colour.
        let speck = std::iter::repeat_n((128, 128, 128), 5000).chain(std::iter::once((255, 0, 0)));
        assert_eq!(accent_of_pixels(speck), None);
    }

    #[test]
    fn high_contrast_sets_the_borders_whatever_else_is_said() {
        let mut appearance = AppearanceProfile::default();
        appearance.border_enabled = false;
        appearance.border_width = 1;
        appearance.active_border_color = (10, 20, 30);
        appearance.active_border_alpha = 90;
        apply_high_contrast(&mut appearance);
        // Not asked for: nothing changes.
        assert!(!appearance.border_enabled);
        assert_eq!(appearance.active_border_color, (10, 20, 30));
        assert_eq!(
            (appearance.border_width, appearance.active_border_alpha),
            (1, 90)
        );
        appearance.high_contrast = true;
        appearance.theme.palette = Some("Ocean".to_string());
        apply_theme_with_wallpaper(&mut appearance, Some((230, 40, 40)));
        apply_high_contrast(&mut appearance);
        assert!(appearance.border_enabled);
        assert_eq!(appearance.border_width, HIGH_CONTRAST_WIDTH);
        assert_eq!(appearance.active_border_color, (255, 255, 255));
        assert_eq!(appearance.inactive_border_color, (128, 128, 128));
        assert_eq!(
            (
                appearance.active_border_alpha,
                appearance.inactive_border_alpha
            ),
            (255, 255)
        );
        // A border that is wider already stays so.
        appearance.border_width = 6;
        apply_high_contrast(&mut appearance);
        assert_eq!(appearance.border_width, 6);
    }

    #[test]
    fn a_theme_can_take_its_accent_from_the_wallpaper() {
        let mut appearance = AppearanceProfile::default();
        appearance.theme.accent = Some("Wallpaper".to_string());
        assert!(follows_wallpaper(&appearance.theme));
        let defaults = AppearanceProfile::default();
        // Not known yet: as it was.
        let mut unknown = appearance.clone();
        apply_theme_with_wallpaper(&mut unknown, None);
        assert_eq!(unknown.active_border_color, defaults.active_border_color);
        // Known, without a palette: the accent only.
        let mut known = appearance.clone();
        apply_theme_with_wallpaper(&mut known, Some((230, 40, 40)));
        assert_eq!(known.active_border_color, (230, 40, 40));
        assert_eq!(known.stackbar.active_text_color, (230, 40, 40));
        assert_eq!(known.inactive_border_color, defaults.inactive_border_color);
        // With a palette: the rest from the palette.
        let mut both = appearance.clone();
        both.theme.palette = Some("Ocean".to_string());
        apply_theme_with_wallpaper(&mut both, Some((230, 40, 40)));
        assert_eq!(both.active_border_color, (230, 40, 40));
        assert_ne!(both.inactive_border_color, defaults.inactive_border_color);
        // A colour that was set by hand stays.
        let mut by_hand = appearance.clone();
        by_hand.active_border_color = (1, 2, 3);
        apply_theme_with_wallpaper(&mut by_hand, Some((230, 40, 40)));
        assert_eq!(by_hand.active_border_color, (1, 2, 3));
        // A theme that does not ask for it ignores the wallpaper.
        let mut plain = AppearanceProfile::default();
        apply_theme_with_wallpaper(&mut plain, Some((230, 40, 40)));
        assert_eq!(plain.active_border_color, defaults.active_border_color);
    }

    #[test]
    fn palette_lookup_tolerates_case_and_separators() {
        assert_eq!(find_palette("Ocean").unwrap().name, "Ocean");
        assert_eq!(find_palette("ocean").unwrap().name, "Ocean");
        assert_eq!(find_palette("default dark").unwrap().name, "Default Dark");
        assert_eq!(find_palette("gruvbox-dark").unwrap().name, "Gruvbox Dark");
        assert_eq!(
            find_palette("solarized_dark").unwrap().name,
            "Solarized Dark"
        );
        assert_eq!(find_palette("monokai").unwrap().name, "Monokai");
        assert!(find_palette("dracula").is_none());
        assert!(find_palette("").is_none());
    }

    #[test]
    fn accent_slots_parse_base16_names() {
        assert_eq!(accent_index("base00"), Some(0));
        assert_eq!(accent_index("base0d"), Some(0x0D));
        assert_eq!(accent_index("Base0F"), Some(0x0F));
        assert_eq!(accent_index("base09"), Some(9));
        assert_eq!(accent_index("base10"), None);
        assert_eq!(accent_index("red"), None);
        assert_eq!(accent_index("base0"), None);
        assert_eq!(accent_index(""), None);
    }

    #[test]
    fn theme_resolution_uses_accent_and_base03() {
        let theme = ThemeProfile {
            extra: Default::default(),
            palette: Some("Solarized Dark".to_string()),
            accent: Some("base08".to_string()),
        };
        let colors = resolve_theme_colors(&theme).unwrap();
        assert_eq!(colors.active_border, (0xdc, 0x32, 0x2f));
        assert_eq!(colors.stackbar_accent, (0xdc, 0x32, 0x2f));
        assert_eq!(colors.inactive_border, (0x65, 0x7b, 0x83));
    }

    #[test]
    fn theme_resolution_defaults_accent_to_base0d() {
        let theme = ThemeProfile {
            extra: Default::default(),
            palette: Some("ocean".to_string()),
            accent: None,
        };
        let colors = resolve_theme_colors(&theme).unwrap();
        assert_eq!(colors.active_border, (0x8f, 0xa1, 0xb3));
        assert_eq!(colors.inactive_border, (0x65, 0x73, 0x7e));
    }

    #[test]
    fn no_palette_resolves_to_nothing() {
        assert!(resolve_theme_colors(&ThemeProfile::default()).is_none());
        assert!(resolve_theme_colors(&ThemeProfile {
            extra: Default::default(),
            palette: Some("dracula".to_string()),
            accent: None,
        })
        .is_none());
    }

    #[test]
    fn explicit_colors_override_theme_derived_colors() {
        let mut appearance = AppearanceProfile {
            theme: ThemeProfile {
                extra: Default::default(),
                palette: Some("Monokai".to_string()),
                accent: None,
            },
            // Explicitly configured colors survive the theme.
            active_border_color: (1, 2, 3),
            ..AppearanceProfile::default()
        };
        apply_theme_to_appearance(&mut appearance);
        assert_eq!(appearance.active_border_color, (1, 2, 3));
        // Defaulted colors are replaced by the theme.
        assert_eq!(appearance.inactive_border_color, (0x75, 0x71, 0x5e));
        assert_eq!(appearance.stackbar.active_text_color, (0x66, 0xd9, 0xef));
    }

    #[test]
    fn theme_applies_all_derived_colors_when_untouched() {
        let mut appearance = AppearanceProfile {
            theme: ThemeProfile {
                extra: Default::default(),
                palette: Some("Gruvbox Dark".to_string()),
                accent: Some("base0b".to_string()),
            },
            ..AppearanceProfile::default()
        };
        apply_theme_to_appearance(&mut appearance);
        assert_eq!(appearance.active_border_color, (0xb8, 0xbb, 0x26));
        assert_eq!(appearance.inactive_border_color, (0x66, 0x5c, 0x54));
        assert_eq!(appearance.stackbar.active_text_color, (0xb8, 0xbb, 0x26));
    }
}
