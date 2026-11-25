//! The stackbar: the strip of tabs over a container that holds several
//! windows. On Wayland the compositor paints it itself, into a small
//! picture that the renderer shows like any other surface.

use gevrek_model::settings::StackbarProfile;

/// Each glyph of the built-in 8x8 font is drawn this many times larger
/// (times the scale of the picture).
const GLYPH_SCALE: i32 = 2;
/// The height of a title's letters, as a part of the bar's height (16
/// pixels in a bar of 40).
const TEXT_HEIGHT: f32 = 0.4;
/// Space between the edge of a tab and its title.
const PADDING: i32 = 10;
/// Height of the line that marks the focused tab.
const ACCENT: i32 = 3;

/// Where each tab starts and how wide it is: the bar divided evenly, the
/// first tabs one pixel wider when it does not divide (as on Windows).
pub fn tab_spans(bar_width: i32, tab_count: usize) -> Vec<(i32, i32)> {
    if tab_count == 0 || bar_width <= 0 {
        return Vec::new();
    }
    let count = tab_count as i32;
    let base = bar_width / count;
    let remainder = bar_width % count;
    let mut tabs = Vec::with_capacity(tab_count);
    let mut offset = 0;
    for index in 0..count {
        let width = base + i32::from(index < remainder);
        tabs.push((offset, width));
        offset += width;
    }
    tabs
}

/// The tab at a horizontal position of the bar.
pub fn tab_at(bar_width: i32, tab_count: usize, x: i32) -> Option<usize> {
    tab_spans(bar_width, tab_count)
        .iter()
        .position(|(start, width)| x >= *start && x < start + width)
}

/// The 8 rows of a character in the built-in font; bit 0 is the leftmost
/// pixel. Letters the font does not have are shown without their mark
/// (ğ as g, ş as s, ı as i), anything else as `?`.
fn glyph(character: char) -> [u8; 8] {
    use font8x8::UnicodeFonts;
    let plain = match character {
        'ğ' => 'g',
        'Ğ' => 'G',
        'ş' => 's',
        'Ş' => 'S',
        'ı' => 'i',
        'İ' => 'I',
        'ç' | 'Ç' | 'ö' | 'Ö' | 'ü' | 'Ü' => character,
        other => other,
    };
    font8x8::BASIC_FONTS
        .get(plain)
        .or_else(|| font8x8::LATIN_FONTS.get(plain))
        .or_else(|| font8x8::BASIC_FONTS.get('?'))
        .unwrap_or([0; 8])
}

/// The font of the titles: a sans-serif font of the system, looked for
/// once. `GEVREK_FONT` names a font file to use instead. Without any, the
/// titles are written in the built-in 8x8 font.
pub fn title_font() -> Option<&'static ab_glyph::FontVec> {
    static FONT: std::sync::OnceLock<Option<ab_glyph::FontVec>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Some(path) = std::env::var_os("GEVREK_FONT") {
            candidates.push(path.into());
        }
        // What fontconfig gives for "sans", where it is installed.
        if let Ok(output) = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", "sans"])
            .output()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if output.status.success() && !path.is_empty() {
                candidates.push(path.into());
            }
        }
        for path in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        ] {
            candidates.push(path.into());
        }
        for path in candidates {
            // A collection (.ttc) is read as its first font.
            let font = std::fs::read(&path)
                .ok()
                .and_then(|bytes| ab_glyph::FontVec::try_from_vec(bytes).ok());
            if let Some(font) = font {
                log::info!("Stackbar titles are written in {}", path.display());
                return Some(font);
            }
        }
        log::warn!("No font found for the stackbar titles: using the built-in one");
        None
    })
    .as_ref()
}

/// The font of the titles where the stackbar's settings name a family:
/// that family's regular face, as fontconfig finds it (read once for each
/// family). Where none is named, or fontconfig has no such family,
/// [`title_font`].
pub fn font_of(family: &str) -> Option<&'static ab_glyph::FontVec> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Found = Mutex<HashMap<String, Option<&'static ab_glyph::FontVec>>>;
    static FOUND: OnceLock<Found> = OnceLock::new();
    let family = family.trim();
    if family.is_empty() {
        return title_font();
    }
    let Ok(mut found) = FOUND.get_or_init(Default::default).lock() else {
        return title_font();
    };
    let font = *found.entry(family.to_owned()).or_insert_with(|| {
        let output = std::process::Command::new("fc-match")
            .args([
                "-f",
                "%{family}\t%{file}",
                &format!("{family}:style=Regular"),
            ])
            .output()
            .ok()?;
        let said = String::from_utf8_lossy(&output.stdout).into_owned();
        let (families, path) = said.split_once('\t')?;
        // fontconfig gives another family where it has not this one.
        if !families
            .split(',')
            .any(|one| one.trim().eq_ignore_ascii_case(family))
        {
            log::warn!(
                "No font of the family {family:?} for the stackbar titles: the system's is used"
            );
            return None;
        }
        let font = std::fs::read(path)
            .ok()
            .and_then(|bytes| ab_glyph::FontVec::try_from_vec(bytes).ok())?;
        log::info!("Stackbar titles are written in {path}");
        Some(&*Box::leak(Box::new(font)))
    });
    font.or_else(title_font)
}

/// Paint a bar of `width` x `height` logical pixels for an output at
/// `scale`: the picture is `scale` times as large each way, four bytes a
/// pixel in the order blue, green, red, alpha (the renderer's `Argb8888`).
/// The titles are written in the font the profile names, or the system's.
pub fn paint(
    width: i32,
    height: i32,
    titles: &[String],
    focused_tab: usize,
    profile: &StackbarProfile,
    scale: i32,
) -> Vec<u8> {
    paint_with(
        width,
        height,
        titles,
        focused_tab,
        profile,
        scale,
        font_of(&profile.font),
    )
}

/// As `paint`, in the given font (`None`: the built-in one).
pub fn paint_with(
    width: i32,
    height: i32,
    titles: &[String],
    focused_tab: usize,
    profile: &StackbarProfile,
    scale: i32,
    font: Option<&ab_glyph::FontVec>,
) -> Vec<u8> {
    let scale = scale.max(1);
    let (width, height) = (width.max(1) * scale, height.max(1) * scale);
    let (padding, accent) = (PADDING * scale, ACCENT * scale);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut put = |x: i32, y: i32, color: (u8, u8, u8)| {
        if x >= 0 && x < width && y >= 0 && y < height {
            let at = ((y * width + x) * 4) as usize;
            pixels[at] = color.2;
            pixels[at + 1] = color.1;
            pixels[at + 2] = color.0;
            pixels[at + 3] = 255;
        }
    };
    let background = profile.background_color;
    // The focused tab is a little lighter than the others.
    let lift = |value: u8| value.saturating_add(18);
    let focused_background = (lift(background.0), lift(background.1), lift(background.2));
    let separator = (
        background.0.saturating_add(40),
        background.1.saturating_add(40),
        background.2.saturating_add(40),
    );

    // The titles in the system's font are written once the tabs are filled.
    let mut texts = Vec::new();
    for (index, (start, tab_width)) in tab_spans(width, titles.len()).into_iter().enumerate() {
        let focused = index == focused_tab;
        let fill = if focused {
            focused_background
        } else {
            background
        };
        for y in 0..height {
            for x in start..start + tab_width {
                put(x, y, fill);
            }
        }
        if focused {
            for y in (height - accent).max(0)..height {
                for x in start..start + tab_width {
                    put(x, y, profile.active_text_color);
                }
            }
        }
        if index > 0 {
            for y in 0..height {
                for x in start..start + scale {
                    put(x, y, separator);
                }
            }
        }

        // The title, cut where the tab ends.
        let color = if focused {
            profile.active_text_color
        } else {
            profile.inactive_text_color
        };
        let (left, right) = (start + padding, start + tab_width - padding);
        if let Some(font) = font {
            texts.push((left, right, fill, color, titles[index].as_str(), font));
            continue;
        }
        let dot = GLYPH_SCALE * scale;
        let glyph_width = 8 * dot;
        let room = ((right - left) / glyph_width).max(0) as usize;
        let top = (height - glyph_width) / 2;
        for (position, character) in titles[index].chars().take(room).enumerate() {
            let rows = glyph(character);
            let left = left + position as i32 * glyph_width;
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..8 {
                    if bits & (1 << column) == 0 {
                        continue;
                    }
                    for dy in 0..dot {
                        for dx in 0..dot {
                            put(left + column * dot + dx, top + row as i32 * dot + dy, color);
                        }
                    }
                }
            }
        }
    }
    for (left, right, fill, color, title, font) in texts {
        write_title(
            &mut pixels,
            width,
            height,
            left,
            right,
            fill,
            color,
            title,
            font,
        );
    }
    pixels
}

/// Write `title` between `left` and `right` on a background of `fill`,
/// centred in the height of the picture, cut at the last letter that fits
/// whole.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_title(
    pixels: &mut [u8],
    width: i32,
    height: i32,
    left: i32,
    right: i32,
    fill: (u8, u8, u8),
    color: (u8, u8, u8),
    title: &str,
    font: &ab_glyph::FontVec,
) {
    use ab_glyph::{Font, ScaleFont};
    let size = ab_glyph::PxScale::from((height as f32 * TEXT_HEIGHT).max(6.0));
    let scaled = font.as_scaled(size);
    let baseline = (height as f32 - (scaled.ascent() - scaled.descent())) / 2.0 + scaled.ascent();
    let mut pen = left as f32;
    let mut previous = None;
    for character in title.chars() {
        let id = scaled.glyph_id(character);
        if let Some(previous) = previous {
            pen += scaled.kern(previous, id);
        }
        let advance = scaled.h_advance(id);
        if pen + advance > right as f32 {
            break;
        }
        let glyph = id.with_scale_and_position(size, ab_glyph::point(pen, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                let (x, y) = (
                    bounds.min.x as i32 + x as i32,
                    bounds.min.y as i32 + y as i32,
                );
                if x < left || x >= right.min(width) || y < 0 || y >= height {
                    return;
                }
                let coverage = coverage.clamp(0.0, 1.0);
                let mix = |text: u8, back: u8| {
                    (f32::from(back) + (f32::from(text) - f32::from(back)) * coverage).round() as u8
                };
                let at = ((y * width + x) * 4) as usize;
                pixels[at] = mix(color.2, fill.2);
                pixels[at + 1] = mix(color.1, fill.1);
                pixels[at + 2] = mix(color.0, fill.0);
            });
        }
        pen += advance;
        previous = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> StackbarProfile {
        StackbarProfile {
            extra: Default::default(),
            enabled: true,
            height: 40,
            background_color: (32, 32, 32),
            active_text_color: (0, 170, 255),
            inactive_text_color: (180, 180, 180),
            font: String::new(),
        }
    }

    fn pixel(pixels: &[u8], width: i32, x: i32, y: i32) -> (u8, u8, u8) {
        let at = ((y * width + x) * 4) as usize;
        (pixels[at + 2], pixels[at + 1], pixels[at])
    }

    #[test]
    fn tabs_share_the_bar_and_a_position_finds_its_tab() {
        assert_eq!(tab_spans(300, 3), [(0, 100), (100, 100), (200, 100)]);
        assert_eq!(tab_spans(301, 3), [(0, 101), (101, 100), (201, 100)]);
        assert_eq!(tab_at(300, 3, 0), Some(0));
        assert_eq!(tab_at(300, 3, 150), Some(1));
        assert_eq!(tab_at(300, 3, 299), Some(2));
        assert_eq!(tab_at(300, 3, 300), None);
        assert_eq!(tab_at(300, 0, 10), None);
    }

    #[test]
    fn the_focused_tab_is_marked_and_titles_are_written_in_their_colours() {
        let profile = profile();
        let titles = vec!["editor".to_string(), "terminal".to_string()];
        let (width, height) = (400, 40);
        let pixels = paint_with(width, height, &titles, 1, &profile, 1, None);
        assert_eq!(pixels.len(), (width * height * 4) as usize);

        // The accent line is under the focused tab only.
        assert_eq!(
            pixel(&pixels, width, 300, height - 1),
            profile.active_text_color
        );
        assert_eq!(
            pixel(&pixels, width, 100, height - 1),
            profile.background_color
        );
        // Tab backgrounds: plain, and lighter where focused.
        assert_eq!(pixel(&pixels, width, 190, 2), profile.background_color);
        assert_eq!(pixel(&pixels, width, 390, 2), (50, 50, 50));

        // Each title is written in its tab, in its colour, and nowhere else.
        let count = |from: i32, to: i32, color: (u8, u8, u8)| {
            let mut found = 0;
            for y in 0..height - ACCENT {
                for x in from..to {
                    if pixel(&pixels, width, x, y) == color {
                        found += 1;
                    }
                }
            }
            found
        };
        assert!(count(0, 200, profile.inactive_text_color) > 50);
        assert_eq!(count(0, 200, profile.active_text_color), 0);
        assert!(count(201, 400, profile.active_text_color) > 50);
        assert_eq!(count(201, 400, profile.inactive_text_color), 0);
    }

    #[test]
    fn a_long_title_stays_inside_its_tab_and_unknown_letters_do_not_break_it() {
        let profile = profile();
        let titles = vec!["x".repeat(200), "ğüşiöç — Ω".to_string()];
        let (width, height) = (240, 40);
        let pixels = paint_with(width, height, &titles, 0, &profile, 1, None);
        // Nothing of the first title crosses into the second tab's padding.
        for y in 0..height - ACCENT {
            for x in 120 - PADDING..121 {
                assert_ne!(
                    pixel(&pixels, width, x, y),
                    profile.active_text_color,
                    "{x},{y}"
                );
            }
        }
        // The second title is drawn (letters without their marks, or `?`).
        let mut drawn = 0;
        for y in 0..height {
            for x in 121..240 {
                if pixel(&pixels, width, x, y) == profile.inactive_text_color {
                    drawn += 1;
                }
            }
        }
        assert!(drawn > 50);
    }

    #[test]
    fn a_picture_for_a_scaled_output_is_larger_and_drawn_anew() {
        let profile = profile();
        let titles = vec!["editor".to_string(), "terminal".to_string()];
        let once = paint_with(400, 40, &titles, 1, &profile, 1, None);
        let twice = paint_with(400, 40, &titles, 1, &profile, 2, None);
        assert_eq!(twice.len(), once.len() * 4);
        // The same picture, each pixel four times.
        for (x, y) in [(15, 14), (20, 20), (300, 39), (390, 2), (200, 10)] {
            assert_eq!(
                pixel(&twice, 800, 2 * x, 2 * y),
                pixel(&once, 400, x, y),
                "{x},{y}"
            );
            assert_eq!(
                pixel(&twice, 800, 2 * x + 1, 2 * y + 1),
                pixel(&once, 400, x, y)
            );
        }
    }

    /// With a font of the system (skipped where there is none).
    #[test]
    fn titles_in_the_system_font_keep_their_letters_and_stay_in_their_tabs() {
        let Some(font) = title_font() else {
            eprintln!("no system font: not checked");
            return;
        };
        let profile = profile();
        let (width, height) = (240, 40);
        let text = |title: &str, scale: i32| {
            let titles = vec![title.to_string(), "x".repeat(200)];
            paint_with(width, height, &titles, 0, &profile, scale, Some(font))
        };
        let inked = |pixels: &[u8], scale: i32, from: i32, to: i32| {
            let mut found = 0;
            for y in 0..(height - ACCENT) * scale {
                for x in from * scale..to * scale {
                    let colour = pixel(pixels, width * scale, x, y);
                    if colour != (50, 50, 50) && colour != profile.background_color {
                        found += 1;
                    }
                }
            }
            found
        };
        // Letters with marks are their own letters, not the plain ones.
        let marked = text("\u{15f}\u{11f}\u{131}", 1);
        let plain = text("sgi", 1);
        assert!(inked(&marked, 1, 0, 120) > 20);
        assert_ne!(marked, plain);
        // The long title of the second tab ends before its padding does,
        // and nothing of it is in the first tab.
        assert!(inked(&marked, 1, 122, 240) > 50);
        assert_eq!(inked(&marked, 1, 240 - PADDING, 240), 0);
        assert_eq!(inked(&text("", 1), 1, 0, 120), 0);
        // At scale 2 the letters are drawn with four times the pixels,
        // not enlarged.
        let large = text("\u{15f}\u{11f}\u{131}", 2);
        let ink = inked(&large, 2, 0, 120);
        assert!(ink > 2 * inked(&marked, 1, 0, 120), "{ink}");
    }
}
