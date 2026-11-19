//! The zones drawn on a display's surface, and the pointer read from it.
//!
//! A zone is a quiet sheet on the wallpaper: a ground that lets the
//! picture behind it through, a hairline around it, and a title set off
//! by a short mark in the accent colour. Nothing moves by itself and
//! nothing is drawn unless something changed, so a desktop that is left
//! alone costs nothing.

use super::gesture::{self, Doing, Key, Pointer};
use super::view::{self, DeskBox, ItemBox, Layout, ZoneBox};
use super::{Desk, Zones};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind};
use gevrek_ui::theme::Palette;
use gevrek_zones::backend::{IconHandle, IconRequest, IconSize};
use gevrek_zones::config::View;
use gevrek_zones::core::ZonesCore;
use gevrek_zones::item::{Item, ItemKind};
use gevrek_zones::{DisplayId, ItemId};
use std::collections::HashMap;

/// How a picture that was asked for stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Asked {
    /// The small picture of what the file holds was asked for.
    Thumbnail(IconRequest),
    /// There is none: the icon was asked for.
    Icon(IconRequest),
    /// The answer is there (a handle, or that there is no picture).
    There(Option<IconHandle>),
}

/// The pictures of the items: asked of the backend when an item is first
/// drawn at a size, kept for as long as the item is there.
#[derive(Default)]
pub struct Pictures {
    /// By item and by the pixels its picture is wide.
    asked: HashMap<(ItemId, u32), Asked>,
    /// What each surface draws with (a surface has its own textures).
    textures: HashMap<(DisplayId, ItemId, u32), Option<egui::TextureHandle>>,
}

impl Pictures {
    /// The backend answered a request. Says whether it was one of ours
    /// and there is now something else to draw.
    pub fn answered(&mut self, request: IconRequest, icon: Option<IconHandle>) -> bool {
        let Some((key, asked)) = self
            .asked
            .iter_mut()
            .find(|(_, asked)| matches!(asked, Asked::Thumbnail(of) | Asked::Icon(of) if *of == request))
        else {
            return false;
        };
        *asked = match (*asked, icon) {
            // No thumbnail: the icon is asked for when it is next drawn.
            (Asked::Thumbnail(_), None) => Asked::Icon(IconRequest(0)),
            (_, icon) => Asked::There(icon),
        };
        let key = key.clone();
        self.textures.retain(|(_, item, pixels), _| (item, *pixels) != (&key.0, key.1));
        true
    }

    /// The picture of an item that has come, at whichever size.
    pub fn handle_of(&self, item: &ItemId) -> Option<IconHandle> {
        self.asked.iter().find_map(|((of, _), asked)| match asked {
            Asked::There(Some(handle)) if of == item => Some(*handle),
            _ => None,
        })
    }

    /// Forget the pictures of items that are no longer there, and of
    /// those whose file changed.
    pub fn forget_gone(&mut self, core: &ZonesCore) {
        self.asked.retain(|(item, _), _| core.item(item).is_some());
        self.textures.retain(|(_, item, _), _| core.item(item).is_some());
    }

    /// The picture of an item `points` wide on a surface of this scale,
    /// if it is there; asked for if it was not yet. Crisp at any scale:
    /// asked for in the surface's own pixels.
    fn of(
        &mut self,
        context: &egui::Context,
        desk: &mut dyn Desk,
        display: &DisplayId,
        item: &ItemId,
        points: u32,
    ) -> Option<egui::TextureHandle> {
        let scale = context.pixels_per_point();
        let pixels = (points as f32 * scale).round().max(1.0) as u32;
        let size = IconSize { size: points, scale: f64::from(scale) };
        let key = (item.clone(), pixels);
        let asked = self.asked.entry(key.clone()).or_insert_with(|| Asked::Thumbnail(desk.request_thumbnail(item, size)));
        if *asked == Asked::Icon(IconRequest(0)) {
            *asked = Asked::Icon(desk.request_icon(item, size));
        }
        let Asked::There(Some(handle)) = *asked else {
            return None;
        };
        self.textures
            .entry((display.clone(), key.0, key.1))
            .or_insert_with(|| {
                let picture = desk.picture(handle)?;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [picture.width as usize, picture.height as usize],
                    &picture.pixels,
                );
                Some(context.load_texture(format!("zone-item-{}", handle.0), image, egui::TextureOptions::LINEAR))
            })
            .clone()
    }
}

/// The look of the zones on one frame.
pub struct Look<'a> {
    pub palette: &'a Palette,
    pub settings: &'a gevrek_ui::Settings,
}

impl Look<'_> {
    fn high_contrast(&self) -> bool {
        self.settings.preset.is_high_contrast()
    }

    fn label_font(&self) -> FontId {
        FontId::proportional(11.5)
    }

    fn title_font(&self) -> FontId {
        FontId::proportional(12.5)
    }
}

/// Read what the pointer and the keys did on this surface, and tell the
/// zones. A surface the pointer is not on tells nothing.
fn hear(zones: &mut Zones, ui: &egui::Ui, origin: Pos2, desk: Rect) {
    let (pointer, double, wheel, keys, typed) = ui.input(|input| {
        let at = input.pointer.latest_pos().map(|at| at + origin.to_vec2());
        let pointer = Pointer {
            at,
            pressed: input.pointer.primary_pressed(),
            released: input.pointer.primary_released(),
            down: input.pointer.primary_down(),
            double: false,
            menu: input.pointer.secondary_pressed(),
            ctrl: input.modifiers.ctrl || input.modifiers.command,
            shift: input.modifiers.shift,
            alt: input.modifiers.alt,
        };
        let mut keys = Vec::new();
        if input.focused {
            for (key, means) in [
                (egui::Key::Enter, Key::Enter),
                (egui::Key::Escape, Key::Escape),
                (egui::Key::ArrowLeft, Key::Step(-1, 0)),
                (egui::Key::ArrowRight, Key::Step(1, 0)),
                (egui::Key::ArrowUp, Key::Step(0, -1)),
                (egui::Key::ArrowDown, Key::Step(0, 1)),
            ] {
                if input.key_pressed(key) {
                    keys.push(means);
                }
            }
            if input.modifiers.command && input.key_pressed(egui::Key::A) {
                keys.push(Key::SelectAll);
            }
            for (key, means) in [(egui::Key::Delete, Key::Delete), (egui::Key::F2, Key::Rename)] {
                if input.key_pressed(key) {
                    keys.push(means);
                }
            }
        }
        // Letters, for going to an item by the beginning of its name.
        let typed: String = if input.focused && !input.modifiers.command && !input.modifiers.alt {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        } else {
            String::new()
        };
        (
            pointer,
            input.pointer.button_double_clicked(egui::PointerButton::Primary),
            input.smooth_scroll_delta.y,
            keys,
            typed,
        )
    });
    if pointer.at.is_some() || pointer.released {
        zones.pointer(&pointer);
        // (The toolkit says "twice" when the button comes up the second
        // time; to the zones it is the second press that counts.)
        if double {
            zones.pointer(&Pointer { pressed: true, down: true, double: true, released: false, ..pointer });
            zones.pointer(&Pointer { pressed: false, down: false, released: true, ..pointer });
        }
        if let (Some(at), true) = (pointer.at, wheel != 0.0) {
            zones.scroll(at, wheel);
        }
    } else if zones.hover.is_some_and(|at| desk.contains(at)) {
        // The pointer left this display.
        zones.hover = None;
        zones.redraw = true;
    }
    for key in keys {
        zones.key(key);
    }
    if !typed.trim().is_empty() {
        zones.typed(&typed);
    }
}

/// Draw the zones and the loose items of one display, and hear the
/// pointer on it. The surface's root `Ui` covers the display.
pub fn draw(zones: &mut Zones, ui: &mut egui::Ui, display: &DisplayId, look: &Look) {
    zones.set_density(look.settings.density);
    zones.surface_scale(display, ui.ctx().pixels_per_point());
    let Some(desk) = zones.layout().desks.iter().find(|desk| &desk.display == display).cloned() else {
        return;
    };
    let origin = desk.rect.min;
    hear(zones, ui, origin, desk.rect);
    paint(zones, ui, display, look);
    // While something is carried or drawn the next movement counts; at
    // rest nothing asks to be drawn again.
    if zones.gestures.busy() {
        ui.ctx().request_repaint();
    }
}

/// Everything of one display, drawn. No input is read.
pub fn paint(zones: &mut Zones, ui: &mut egui::Ui, display: &DisplayId, look: &Look) {
    let layout = zones.layout().clone();
    let Some(desk) = layout.desks.iter().find(|desk| &desk.display == display) else {
        return;
    };
    // From the desktop's units to this surface's points.
    let shift = -desk.rect.min.to_vec2();
    let screen = desk.rect.translate(shift);
    let painter = ui.painter().with_clip_rect(screen);
    let carried: Vec<(ItemId, Rect)> = match &zones.gestures.doing {
        Doing::Carrying { items, at } => gesture::carried(items, *at, &layout),
        _ => Vec::new(),
    };
    let being_carried = |item: &ItemId| carried.iter().any(|(carried, _)| carried == item);
    let mut scene = Scene { zones, ui, painter: &painter, look, display, shift };
    for item in &desk.items {
        scene.item(&painter, item, View::Grid, desk.icon_size, true, being_carried(&item.item));
    }
    for zone in layout.zones.iter().filter(|zone| zone.rect.intersects(desk.rect)) {
        scene.zone(zone, &being_carried);
    }
    scene.over(&layout, desk, &carried);
}

/// What one frame of one display is drawn with.
struct Scene<'a> {
    zones: &'a mut Zones,
    ui: &'a egui::Ui,
    painter: &'a egui::Painter,
    look: &'a Look<'a>,
    display: &'a DisplayId,
    shift: egui::Vec2,
}

impl Scene<'_> {
    fn zone(&mut self, zone: &ZoneBox, being_carried: &dyn Fn(&ItemId) -> bool) {
        let palette = self.look.palette;
        let rect = zone.rect.translate(self.shift);
        let head = zone.head.translate(self.shift);
        let strong = self.look.high_contrast();
        let radius = zone
            .look
            .corner_radius
            .map_or(f32::from(self.look.settings.radius) + 4.0, |radius| radius as f32)
            .min(rect.height() / 2.0);
        let ground = zone
            .look
            .background
            .as_deref()
            .and_then(gevrek_ui::color::parse)
            .unwrap_or(palette.surface);
        // (With high contrast nothing shows through: words are on one ground.)
        let opacity = if strong { 1.0 } else { f32::from(zone.look.opacity.unwrap_or(70).min(100)) / 100.0 };
        let hovered = self.zones.hover.is_some_and(|at| zone.rect.contains(at));
        let dropping = self.zones.incoming.is_some_and(|(at, taken)| taken && zone.rect.contains(at))
            || matches!(&self.zones.gestures.doing, Doing::Carrying { at, .. }
                if zone.rect.contains(*at) && !zone.locked_items && zone.fills != view::Fills::Query);
        self.painter.rect_filled(rect, radius, ground.gamma_multiply(opacity));
        let rim = if dropping {
            Stroke::new(1.5, palette.accent)
        } else if strong {
            Stroke::new(1.0, palette.border_strong())
        } else {
            Stroke::new(1.0, palette.border.gamma_multiply(if hovered { 0.95 } else { 0.6 }))
        };
        self.painter.rect_stroke(rect, radius, rim, StrokeKind::Inside);

        // The title: a short mark, the name, and how many it holds.
        let title_shown = !zone.look.title_on_hover.unwrap_or(false) || hovered || zone.rolled_up;
        if title_shown {
            let ink = zone
                .look
                .title_colour
                .as_deref()
                .and_then(gevrek_ui::color::parse)
                .unwrap_or(palette.text);
            let mark = Rect::from_center_size(pos2(head.left() + 12.0, head.center().y), vec2(3.0, 12.0));
            self.painter.rect_filled(mark, 1.5, palette.accent);
            let count = match zone.items.len() {
                0 => String::new(),
                count => count.to_string(),
            };
            let count_wide = if count.is_empty() { 0.0 } else { 8.0 + 7.0 * count.len() as f32 };
            let room = head.width() - 22.0 - 10.0 - count_wide;
            let font = self.look.title_font();
            let name = self.one_line(&zone.title, room, &font);
            self.painter.text(pos2(head.left() + 22.0, head.center().y), Align2::LEFT_CENTER, name, font, ink);
            if !count.is_empty() {
                self.painter.text(
                    pos2(head.right() - 10.0, head.center().y),
                    Align2::RIGHT_CENTER,
                    count,
                    FontId::proportional(11.0),
                    palette.faint,
                );
            }
            if zone.locked_place || zone.locked_items {
                // A small bolt where the count is, for a zone that is held.
                let at = pos2(head.right() - 14.0 - count_wide, head.center().y);
                self.painter.circle_stroke(at + vec2(0.0, -2.5), 2.5, Stroke::new(1.0, palette.muted));
                self.painter.rect_filled(Rect::from_center_size(at + vec2(0.0, 1.5), vec2(8.0, 6.0)), 1.5, palette.muted);
            }
        }
        if zone.rolled_up {
            // Rolled up: a small angle says there is more under the title.
            let at = pos2(head.right() - 16.0, head.center().y);
            let stroke = Stroke::new(1.3, palette.muted);
            self.painter.line_segment([at + vec2(-3.5, -1.5), at + vec2(0.0, 2.0)], stroke);
            self.painter.line_segment([at + vec2(0.0, 2.0), at + vec2(3.5, -1.5)], stroke);
            return;
        }
        let hairline = Stroke::new(1.0, palette.border.gamma_multiply(if strong { 1.0 } else { 0.45 }));
        self.painter.hline((rect.left() + 10.0)..=(rect.right() - 10.0), head.bottom() - 0.5, hairline);
        for tab in &zone.tabs {
            let place = tab.rect.translate(self.shift);
            let ink = if tab.front { palette.text } else { palette.muted };
            let font = FontId::proportional(11.5);
            let name = self.one_line(&tab.title, place.width() - 20.0, &font);
            self.painter.text(pos2(place.left() + 12.0, place.center().y), Align2::LEFT_CENTER, name, font, ink);
            if tab.front {
                let under = Rect::from_min_max(
                    pos2(place.left() + 10.0, place.bottom() - 2.0),
                    pos2(place.right() - 10.0, place.bottom()),
                );
                self.painter.rect_filled(under, 1.0, palette.accent);
            } else if self.zones.hover.is_some_and(|at| tab.rect.contains(at)) {
                self.painter.rect_filled(place.shrink2(vec2(4.0, 3.0)), 6.0, palette.text.gamma_multiply(0.07));
            }
        }
        // The items, seen through the zone's content only.
        let content = zone.content.translate(self.shift);
        let clipped = self
            .painter
            .with_clip_rect(content.expand2(vec2(2.0, 2.0)).intersect(self.painter.clip_rect()));
        for item in zone.items.iter().filter(|item| zone.content.intersects(item.rect)) {
            self.item(&clipped, item, zone.view, zone.icon_size, false, being_carried(&item.item));
        }
        if zone.scroll_extent > 0.0 && content.height() > 8.0 {
            // How much is seen of all there is, and where.
            let all = content.height() + zone.scroll_extent;
            let long = (content.height() * content.height() / all).max(18.0);
            let top = content.top() + (content.height() - long) * (zone.scroll / zone.scroll_extent);
            let bar = Rect::from_min_size(pos2(rect.right() - 6.0, top), vec2(3.0, long));
            self.painter.rect_filled(bar, 1.5, palette.text.gamma_multiply(if hovered { 0.45 } else { 0.22 }));
        }
    }

    /// One item: its ground when it is selected or under the pointer, its
    /// picture, its name. `bare`: it lies on the wallpaper, not in a zone.
    fn item(&mut self, painter: &egui::Painter, shown: &ItemBox, view: View, icon_size: u32, bare: bool, carried: bool) {
        let palette = self.look.palette;
        let rect = shown.rect.translate(self.shift);
        let selected = self.zones.gestures.selected.contains(&shown.item);
        let focused = self.zones.gestures.focus.as_ref() == Some(&shown.item) && self.ui.input(|input| input.focused);
        let hovered = self.zones.hover.is_some_and(|at| shown.rect.contains(at)) && !self.zones.gestures.busy();
        let Some(item) = self.zones.core.item(&shown.item).cloned() else {
            return;
        };
        // What is looked for dims what does not fit; what is carried is
        // faint where it was.
        let found = self.zones.filter.trim().is_empty()
            || gevrek_zones::search::score(&item.name, &self.zones.filter).is_some();
        let alpha = if carried { 0.4 } else if found { 1.0 } else { 0.38 };
        let ground = rect.shrink(1.0);
        if selected {
            painter.rect_filled(ground, 8.0, palette.accent.gamma_multiply(0.22));
            painter.rect_stroke(ground, 8.0, Stroke::new(1.0, palette.accent.gamma_multiply(0.75)), StrokeKind::Inside);
        } else if hovered {
            painter.rect_filled(ground, 8.0, palette.text.gamma_multiply(if bare { 0.12 } else { 0.07 }));
        }
        if focused {
            painter.rect_stroke(ground.expand(1.0), 9.0, Stroke::new(1.5, palette.accent), StrokeKind::Outside);
        }
        let ink = if selected { palette.text } else { palette.text.gamma_multiply(0.92) }.gamma_multiply(alpha);
        let font = self.look.label_font();
        match view {
            View::Grid => {
                let side = icon_size as f32;
                let icon = Rect::from_min_size(pos2(rect.center().x - side / 2.0, rect.top() + 4.0), vec2(side, side));
                self.picture(painter, &item, icon, icon_size, alpha);
                let mut lines = {
                    let ui = self.ui;
                    let mut measure = |letter: char| ui.fonts_mut(|fonts| fonts.glyph_width(&font, letter));
                    view::label_lines(&item.name, rect.width() - 6.0, 2, &mut measure)
                };
                lines.truncate(2);
                for (number, line) in lines.into_iter().enumerate() {
                    let at = pos2(rect.center().x, icon.bottom() + 4.0 + number as f32 * 14.0);
                    if bare && !selected {
                        // On a picture, words need a shade to stand on.
                        painter.text(at + vec2(0.0, 1.0), Align2::CENTER_TOP, &line, font.clone(), Color32::from_black_alpha((170.0 * alpha) as u8));
                    }
                    painter.text(at, Align2::CENTER_TOP, line, font.clone(), ink);
                }
            }
            View::List => {
                let side = (rect.height() - 4.0).clamp(8.0, 32.0);
                let icon = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - side / 2.0), vec2(side, side));
                self.picture(painter, &item, icon, side as u32, alpha);
                let left = icon.right() + 8.0;
                let name = self.one_line(&item.name, rect.right() - 6.0 - left, &font);
                painter.text(pos2(left, rect.center().y), Align2::LEFT_CENTER, name, font, ink);
            }
        }
    }

    /// As much of a text as fits on one line, with an ellipsis for the rest.
    fn one_line(&self, text: &str, width: f32, font: &FontId) -> String {
        let ui = self.ui;
        let mut measure = |letter: char| ui.fonts_mut(|fonts| fonts.glyph_width(font, letter));
        view::label_lines(text, width.max(8.0), 1, &mut measure).pop().unwrap_or_default()
    }

    /// An item's picture: the system's when it has come, and until then
    /// (or where there is none) a figure of our own for what it is.
    fn picture(&mut self, painter: &egui::Painter, item: &Item, place: Rect, points: u32, alpha: f32) {
        let context = self.ui.ctx().clone();
        let zones = &mut *self.zones;
        let texture = zones.pictures.of(&context, &mut *zones.desk, self.display, &item.id, points.clamp(8, 256));
        match texture {
            Some(texture) => {
                let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(texture.id(), place, whole, Color32::WHITE.linear_multiply(alpha));
            }
            None => figure(painter, self.look.palette, item, place, alpha),
        }
    }

    /// What lies over everything: items in the hand, the rectangle drawn
    /// to select with, the lines a moved zone meets, a drop from outside.
    fn over(&mut self, layout: &Layout, desk: &DeskBox, carried: &[(ItemId, Rect)]) {
        let palette = self.look.palette;
        for (item, rect) in carried {
            if !rect.intersects(desk.rect) {
                continue;
            }
            // (Where it came from decides how it is drawn while in the hand.)
            let from = layout.zones.iter().find(|zone| zone.items.iter().any(|shown| &shown.item == item));
            let (view, size) = from.map_or((View::Grid, desk.icon_size), |zone| (zone.view, zone.icon_size));
            let shown = ItemBox { item: item.clone(), rect: *rect };
            self.painter.rect_filled(rect.translate(self.shift).shrink(1.0), 8.0, palette.surface.gamma_multiply(0.55));
            let painter = self.painter;
            self.item(painter, &shown, view, size, from.is_none(), false);
        }
        if let Some(band) = self.zones.gestures.band() {
            let band = band.translate(self.shift);
            self.painter.rect_filled(band, 3.0, palette.accent.gamma_multiply(0.14));
            self.painter.rect_stroke(band, 3.0, Stroke::new(1.0, palette.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        }
        for guide in &self.zones.guides {
            let line = [guide.from + self.shift, guide.to + self.shift];
            self.painter.extend(egui::Shape::dashed_line(&line, Stroke::new(1.0, palette.accent), 6.0, 4.0));
        }
        if let Some((at, taken)) = self.zones.incoming {
            // On the bare desktop a drop that is taken shows where.
            if taken && desk.rect.contains(at) && !layout.zones.iter().any(|zone| zone.rect.contains(at)) {
                let mark = Rect::from_center_size(at + self.shift, vec2(28.0, 28.0));
                self.painter.rect_stroke(mark, 8.0, Stroke::new(1.5, palette.accent), StrokeKind::Inside);
            }
        }
    }
}

/// A figure for an item whose picture is not there: a folder, a sheet
/// with its extension written on it, a sheet with an arrow for a
/// shortcut, a rounded square for what is no file. Coloured by what it
/// is, so that a zone of papers and pictures can be told apart at a look.
fn figure(painter: &egui::Painter, palette: &Palette, item: &Item, place: Rect, alpha: f32) {
    let tones = [palette.blue, palette.purple, palette.ok, palette.warn, palette.accent];
    let extension = item.facts.extension.clone().unwrap_or_default();
    let tone = match item.kind {
        ItemKind::Folder => palette.accent,
        ItemKind::Virtual => palette.muted,
        _ => {
            let sum: usize = extension.bytes().map(usize::from).sum();
            tones[sum % tones.len()]
        }
    };
    let ink = |colour: Color32, by: f32| colour.gamma_multiply(by * alpha);
    let side = place.width().min(place.height());
    let unit = side / 16.0;
    let centre = place.center();
    let round = (2.0 * unit).max(1.5);
    match item.kind {
        ItemKind::Folder => {
            let body = Rect::from_center_size(centre + vec2(0.0, unit), vec2(13.0 * unit, 9.5 * unit));
            let tab = Rect::from_min_size(body.left_top() - vec2(0.0, 2.0 * unit), vec2(5.5 * unit, 3.5 * unit));
            painter.rect_filled(tab, round * 0.7, ink(tone, 0.75));
            painter.rect_filled(body, round, ink(tone, 0.95));
            painter.hline((body.left() + round)..=(body.right() - round), body.top() + 1.5 * unit, Stroke::new(unit.max(1.0) * 0.6, ink(palette.surface, 0.35)));
        }
        ItemKind::Virtual => {
            let body = Rect::from_center_size(centre, vec2(11.0 * unit, 11.0 * unit));
            painter.rect_filled(body, 3.0 * unit, ink(palette.control, 1.0));
            painter.rect_stroke(body, 3.0 * unit, Stroke::new(1.0, ink(tone, 0.9)), StrokeKind::Inside);
            painter.circle_filled(centre, 1.8 * unit, ink(tone, 0.9));
        }
        _ => {
            // A sheet, a corner of it folded, a band of its colour below.
            let sheet = Rect::from_center_size(centre, vec2(10.0 * unit, 13.0 * unit));
            let fold = 3.0 * unit;
            let paper = ink(palette.text, 0.88);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    sheet.left_top() + vec2(round, 0.0),
                    sheet.right_top() - vec2(fold, 0.0),
                    sheet.right_top() + vec2(0.0, fold),
                    sheet.right_bottom() - vec2(0.0, round),
                    sheet.right_bottom() - vec2(round, 0.0),
                    sheet.left_bottom() + vec2(round, 0.0),
                    sheet.left_bottom() - vec2(0.0, round),
                    sheet.left_top() + vec2(0.0, round),
                ],
                paper,
                Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![sheet.right_top() - vec2(fold, 0.0), sheet.right_top() + vec2(0.0, fold), sheet.right_top() + vec2(-fold, fold)],
                ink(tone, 0.55),
                Stroke::NONE,
            ));
            let band = Rect::from_min_max(
                pos2(sheet.left() + 1.2 * unit, sheet.bottom() - 5.2 * unit),
                pos2(sheet.right() - 1.2 * unit, sheet.bottom() - 1.4 * unit),
            );
            painter.rect_filled(band, unit * 0.9, ink(tone, 1.0));
            let word: String = extension.chars().take(4).collect::<String>().to_uppercase();
            if !word.is_empty() && side >= 28.0 {
                painter.text(
                    band.center(),
                    Align2::CENTER_CENTER,
                    word,
                    FontId::proportional((3.1 * unit).clamp(6.0, 14.0)),
                    ink(palette.on_accent(), 1.0),
                );
            }
            if item.kind == ItemKind::Shortcut || item.kind == ItemKind::Launcher {
                // A small arrow at the foot: it leads somewhere else.
                let at = sheet.left_bottom() + vec2(0.5 * unit, -0.5 * unit);
                painter.circle_filled(at, 2.6 * unit, ink(palette.surface, 1.0));
                let stroke = Stroke::new((unit * 0.9).max(1.0), ink(palette.text, 1.0));
                painter.line_segment([at + vec2(-1.2 * unit, 1.2 * unit), at + vec2(1.2 * unit, -1.2 * unit)], stroke);
                painter.line_segment([at + vec2(1.2 * unit, -1.2 * unit), at + vec2(-0.4 * unit, -1.2 * unit)], stroke);
                painter.line_segment([at + vec2(1.2 * unit, -1.2 * unit), at + vec2(1.2 * unit, 0.4 * unit)], stroke);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::zones::Spot;
    use gevrek_system::icons::Picture;
    use gevrek_zones::backend::*;
    use gevrek_zones::config::{Sort, ZoneKind, ZonesConfig};
    use gevrek_zones::core::Command;
    use gevrek_zones::geometry::{Display, ReservedArea};
    use gevrek_zones::testing::FakeBackend;
    use gevrek_zones::ZoneId;
    use std::sync::{Arc, Mutex};

    /// The backend in memory, with what a renderer asks besides: every
    /// display has a spot, and a picture file has a picture (a plain
    /// square of one colour, as large as it was asked for).
    pub struct FakeDesk {
        pub backend: FakeBackend,
        pub woken: Arc<Mutex<usize>>,
        sizes: HashMap<u64, u32>,
        /// What was asked of the system that is not a file operation:
        /// items opened, menus shown, drags begun, and what was done
        /// about its own icons.
        pub said: Arc<Mutex<Vec<String>>>,
        /// The system's own icons are put away.
        pub icons_hidden: bool,
        /// The next of each fails with this.
        pub fail_journal: Option<BackendError>,
        pub fail_hide: Option<BackendError>,
        pub fail_restore: Option<BackendError>,
    }

    impl FakeDesk {
        pub fn new(backend: FakeBackend) -> Self {
            Self {
                backend,
                woken: Arc::default(),
                sizes: HashMap::new(),
                said: Arc::default(),
                icons_hidden: false,
                fail_journal: None,
                fail_hide: None,
                fail_restore: None,
            }
        }

        fn say(&self, what: String) {
            // (Also while a test that failed is taken down.)
            self.said.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(what);
        }
    }

    impl ZonesBackend for FakeDesk {
        fn list(&mut self, location: &Location) -> Result<Vec<Item>, BackendError> {
            self.backend.list(location)
        }
        fn watch(&mut self, location: &Location) -> Result<WatchId, BackendError> {
            self.backend.watch(location)
        }
        fn unwatch(&mut self, watch: WatchId) {
            self.backend.unwatch(watch)
        }
        fn drain_events(&mut self) -> Vec<BackendEvent> {
            self.backend.drain_events()
        }
        fn facts(&mut self, item: &ItemId) -> Option<gevrek_zones::item::ItemFacts> {
            self.backend.facts(item)
        }
        fn request_icon(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
            self.backend.request_icon(item, size)
        }
        fn request_thumbnail(&mut self, item: &ItemId, size: IconSize) -> IconRequest {
            let request = self.backend.request_thumbnail(item, size);
            self.sizes.insert(request.0, (f64::from(size.size) * size.scale).round() as u32);
            request
        }
        fn open(&mut self, item: &ItemId) -> Result<(), BackendError> {
            self.say(format!("open {item}"));
            self.backend.open(item)
        }
        fn open_with(&mut self, item: &ItemId, program: Option<&str>) -> Result<(), BackendError> {
            self.backend.open_with(item, program)
        }
        fn context_menu(&mut self, items: &[ItemId], at: gevrek_model::geometry::Point) -> Result<(), BackendError> {
            self.say(format!("menu of {} at {},{}", items.len(), at.x, at.y));
            self.backend.context_menu(items, at)
        }
        fn perform(&mut self, operation: &FileOperation) -> Result<FileOperation, BackendError> {
            self.backend.perform(operation)
        }
        fn drop_payload(&mut self) -> Option<DropPayload> {
            self.backend.drop_payload()
        }
        fn start_drag(&mut self, items: &[ItemId], allowed: DragActions) -> Result<(), BackendError> {
            self.say(format!("drag {}", items.len()));
            self.backend.start_drag(items, allowed)
        }
        fn displays(&mut self) -> Vec<Display> {
            self.backend.displays()
        }
        fn reserved_areas(&mut self) -> Vec<ReservedArea> {
            self.backend.reserved_areas()
        }
        fn desktop_folder(&mut self) -> Option<String> {
            self.backend.desktop_folder()
        }
        fn journal_system_icons(&mut self) -> Result<(), BackendError> {
            self.say("journal".to_string());
            match self.fail_journal.take() {
                Some(error) => Err(error),
                None => self.backend.journal_system_icons(),
            }
        }
        fn system_icons_to_restore(&mut self) -> bool {
            self.backend.system_icons_to_restore()
        }
        fn restore_system_icons(&mut self) -> Result<(), BackendError> {
            self.say("restore".to_string());
            if let Some(error) = self.fail_restore.take() {
                return Err(error);
            }
            self.icons_hidden = false;
            self.backend.restore_system_icons()
        }
    }

    impl Desk for FakeDesk {
        fn on_news(&mut self, _wake: Box<dyn Fn() + Send>) {}

        fn hide_system_icons(&mut self) -> Result<(), BackendError> {
            self.say("hide".to_string());
            // (As the real one: not without the desktop written down.)
            if !self.backend.journal_open {
                return Err(BackendError::Refused { what: "nothing is written down".to_string() });
            }
            match self.fail_hide.take() {
                Some(error) => Err(error),
                None => {
                    self.icons_hidden = true;
                    Ok(())
                }
            }
        }

        fn picture(&mut self, icon: IconHandle) -> Option<Picture> {
            // (Only thumbnails are answered with a size; the fake's
            // icons have no pixels, and the figure is drawn.)
            let side = *self.sizes.get(&icon.0)?;
            let mut pixels = Vec::with_capacity((side * side * 4) as usize);
            for y in 0..side {
                for x in 0..side {
                    // A sky over a hill, to be told from a figure at a look.
                    let hill = y as f32 > side as f32 * 0.62 + (x as f32 * 0.12).sin() * side as f32 * 0.08;
                    pixels.extend_from_slice(if hill { &[58, 122, 84, 255] } else { &[96, 156, 214, 255] });
                }
            }
            Some(Picture { width: side, height: side, pixels })
        }

        fn spot_of(&mut self, display: &DisplayId) -> Option<Spot> {
            let display = self.backend.displays().into_iter().find(|known| &known.id == display)?;
            let rect = display.rect;
            Some(Spot {
                output: display.id.as_str().to_string(),
                pixels: (
                    (f64::from(rect.x) * display.scale) as i32,
                    (f64::from(rect.y) * display.scale) as i32,
                    (f64::from(rect.width) * display.scale) as i32,
                    (f64::from(rect.height) * display.scale) as i32,
                ),
            })
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("gevrek-shell-zones-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    /// Zones started on a desk in memory. What the zones ask to be woken
    /// for is written down, not done.
    pub fn zones_on(backend: FakeBackend, name: &str) -> (Zones, Arc<Mutex<usize>>) {
        zones_with(FakeDesk::new(backend), name)
    }

    /// As `zones_on`, on a desk that was prepared.
    pub fn zones_with(desk: FakeDesk, name: &str) -> (Zones, Arc<Mutex<usize>>) {
        let woken = Arc::clone(&desk.woken);
        let wake = Arc::clone(&woken);
        let config = ZonesConfig { enabled: true, ..ZonesConfig::default() };
        let zones = Zones::start(
            &config,
            Box::new(desk),
            scratch(name),
            move || {
                *wake.lock().unwrap() += 1;
            },
            true,
        );
        (zones, woken)
    }

    pub fn zone(zones: &mut Zones, title: &str, kind: ZoneKind, rect: (i32, i32, i32, i32)) -> ZoneId {
        zones.command(Command::CreateZone {
            title: title.to_string(),
            kind,
            rect: Some(gevrek_zones::Rect::new(rect.0, rect.1, rect.2, rect.3)),
        });
        zones.core().config().zones.last().unwrap().id.clone()
    }

    /// A desktop as it might be on a working day: papers in a grid, a
    /// list of downloads, a zone rolled up, two zones as tabs, pictures
    /// found by a smart zone, and a few things left lying loose.
    pub fn a_working_desktop(name: &str) -> Zones {
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![Display { primary: true, ..Display::new("picture-1", gevrek_zones::Rect::new(0, 0, 1280, 800)) }]);
        backend.add_folder("/downloads");
        for file in ["gevrek-0.6.0.tar.gz", "Invoice 2026-11.pdf", "setup.exe", "a very long name of a file that goes on.iso", "notes.md"] {
            backend.add("/downloads", Item::file(&format!("/downloads/{file}")));
        }
        let names = [
            "Annual report 2026.pdf", "Budget.xlsx", "Letter to the council.docx", "Minutes.txt", "Contract (signed).pdf",
            "Offer.pdf", "Notes on zones.md", "Slides.pptx", "Holiday.png", "Harbour at night.jpg", "Garden.png",
            "Projects", "Editor.lnk", "main.rs", "todo.txt", "Scan_0042.png",
        ];
        for name in names {
            let mut item = Item::file(&format!("/desk/{name}"));
            if name == "Projects" {
                item = Item::folder("/desk/Projects");
            }
            if name == "Editor.lnk" {
                item.kind = ItemKind::Shortcut;
            }
            backend.add("/desk", item);
        }
        backend.add(
            "/desk",
            Item { id: ItemId::new("::computer"), kind: ItemKind::Virtual, name: "This PC".to_string(), ..Item::default() },
        );
        let (mut zones, _) = zones_on(backend, name);
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (124, 40, 430, 250));
        let downloads = zone(&mut zones, "Downloads", ZoneKind::Portal { directory: "/downloads".to_string() }, (124, 310, 430, 190));
        let pictures = zone(
            &mut zones,
            "Pictures",
            ZoneKind::Smart { query: vec![gevrek_zones::config::Matcher::Extension { any_of: vec!["png".to_string(), "jpg".to_string()] }] },
            (574, 40, 330, 210),
        );
        let later = zone(&mut zones, "For later", ZoneKind::Manual, (574, 270, 330, 150));
        let code = zone(&mut zones, "Code", ZoneKind::Manual, (574, 318, 330, 150));
        let work = zone(&mut zones, "Work", ZoneKind::Manual, (924, 40, 320, 230));
        let home = zone(&mut zones, "Home", ZoneKind::Manual, (924, 300, 300, 200));
        let put = |zones: &mut Zones, zone: &ZoneId, name: &str| {
            zones.command(Command::AddItem { zone: zone.clone(), item: ItemId::new(format!("/desk/{name}")), index: None, cell: None });
        };
        for name in &names[..8] {
            put(&mut zones, &papers, name);
        }
        put(&mut zones, &later, "todo.txt");
        put(&mut zones, &code, "main.rs");
        put(&mut zones, &work, "Projects");
        put(&mut zones, &work, "Editor.lnk");
        put(&mut zones, &home, "Garden.png");
        zones.command(Command::SetView { zone: downloads, view: Some(View::List), sort: Some(Sort::Name), descending: None, icon_size: None });
        zones.command(Command::SetView { zone: pictures, view: None, sort: None, descending: None, icon_size: Some(64) });
        zones.command(Command::RollUp { zone: later, rolled_up: Some(true) });
        zones.command(Command::Tab { zone: home, with: work.clone() });
        zones.command(Command::ActivateTab { zone: work });
        zones.command(Command::Lock { zone: code, geometry: Some(true), members: None });
        zones.tick();
        zones
    }

    /// A wallpaper to draw on, so that what shows through is seen.
    fn wallpaper(ui: &egui::Ui, size: egui::Vec2) {
        let rows = 48;
        for row in 0..rows {
            let along = row as f32 / (rows - 1) as f32;
            let colour = gevrek_ui::theme::mix(Color32::from_rgb(24, 38, 66), Color32::from_rgb(168, 92, 74), along * along);
            let top = size.y * row as f32 / rows as f32;
            ui.painter().rect_filled(
                Rect::from_min_size(pos2(0.0, top), vec2(size.x, size.y / rows as f32 + 1.0)),
                0.0,
                colour,
            );
        }
        ui.painter().circle_filled(pos2(size.x * 0.72, size.y * 0.78), size.y * 0.16, Color32::from_rgb(236, 170, 110).gamma_multiply(0.55));
    }

    /// Draw the working desktop at a scale into pixels (and the size in pixels).
    pub fn pictured(zones: &mut Zones, scale: f32, preset: Option<gevrek_ui::Preset>) -> (Vec<u8>, (u32, u32), gevrek_ui::picture::Painter) {
        let size = ((1280.0 * scale) as u32, (800.0 * scale) as u32);
        let mut painter = gevrek_ui::picture::Painter::sized(size).expect("a graphics device that computes on the processor");
        let context = egui::Context::default();
        context.set_pixels_per_point(scale);
        let mut settings = gevrek_ui::Settings::default();
        if let Some(preset) = preset {
            settings.preset = preset;
        }
        gevrek_ui::theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
        gevrek_ui::theme::apply(&context, &palette, &settings);
        let display = DisplayId::new("picture-1");
        let mut pixels = Vec::new();
        // (Pictures are asked for in the first frame and there in the next.)
        for step in 0..3 {
            zones.tick();
            pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.2, Vec::new(), |ui| {
                wallpaper(ui, vec2(1280.0, 800.0));
                paint(zones, ui, &display, &Look { palette: &palette, settings: &settings });
            });
        }
        (pixels, size, painter)
    }

    /// Where the pictures go: under the target folder, beside what is built.
    fn pictures_folder() -> std::path::PathBuf {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        target.join("zones-pictures")
    }

    /// How many pixels differ much from the one a step before them: a
    /// picture with things drawn on it has many.
    fn busy(pixels: &[u8]) -> usize {
        // (Four numbers to a pixel; three pixels on.)
        (0..pixels.len().saturating_sub(12))
            .step_by(4)
            .filter(|at| (0..3).any(|part| pixels[at + part].abs_diff(pixels[at + 12 + part]) > 40))
            .count()
    }

    /// The working desktop laid out and painted at a scale without a
    /// graphics device: how many shapes came of it.
    fn painted(zones: &mut Zones, scale: f32) -> usize {
        let context = egui::Context::default();
        context.set_pixels_per_point(scale);
        let settings = gevrek_ui::Settings::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
        gevrek_ui::theme::apply(&context, &palette, &settings);
        let display = DisplayId::new("picture-1");
        let mut shapes = 0;
        for step in 0..3 {
            zones.tick();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                time: Some(10.0 + f64::from(step) * 0.2),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                paint(zones, ui, &display, &Look { palette: &palette, settings: &settings });
            });
            // (Nothing is drawn with: the textures go nowhere.)
            output.textures_delta.clear();
            shapes = output.shapes.len();
        }
        shapes
    }

    #[test]
    fn everything_of_the_desktop_is_painted_and_pictures_are_asked_for_at_the_surfaces_scale() {
        let mut zones = a_working_desktop("painted");
        let at_one = painted(&mut zones, 1.0);
        // Six zones in front, some thirty items, each with ground, picture and name.
        assert!(at_one > 150, "{at_one} shapes");
        let asked = |zones: &Zones| -> Vec<u32> { zones.pictures.asked.keys().map(|(_, pixels)| *pixels).collect() };
        assert!(asked(&zones).contains(&48) && asked(&zones).contains(&64), "{:?}", asked(&zones));
        assert!(!asked(&zones).contains(&96));
        // At one and a half, 64 points are 96 pixels and 48 are 72.
        let at_more = painted(&mut zones, 1.5);
        assert!(asked(&zones).contains(&96) && asked(&zones).contains(&72), "{:?}", asked(&zones));
        assert!(at_more >= at_one);
        // A picture that came is drawn as a picture: a texture was made.
        assert!(zones.pictures.textures.values().any(Option::is_some));
        // Hidden with one key: only what is always visible is left to paint.
        zones.command(Command::QuickHide { hidden: Some(true) });
        assert!(painted(&mut zones, 1.0) < at_one / 10);
    }

    /// The zones drawn into pictures, to be looked at:
    ///
    ///     cargo test -p gevrek-shell the_zones_are_drawn_to_be_looked_at -- --ignored
    ///
    /// writes `zones-1x.png`, `zones-1.5x.png` and
    /// `zones-high-contrast.png` into `target/zones-pictures/`.
    #[test]
    #[ignore = "draws pictures to look at; needs a graphics device that computes on the processor"]
    fn the_zones_are_drawn_to_be_looked_at() {
        let folder = pictures_folder();
        std::fs::create_dir_all(&folder).unwrap();
        let mut zones = a_working_desktop("picture");
        // Something selected, the pointer over an item, and a search.
        zones.gestures.selected.insert(ItemId::new("/desk/Budget.xlsx"));
        zones.gestures.selected.insert(ItemId::new("/desk/Offer.pdf"));
        zones.hover = Some(pos2(620.0, 100.0));
        let mut sizes = Vec::new();
        for (scale, name) in [(1.0, "zones-1x.png"), (1.5, "zones-1.5x.png")] {
            let (pixels, size, painter) = pictured(&mut zones, scale, None);
            assert_eq!(pixels.len(), (size.0 * size.1 * 4) as usize);
            assert!(busy(&pixels) > 4000, "{name}: little is drawn ({} edges)", busy(&pixels));
            let file = folder.join(name);
            painter.save(&pixels, &file).expect("the picture is written");
            assert!(std::fs::metadata(&file).unwrap().len() > 20_000, "{name} is nearly empty");
            println!("drawn: {}", file.display());
            sizes.push(size);
        }
        assert_eq!(sizes, [(1280, 800), (1920, 1200)]);
        // With high contrast, and with what is looked for.
        zones.filter = "report".to_string();
        let (pixels, _, painter) = pictured(&mut zones, 1.0, Some(gevrek_ui::Preset::ALL.into_iter().find(|preset| preset.is_high_contrast()).unwrap()));
        painter.save(&pixels, &folder.join("zones-high-contrast.png")).unwrap();
        // The pictures were asked for at the surface's own pixels: 64
        // points at one and a half is 96.
        let asked: Vec<u32> = zones.pictures.asked.keys().map(|(_, pixels)| *pixels).collect();
        assert!(asked.contains(&96) && asked.contains(&64) && asked.contains(&72) && asked.contains(&48), "{asked:?}");
    }

    #[test]
    fn a_surface_drawn_at_another_scale_than_its_display_has_is_noticed() {
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![Display { scale: 1.5, ..Display::new("hidpi", gevrek_zones::Rect::new(0, 0, 1280, 800)) }]);
        let (mut zones, _) = zones_on(backend, "scale");
        let display = DisplayId::new("hidpi");
        assert!(zones.surface_scale(&display, 1.5));
        assert!(!zones.surface_scale(&display, 1.0));
        assert_eq!(zones.odd_scales, [display]);
    }

    #[test]
    fn nothing_is_drawn_of_zones_that_are_switched_off() {
        let (mut zones, _) = zones_on(FakeBackend::new(), "off");
        let off = ZonesConfig::default();
        zones.configure(&off);
        assert!(zones.spots().is_empty(), "no surface is asked for");
        assert!(zones.layout().zones.is_empty());
    }
}
