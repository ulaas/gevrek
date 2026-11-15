//! Pictures of what the settings do, drawn by the code that does it.
//!
//! The border, the glow and the shadow around the two sample windows are
//! painted by the rasteriser the window manager itself uses on every
//! system (`gevrek_engine::decoration_render`), from the configuration in
//! the window: what is shown here is what will be on the screen, not a
//! likeness of it.

use egui::{Color32, Ui};
use gevrek_engine::decoration_render::{self, DecoratedWindow, DecorationSpec};
use gevrek_model::settings::AppearanceProfile;
use gevrek_ui::theme;

/// The pictures of the two windows, kept until what they show changes.
#[derive(Default)]
pub struct AppearancePreview {
    painted: Vec<(DecorationSpec, egui::TextureHandle)>,
}

/// The stage the windows stand on, and the windows, in points.
const STAGE: egui::Vec2 = egui::vec2(640.0, 300.0);
const BACK: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(300.0, 46.0), egui::pos2(570.0, 226.0));
const FRONT: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(70.0, 78.0), egui::pos2(360.0, 258.0));

impl AppearancePreview {
    pub fn show(&mut self, ui: &mut Ui, appearance: &AppearanceProfile) {
        let palette = theme::pal(ui.ctx());
        let width = STAGE.x.min(ui.available_width());
        let (stage, _) = ui.allocate_exact_size(egui::vec2(width, STAGE.y), egui::Sense::hover());
        let painter = ui.painter_at(stage);
        // The desktop behind the windows: the colour gevrek clears with,
        // lightened a little so a dark shadow can be seen on it.
        let desktop = if palette.dark {
            Color32::from_rgb(30, 31, 40)
        } else {
            Color32::from_rgb(214, 218, 228)
        };
        painter.rect_filled(stage, 10.0, desktop);
        let mut used = Vec::new();
        for (rect, focused) in [(BACK, false), (FRONT, true)] {
            let rect = rect.translate(stage.min.to_vec2());
            self.window(ui, &painter, appearance, rect, focused, &mut used);
        }
        // Pictures of settings that are gone are let go.
        self.painted.retain(|(spec, _)| used.contains(spec));
    }

    fn window(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        appearance: &AppearanceProfile,
        rect: egui::Rect,
        focused: bool,
        used: &mut Vec<DecorationSpec>,
    ) {
        let palette = theme::pal(ui.ctx());
        let effects = &appearance.effects;
        let (colour, alpha) = if focused {
            (
                appearance.active_border_color,
                appearance.active_border_alpha,
            )
        } else {
            (
                appearance.inactive_border_color,
                appearance.inactive_border_alpha,
            )
        };
        let width = if appearance.border_enabled {
            appearance.border_width.max(0)
        } else {
            0
        };
        let radius = effects.corner_radius.0 as f32;
        if DecorationSpec::wanted(effects) {
            let spec = DecorationSpec::for_window(
                effects,
                DecoratedWindow {
                    frame: (rect.width() as i32, rect.height() as i32),
                    border_width: width,
                    border_offset: appearance.border_offset,
                    border_color: colour,
                    border_alpha: alpha,
                    focused,
                },
                effects.gradient.angle,
            );
            let texture = match self.painted.iter().find(|(painted, _)| *painted == spec) {
                Some((_, texture)) => texture.clone(),
                None => {
                    let pixels = decoration_render::render(&spec);
                    // `0xAARRGGBB`, the colour already multiplied by its alpha.
                    let bytes: Vec<u8> = pixels
                        .iter()
                        .flat_map(|pixel| {
                            let [blue, green, red, alpha] = pixel.to_le_bytes();
                            [red, green, blue, alpha]
                        })
                        .collect();
                    let image = egui::ColorImage::from_rgba_premultiplied(
                        [spec.width().max(1) as usize, spec.height().max(1) as usize],
                        &bytes,
                    );
                    let name = if focused {
                        "preview-focused"
                    } else {
                        "preview-unfocused"
                    };
                    let texture = ui
                        .ctx()
                        .load_texture(name, image, egui::TextureOptions::LINEAR);
                    self.painted.push((spec.clone(), texture.clone()));
                    texture
                }
            };
            let around = rect.expand(spec.margin as f32);
            painter.image(
                texture.id(),
                around,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            used.push(spec);
        } else if width > 0 {
            // The plain border: one colour, as wide as it is set.
            let stroke = egui::Stroke::new(
                width as f32,
                Color32::from_rgba_unmultiplied(colour.0, colour.1, colour.2, alpha),
            );
            painter.rect_stroke(
                rect.expand(appearance.border_offset as f32),
                radius,
                stroke,
                egui::StrokeKind::Outside,
            );
        }
        // The window itself: as see-through as the settings make it.
        let opacity = if !appearance.transparency_enabled {
            255
        } else if focused || !appearance.dim_inactive {
            appearance.window_opacity
        } else {
            appearance.inactive_window_opacity
        };
        let body = if palette.dark {
            Color32::from_rgb(40, 42, 54)
        } else {
            Color32::from_rgb(250, 250, 252)
        };
        painter.rect_filled(
            rect,
            radius,
            body.gamma_multiply(f32::from(opacity) / 255.0),
        );
        // A hint of content, so it reads as a window.
        let ink = if palette.dark {
            Color32::from_gray(200)
        } else {
            Color32::from_gray(60)
        };
        let faint = ink.gamma_multiply(if focused { 0.55 } else { 0.25 });
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(16.0, 16.0),
            egui::vec2(rect.width() * 0.45, 8.0),
        );
        painter.rect_filled(bar, 4.0, faint);
        for line in 0..4 {
            let at = rect.min + egui::vec2(16.0, 42.0 + line as f32 * 18.0);
            let long = rect.width() * [0.78, 0.62, 0.7, 0.4][line];
            painter.rect_filled(
                egui::Rect::from_min_size(at, egui::vec2(long, 6.0)),
                3.0,
                faint.gamma_multiply(0.6),
            );
        }
        // The tint over a window, when it is on.
        if effects.tint.enabled {
            let strength = if focused {
                effects.tint.active_opacity
            } else {
                effects.tint.inactive_opacity
            };
            let (red, green, blue) = effects.tint.color;
            painter.rect_filled(
                rect,
                radius,
                Color32::from_rgba_unmultiplied(red, green, blue, strength),
            );
        }
    }
}

/// Where a workspace puts `windows` windows on a display of `area`
/// (width, height): the rectangles the tiling engine itself computes for
/// the workspace as the configuration describes it, in the order the
/// windows were opened.
pub fn layout_rects(
    profile: &gevrek_model::settings::WorkspaceProfile,
    appearance: &AppearanceProfile,
    windows: usize,
    area: (i32, i32),
) -> Vec<gevrek_engine::Rect> {
    use gevrek_engine::workspace_profile::WorkspaceProfileApply;
    // A workspace whose windows float tiles nothing: each window opens in
    // the middle, two thirds of the display where it has no size of its
    // own. Shown so, each a little beside the one before, so that they
    // can be counted (on the desktop they lie on one another).
    if profile.layout == gevrek_model::layout::LayoutType::Floating {
        let size = (area.0 * 2 / 3, area.1 * 2 / 3);
        let step = ((area.0 - size.0) / 2 / windows.max(1) as i32).clamp(0, 18);
        let first = (
            (area.0 - size.0) / 2 - step * (windows as i32 - 1) / 2,
            (area.1 - size.1) / 2 - step * (windows as i32 - 1) / 2,
        );
        // (The first, the one with the keys, is drawn last: on top.)
        return (0..windows as i32)
            .map(|index| {
                gevrek_engine::Rect::new(
                    (first.0 + step * (windows as i32 - 1 - index)).max(0),
                    (first.1 + step * (windows as i32 - 1 - index)).max(0),
                    size.0,
                    size.1,
                )
            })
            .collect();
    }
    let mut workspace = gevrek_engine::workspace::Workspace::new(1, profile.name.clone());
    profile.apply_to_workspace(&mut workspace, appearance);
    for index in 0..windows {
        workspace.add_window(gevrek_engine::WindowHandle(index as isize + 1), true);
    }
    workspace.apply_layout(gevrek_engine::Rect::new(0, 0, area.0, area.1));
    // (Of a strip that scrolls, what is in view.)
    workspace
        .containers
        .iter()
        .filter(|container| !container.offscreen)
        .map(|container| container.rect)
        .collect()
}

/// The layout of one workspace, drawn small.
pub struct LayoutPreview {
    /// How many sample windows.
    pub windows: usize,
    /// Which of the workspaces of the configuration.
    pub workspace: usize,
}

impl Default for LayoutPreview {
    fn default() -> Self {
        Self {
            windows: 4,
            workspace: 0,
        }
    }
}

/// The display the sample windows are laid out on.
const DISPLAY: (i32, i32) = (1920, 1080);

impl LayoutPreview {
    pub fn show(&mut self, ui: &mut Ui, config: &gevrek_model::Config) {
        let palette = theme::pal(ui.ctx());
        let profiles = &config.desktop.workspaces;
        if profiles.is_empty() {
            ui.label(
                egui::RichText::new("No workspace is set up; gevrek makes its own.")
                    .color(palette.muted),
            );
            return;
        }
        self.workspace = self.workspace.min(profiles.len() - 1);
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("previewed workspace")
                .selected_text(format!("Workspace {}", profiles[self.workspace].name))
                .show_ui(ui, |ui| {
                    for (index, profile) in profiles.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.workspace,
                            index,
                            format!("Workspace {}", profile.name),
                        );
                    }
                });
            ui.label("with");
            ui.add(gevrek_ui::widgets::Slider::new(&mut self.windows, 1..=9).suffix(" windows"));
        });
        ui.add_space(6.0);
        let profile = &profiles[self.workspace];
        let rects = layout_rects(profile, &config.desktop.appearance, self.windows, DISPLAY);
        let floats = profile.layout == gevrek_model::layout::LayoutType::Floating;
        let width = 560.0_f32.min(ui.available_width());
        let scale = width / DISPLAY.0 as f32;
        let (stage, _) = ui.allocate_exact_size(
            egui::vec2(width, DISPLAY.1 as f32 * scale),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(stage);
        let desktop = if palette.dark {
            Color32::from_rgb(30, 31, 40)
        } else {
            Color32::from_rgb(214, 218, 228)
        };
        painter.rect_filled(stage, 6.0, desktop);
        let (red, green, blue) = config.desktop.appearance.active_border_color;
        let accent = Color32::from_rgb(red, green, blue);
        let radius = config.desktop.appearance.effects.corner_radius.0 as f32 * scale;
        // (From the last to the first: where windows lie on one another,
        // the first, which has the keys, is on top.)
        for (index, rect) in rects.iter().enumerate().rev() {
            let on_stage = egui::Rect::from_min_size(
                stage.min + egui::vec2(rect.x as f32 * scale, rect.y as f32 * scale),
                egui::vec2(rect.width as f32 * scale, rect.height as f32 * scale),
            );
            // The first window is the focused one.
            let (fill, edge) = if index == 0 {
                (theme::mix(desktop, accent, 0.28), accent)
            } else {
                (
                    theme::mix(desktop, palette.text, 0.10),
                    theme::mix(desktop, palette.text, 0.30),
                )
            };
            painter.rect_filled(on_stage, radius, fill);
            painter.rect_stroke(
                on_stage,
                radius,
                egui::Stroke::new(1.0, edge),
                egui::StrokeKind::Inside,
            );
            painter.text(
                // (In a corner where windows lie on one another.)
                if floats {
                    on_stage.left_top() + egui::vec2(12.0, 12.0)
                } else {
                    on_stage.center()
                },
                egui::Align2::CENTER_CENTER,
                (index + 1).to_string(),
                egui::FontId::proportional(13.0),
                theme::readable(palette.text, fill, 3.0),
            );
        }
        ui.add_space(4.0);
        let said = if floats {
            "Floating · nothing is tiled: a window opens in the middle, at its own size, \
             above the ones before"
                .to_string()
        } else {
            format!(
                "{} · gaps {} inside, {} outside · laid out by the tiling engine itself",
                gevrek_schema::title(&format!("{:?}", profile.layout)),
                profile.gaps_in,
                profile.gaps_out
            )
        };
        ui.label(egui::RichText::new(said).color(palette.muted));
    }
}

/// The curves of the four kinds of movement, each drawn as the model
/// computes it, with a dot that moves along it in the kind's own time.
pub fn motion(ui: &mut Ui, animations: &gevrek_model::settings::AnimationProfile) {
    let palette = theme::pal(ui.ctx());
    let now = ui.input(|input| input.time);
    ui.horizontal_wrapped(|ui| {
        for (name, channel) in animations.channels() {
            ui.vertical(|ui| {
                let (plot, _) =
                    ui.allocate_exact_size(egui::vec2(150.0, 96.0), egui::Sense::hover());
                let painter = ui.painter_at(plot);
                painter.rect_filled(plot, 6.0, theme::mix(palette.surface, palette.text, 0.05));
                // Room above the line where it ends, for a curve that
                // swings past it.
                let inner = plot.shrink2(egui::vec2(10.0, 14.0));
                let at = |t: f32| {
                    let value = channel.curve.sample(t);
                    egui::pos2(
                        inner.left() + t * inner.width(),
                        inner.bottom() - value * inner.height() * 0.82,
                    )
                };
                let rest = inner.bottom() - inner.height() * 0.82;
                painter.line_segment(
                    [
                        egui::pos2(inner.left(), rest),
                        egui::pos2(inner.right(), rest),
                    ],
                    egui::Stroke::new(1.0, palette.border),
                );
                let on = animations.enabled && channel.enabled;
                let ink = if on {
                    palette.accent_fg()
                } else {
                    palette.faint
                };
                let points: Vec<egui::Pos2> = (0..=48).map(|step| at(step as f32 / 48.0)).collect();
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, ink)));
                if on && channel.duration_ms > 0 {
                    // The movement, then a pause, again and again.
                    let length = f64::from(channel.duration_ms) / 1000.0;
                    let phase = (now % (length + 0.8)) / length;
                    painter.circle_filled(at(phase.min(1.0) as f32), 4.5, palette.text);
                    ui.ctx().request_repaint();
                }
                let words = if on {
                    format!(
                        "{}  ·  {} ms",
                        gevrek_schema::title(name),
                        channel.duration_ms
                    )
                } else {
                    format!("{}  ·  off", gevrek_schema::title(name))
                };
                ui.label(egui::RichText::new(words).size(12.5).color(palette.muted));
            });
            ui.add_space(8.0);
        }
    });
}

/// How a window is drawn at a moment of its coming (`t` from 0, not
/// there, to 1, there; its going is the same backwards): how large it
/// is of its own size, how solid, and how far below its place it is, in
/// shares of its height.
pub fn window_at(style: gevrek_model::settings::WindowTransitionStyle, t: f32) -> (f32, f32, f32) {
    use gevrek_model::settings::WindowTransitionStyle as Style;
    let t = t.clamp(0.0, 1.5);
    let solid = t.clamp(0.0, 1.0);
    match style {
        Style::None => (1.0, if t > 0.0 { 1.0 } else { 0.0 }, 0.0),
        Style::Fade => (1.0, solid, 0.0),
        Style::Slide => (1.0, solid, (1.0 - t) * 0.35),
        Style::Scale => (0.7 + 0.3 * t, solid, 0.0),
    }
}

/// How two workspaces are drawn at a moment of the one giving way to
/// the other (`t` from 0 to 1): for the old one and for the new one,
/// how far it is from its place sideways and downwards (in shares of
/// the display's width and height) and how solid it is.
pub fn workspaces_at(
    style: gevrek_model::settings::WorkspaceTransitionStyle,
    t: f32,
) -> ((f32, f32, f32), (f32, f32, f32)) {
    use gevrek_model::settings::WorkspaceTransitionStyle as Style;
    let t = t.clamp(0.0, 1.0);
    match style {
        Style::Slide => ((-t, 0.0, 1.0), (1.0 - t, 0.0, 1.0)),
        Style::SlideVertical => ((0.0, -t, 1.0), (0.0, 1.0 - t, 1.0)),
        Style::Fade => ((0.0, 0.0, 1.0 - t), (0.0, 0.0, t)),
    }
}

/// Where a movement is at a time: it runs for `length` seconds, rests
/// for `rest`, runs back, rests again, and so on. Its progress from 0 to
/// 1, and whether it is the way back.
pub fn there_and_back(now: f64, length: f64, rest: f64) -> (f32, bool) {
    let length = length.max(0.001);
    let round = 2.0 * (length + rest);
    let at = now.rem_euclid(round);
    let (within, back) = if at < length + rest {
        (at, false)
    } else {
        (at - length - rest, true)
    };
    ((within / length).min(1.0) as f32, back)
}

/// A window coming and going, and one workspace giving way to another,
/// as the settings have them: small displays on which it happens again
/// and again, in the settings' own time and along their own curves.
pub fn scenes(ui: &mut Ui, animations: &gevrek_model::settings::AnimationProfile) {
    let palette = theme::pal(ui.ctx());
    let now = ui.input(|input| input.time);
    let size = egui::vec2(230.0, 132.0);
    let on = |channel: &gevrek_model::settings::AnimationChannelProfile| {
        animations.enabled && channel.enabled && channel.duration_ms > 0
    };
    let said = |ui: &mut Ui, words: String| {
        ui.label(egui::RichText::new(words).size(12.5).color(palette.muted));
    };
    ui.horizontal_wrapped(|ui| {
        // A window opens and closes.
        ui.vertical(|ui| {
            let (display, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter_at(display);
            painter.rect_filled(
                display,
                6.0,
                theme::mix(palette.surface, palette.text, 0.05),
            );
            let place = display.shrink2(egui::vec2(40.0, 24.0));
            let channel = &animations.window;
            let length = f64::from(channel.duration_ms) / 1000.0;
            let (t, going) = if on(channel) {
                ui.ctx().request_repaint();
                there_and_back(now, length, 0.9)
            } else {
                // (Without movement: there for a while, gone for a while.)
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(250));
                (1.0, now.rem_euclid(2.4) > 1.2)
            };
            let style = if going {
                animations.window_exit_style
            } else {
                animations.window_enter_style
            };
            let moved = if on(channel) {
                channel.curve.sample(t)
            } else {
                1.0
            };
            let (scale, solid, below) = window_at(style, if going { 1.0 - moved } else { moved });
            if solid > 0.0 {
                let drawn = egui::Rect::from_center_size(
                    place.center() + egui::vec2(0.0, below * place.height()),
                    place.size() * scale,
                );
                painter.rect_filled(drawn, 5.0, palette.accent.gamma_multiply(0.55 * solid));
                painter.rect_stroke(
                    drawn,
                    5.0,
                    egui::Stroke::new(1.5, palette.accent.gamma_multiply(solid)),
                    egui::StrokeKind::Inside,
                );
            }
            let style_word = |style: gevrek_model::settings::WindowTransitionStyle| {
                serde_json::to_value(style)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            said(
                ui,
                if on(channel) {
                    format!(
                        "A window: in by {}, out by {}",
                        style_word(animations.window_enter_style),
                        style_word(animations.window_exit_style)
                    )
                } else {
                    "A window: there at once, gone at once".to_string()
                },
            );
        });
        ui.add_space(8.0);
        // One workspace gives way to another.
        ui.vertical(|ui| {
            let (display, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter_at(display);
            painter.rect_filled(
                display,
                6.0,
                theme::mix(palette.surface, palette.text, 0.05),
            );
            let channel = &animations.workspace;
            // (Nothing moves across the screen where movement is kept
            // to what is needed.)
            let moves = on(channel) && !animations.reduce_motion;
            let length = f64::from(channel.duration_ms) / 1000.0;
            let (t, back) = if moves {
                ui.ctx().request_repaint();
                there_and_back(now, length, 0.9)
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(250));
                (1.0, now.rem_euclid(2.4) > 1.2)
            };
            let moved = if moves { channel.curve.sample(t) } else { 1.0 };
            let (old, new) = workspaces_at(animations.workspace_style, moved);
            // Two workspaces, each with its own windows and colour; on
            // the way back they change parts.
            let colours = [palette.accent, palette.blue];
            let (leaving, coming) = if back { (1, 0) } else { (0, 1) };
            for (which, (across, down, solid)) in [(leaving, old), (coming, new)] {
                if solid <= 0.0 {
                    continue;
                }
                let shifted = display.translate(egui::vec2(
                    across * display.width(),
                    down * display.height(),
                ));
                let inner = shifted.shrink(10.0);
                let colour = colours[which].gamma_multiply(0.5 * solid);
                // (The first workspace: two beside one another; the
                // second: one over two.)
                let tiles: Vec<egui::Rect> = if which == 0 {
                    let (left, right) = inner.split_left_right_at_fraction(0.55);
                    vec![left.shrink(3.0), right.shrink(3.0)]
                } else {
                    let (top, bottom) = inner.split_top_bottom_at_fraction(0.5);
                    let (left, right) = bottom.split_left_right_at_fraction(0.5);
                    vec![top.shrink(3.0), left.shrink(3.0), right.shrink(3.0)]
                };
                for tile in tiles {
                    painter.rect_filled(tile, 4.0, colour);
                }
            }
            let style = serde_json::to_value(animations.workspace_style)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            said(
                ui,
                if moves {
                    format!("Workspaces: by {style}")
                } else if animations.reduce_motion && on(channel) {
                    "Workspaces: at once (movement is kept low)".to_string()
                } else {
                    "Workspaces: at once".to_string()
                },
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gevrek_model::layout::LayoutType;

    #[test]
    fn the_engine_lays_the_sample_windows_out_inside_the_display() {
        let config = gevrek_model::examples::starting_configuration();
        let mut profile = config
            .desktop
            .workspaces
            .first()
            .cloned()
            .unwrap_or_default();
        for layout in [
            LayoutType::Dwindle,
            LayoutType::BSP,
            LayoutType::Grid,
            LayoutType::Columns,
            LayoutType::Rows,
            LayoutType::MasterStack,
            LayoutType::CenteredMaster,
        ] {
            profile.layout = layout;
            for windows in [1, 2, 5] {
                let rects = layout_rects(&profile, &config.desktop.appearance, windows, DISPLAY);
                assert_eq!(
                    rects.len(),
                    windows,
                    "{layout:?}: one place for each window"
                );
                for (index, rect) in rects.iter().enumerate() {
                    assert!(
                        rect.width > 0 && rect.height > 0,
                        "{layout:?}: window {index} has a size: {rect:?}"
                    );
                    assert!(
                        rect.x >= 0
                            && rect.y >= 0
                            && rect.x + rect.width <= DISPLAY.0
                            && rect.y + rect.height <= DISPLAY.1,
                        "{layout:?}: window {index} is on the display: {rect:?}"
                    );
                    for other in &rects[index + 1..] {
                        let apart = rect.x + rect.width <= other.x
                            || other.x + other.width <= rect.x
                            || rect.y + rect.height <= other.y
                            || other.y + other.height <= rect.y;
                        assert!(
                            apart,
                            "{layout:?} with {windows}: {rect:?} and {other:?} overlap"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_workspace_whose_windows_float_shows_them_in_the_middle() {
        let config = gevrek_model::examples::starting_configuration();
        let mut profile = config
            .desktop
            .workspaces
            .first()
            .cloned()
            .unwrap_or_default();
        profile.layout = LayoutType::Floating;
        for windows in [1, 3, 9] {
            let rects = layout_rects(&profile, &config.desktop.appearance, windows, DISPLAY);
            assert_eq!(rects.len(), windows, "one place for each window");
            for rect in &rects {
                assert_eq!(
                    (rect.width, rect.height),
                    (DISPLAY.0 * 2 / 3, DISPLAY.1 * 2 / 3)
                );
                assert!(
                    rect.x >= 0
                        && rect.y >= 0
                        && rect.x + rect.width <= DISPLAY.0
                        && rect.y + rect.height <= DISPLAY.1,
                    "on the display: {rect:?}"
                );
            }
            // Each beside the one before, so that they can be counted.
            assert!(rects.windows(2).all(|pair| pair[0].x != pair[1].x));
        }
        // One window: in the very middle.
        let one = layout_rects(&profile, &config.desktop.appearance, 1, DISPLAY)[0];
        assert_eq!(one.x * 2 + one.width, DISPLAY.0);
    }

    #[test]
    fn wider_gaps_leave_less_for_the_windows() {
        let config = gevrek_model::examples::starting_configuration();
        let mut profile = config
            .desktop
            .workspaces
            .first()
            .cloned()
            .unwrap_or_default();
        profile.layout = LayoutType::Columns;
        profile.gaps_in = 0;
        profile.gaps_out = 0;
        let tight: i32 = layout_rects(&profile, &config.desktop.appearance, 3, DISPLAY)
            .iter()
            .map(|rect| rect.width)
            .sum();
        profile.gaps_in = 40;
        profile.gaps_out = 40;
        let loose: i32 = layout_rects(&profile, &config.desktop.appearance, 3, DISPLAY)
            .iter()
            .map(|rect| rect.width)
            .sum();
        assert_eq!(tight, DISPLAY.0);
        assert!(loose < tight - 100, "{loose} against {tight}");
    }

    #[test]
    fn a_window_comes_as_its_style_says() {
        use gevrek_model::settings::WindowTransitionStyle as Style;
        // Not there, half there, there: size, solidness, how far below.
        assert_eq!(window_at(Style::Fade, 0.0), (1.0, 0.0, 0.0));
        assert_eq!(window_at(Style::Fade, 0.5), (1.0, 0.5, 0.0));
        assert_eq!(window_at(Style::Scale, 0.0).0, 0.7);
        assert_eq!(window_at(Style::Scale, 1.0), (1.0, 1.0, 0.0));
        assert!(window_at(Style::Slide, 0.0).2 > 0.3 && window_at(Style::Slide, 1.0).2 == 0.0);
        // Without a style: there or not, nothing in between.
        assert_eq!(window_at(Style::None, 0.0).1, 0.0);
        assert_eq!(window_at(Style::None, 0.01), (1.0, 1.0, 0.0));
    }

    #[test]
    fn a_workspace_gives_way_as_its_style_says() {
        use gevrek_model::settings::WorkspaceTransitionStyle as Style;
        // Sideways: the old one a display's width out when the new is in.
        assert_eq!(
            workspaces_at(Style::Slide, 0.0),
            ((0.0, 0.0, 1.0), (1.0, 0.0, 1.0))
        );
        assert_eq!(
            workspaces_at(Style::Slide, 1.0),
            ((-1.0, 0.0, 1.0), (0.0, 0.0, 1.0))
        );
        assert_eq!(workspaces_at(Style::SlideVertical, 1.0).0, (0.0, -1.0, 1.0));
        // Fading: both in their places, one less solid as the other is more.
        assert_eq!(
            workspaces_at(Style::Fade, 0.25),
            ((0.0, 0.0, 0.75), (0.0, 0.0, 0.25))
        );
    }

    #[test]
    fn a_scene_goes_there_rests_and_comes_back() {
        // Half a second there, one of rest, half back, one of rest.
        assert_eq!(there_and_back(0.0, 0.5, 1.0), (0.0, false));
        assert_eq!(there_and_back(0.25, 0.5, 1.0), (0.5, false));
        assert_eq!(there_and_back(1.0, 0.5, 1.0), (1.0, false));
        assert_eq!(there_and_back(1.75, 0.5, 1.0), (0.5, true));
        assert_eq!(there_and_back(3.0, 0.5, 1.0), (0.0, false));
    }
}
