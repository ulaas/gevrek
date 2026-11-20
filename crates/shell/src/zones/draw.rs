//! The zones drawn on a display's surface, and the pointer read from it.
//!
//! A zone is a quiet sheet on the wallpaper: a ground that lets the
//! picture behind it through, a hairline around it, and a title set off
//! by a short mark in the accent colour. What a zone is besides (held
//! where it is, shut to new items, staying when the rest is put away,
//! covered) is said by small figures at the end of its title, always in
//! the same order. The accent colour means one thing: this is where what
//! you do goes (the chosen items, the zone with the keyboard, the place a
//! drop would land).
//!
//! Three things move by themselves, each for a fraction of a second: a
//! zone rolling open or shut, everything fading when it is hidden at
//! once, and the zones coming above the windows ([`super::motion`]).
//! Nothing else does, and nothing is drawn unless something changed, so a
//! desktop that is left alone costs nothing.

use super::gesture::{self, Doing, Key, Pointer};
use super::motion::{Part, FADE, PEEK, ROLL};
use super::view::{self, DeskBox, ItemBox, Layout, ZoneBox};
use super::{Desk, Zones};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind};
use gevrek_shellkit::look::BlurRegion;
use gevrek_ui::theme::Palette;
use gevrek_zones::backend::{IconHandle, IconRequest, IconSize};
use gevrek_zones::config::{TitleBar, View, ZoneLook};
use gevrek_zones::core::ZonesCore;
use gevrek_zones::focus::{Direction, Step};
use gevrek_zones::item::ItemKind;
use gevrek_zones::portal::Listing;
use gevrek_zones::{DisplayId, ItemId};
use std::collections::{BTreeSet, HashMap};

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

/// What a zone does to the colours of its icons: how much of their own
/// colour they keep, and a colour laid over them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tone {
    /// In hundredths: 0 is grey, 100 as they are.
    saturation: u8,
    /// Red, green, blue, and how much of it is laid over.
    tint: [u8; 4],
}

impl Default for Tone {
    fn default() -> Self {
        Self { saturation: 100, tint: [0; 4] }
    }
}

impl Tone {
    fn of(look: &ZoneLook) -> Self {
        let tint = look.icon_tint.as_deref().and_then(gevrek_ui::color::parse);
        Self {
            saturation: look.icon_saturation.unwrap_or(100).min(100),
            tint: tint.map_or([0; 4], |tint| tint.to_srgba_unmultiplied()),
        }
    }

    /// Without any colour of its own: how every icon is drawn while all
    /// of them are asked to be grey. Laid over what a zone says of its
    /// icons for as long as that is asked, and no longer: what the zone
    /// says is kept.
    const GREY: Tone = Tone { saturation: 0, tint: [0; 4] };

    /// This, or grey where every icon is.
    fn or_grey(self, grey: bool) -> Self {
        if grey {
            Self::GREY
        } else {
            self
        }
    }

    fn is_plain(self) -> bool {
        self == Self::default()
    }

    /// One colour as this makes it.
    pub fn apply(self, [red, green, blue]: [u8; 3]) -> [u8; 3] {
        let keep = f32::from(self.saturation) / 100.0;
        let grey = 0.299 * f32::from(red) + 0.587 * f32::from(green) + 0.114 * f32::from(blue);
        let over = f32::from(self.tint[3]) / 255.0;
        let one = |own: u8, tint: u8| {
            let dulled = grey + (f32::from(own) - grey) * keep;
            (dulled + (f32::from(tint) - dulled) * over).round().clamp(0.0, 255.0) as u8
        };
        [one(red, self.tint[0]), one(green, self.tint[1]), one(blue, self.tint[2])]
    }

    fn colour(self, colour: Color32) -> Color32 {
        if self.is_plain() {
            return colour;
        }
        let [red, green, blue, alpha] = colour.to_srgba_unmultiplied();
        let [red, green, blue] = self.apply([red, green, blue]);
        Color32::from_rgba_unmultiplied(red, green, blue, alpha)
    }
}

/// The pictures of the items: asked of the backend when an item is first
/// drawn at a size, kept for as long as the item is there.
///
/// Everything is found by the item's id as it stands in the layout, so a
/// frame that draws two thousand items looks two thousand pictures up
/// and copies no id to do it.
#[derive(Default)]
pub struct Pictures {
    /// By the pixels a picture is wide, and then by item.
    asked: HashMap<u32, HashMap<ItemId, Asked>>,
    /// What each surface draws with (a surface has its own textures): by
    /// display, by size and the colours of the zone that shows it, and
    /// then by item.
    textures: HashMap<DisplayId, HashMap<(u32, Tone), Made>>,
}

/// What was made to draw with, by item.
type Made = HashMap<ItemId, egui::TextureHandle>;

impl Pictures {
    /// The backend answered a request. Says whether it was one of ours
    /// and there is now something else to draw.
    pub fn answered(&mut self, request: IconRequest, icon: Option<IconHandle>) -> bool {
        let waiting = |asked: &Asked| matches!(asked, Asked::Thumbnail(of) | Asked::Icon(of) if *of == request);
        let found = self.asked.iter_mut().find_map(|(pixels, of_size)| {
            of_size.iter_mut().find(|(_, asked)| waiting(asked)).map(|(item, asked)| (*pixels, item.clone(), asked))
        });
        let Some((pixels, item, asked)) = found else {
            return false;
        };
        *asked = match (*asked, icon) {
            // No thumbnail: the icon is asked for when it is next drawn.
            (Asked::Thumbnail(_), None) => Asked::Icon(IconRequest(0)),
            (_, icon) => Asked::There(icon),
        };
        // (What was drawn of it at this size until now is of the past.)
        for of_display in self.textures.values_mut() {
            for ((size, _), of_look) in of_display.iter_mut() {
                if *size == pixels {
                    of_look.remove(&item);
                }
            }
        }
        true
    }

    /// The picture of an item that has come, at whichever size.
    pub fn handle_of(&self, item: &ItemId) -> Option<IconHandle> {
        self.asked.values().find_map(|of_size| match of_size.get(item) {
            Some(Asked::There(Some(handle))) => Some(*handle),
            _ => None,
        })
    }

    /// Forget the pictures of items that are no longer there, and of
    /// those whose file changed.
    pub fn forget_gone(&mut self, core: &ZonesCore) {
        for of_size in self.asked.values_mut() {
            of_size.retain(|item, _| core.item(item).is_some());
        }
        for of_look in self.textures.values_mut().flat_map(HashMap::values_mut) {
            of_look.retain(|item, _| core.item(item).is_some());
        }
    }

    /// The sizes pictures were asked for at, in pixels (each once for
    /// each item it was asked for).
    #[cfg(test)]
    pub fn sizes_asked(&self) -> Vec<u32> {
        self.asked.iter().flat_map(|(pixels, of_size)| of_size.iter().map(|_| *pixels)).collect()
    }

    /// How many pictures were made into something to draw with.
    #[cfg(test)]
    pub fn textures_made(&self) -> usize {
        self.textures.values().flat_map(HashMap::values).map(HashMap::len).sum()
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
        tone: Tone,
    ) -> Option<egui::TextureHandle> {
        let scale = context.pixels_per_point();
        let pixels = (points as f32 * scale).round().max(1.0) as u32;
        // What was made of it already: the usual case, and nothing is
        // asked of anybody.
        let made = self.textures.get(display).and_then(|of_display| of_display.get(&(pixels, tone))).and_then(|of_look| of_look.get(item));
        if let Some(texture) = made {
            return Some(texture.clone());
        }
        let size = IconSize { size: points, scale: f64::from(scale) };
        let of_size = self.asked.entry(pixels).or_default();
        let asked = match of_size.get_mut(item) {
            Some(asked) => asked,
            None => of_size.entry(item.clone()).or_insert_with(|| Asked::Thumbnail(desk.request_thumbnail(item, size))),
        };
        if *asked == Asked::Icon(IconRequest(0)) {
            *asked = Asked::Icon(desk.request_icon(item, size));
        }
        let Asked::There(Some(handle)) = *asked else {
            return None;
        };
        // The backend keeps only so many pictures: one it has let go of
        // is asked for again (it is not that there is none).
        let Some(mut picture) = desk.picture(handle) else {
            of_size.remove(item);
            return None;
        };
        if !tone.is_plain() {
            for pixel in picture.pixels.as_chunks_mut::<4>().0 {
                let [red, green, blue] = tone.apply([pixel[0], pixel[1], pixel[2]]);
                (pixel[0], pixel[1], pixel[2]) = (red, green, blue);
            }
        }
        let image =
            egui::ColorImage::from_rgba_unmultiplied([picture.width as usize, picture.height as usize], &picture.pixels);
        let texture = context.load_texture(format!("zone-item-{}", handle.0), image, egui::TextureOptions::LINEAR);
        if !self.textures.contains_key(display) {
            self.textures.insert(display.clone(), HashMap::new());
        }
        let of_look = self.textures.get_mut(display)?.entry((pixels, tone)).or_default();
        of_look.insert(item.clone(), texture.clone());
        Some(texture)
    }
}

/// The look of the zones on one frame.
pub struct Look<'a> {
    pub palette: &'a Palette,
    pub settings: &'a gevrek_ui::Settings,
    /// The surfaces' host shows the desktop soft behind the grounds that
    /// ask for it. Where it cannot, such a ground is drawn more solid.
    pub blur: bool,
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
///
/// A double press is counted here, and not taken from egui: egui says
/// "twice" when the button comes up the second time, by the time of the
/// frames, and knows where only while the pointer is on the surface. A
/// frame drawn late, a release that went elsewhere or the pointer told
/// gone in the same frame each lose it. Counted at the second press, by
/// each press's own place and the system's own time of it, none of them
/// does.
fn hear(zones: &mut Zones, ui: &egui::Ui, origin: Pos2, desk: Rect) {
    // (A frame the toolkit lays out twice is heard once.)
    if ui.ctx().current_pass_index() > 0 {
        return;
    }
    // (Nothing is put into words for a trace that nobody asked for.)
    let tracing = zones.trace;
    let (mut pointer, heard, wheel, keys, typed) = ui.input(|input| {
        let at = input.pointer.latest_pos().map(|at| at + origin.to_vec2());
        let pointer = Pointer {
            at,
            pressed: input.pointer.primary_pressed(),
            released: input.pointer.primary_released(),
            down: input.pointer.primary_down(),
            double: false,
            middle_double: false,
            menu: input.pointer.secondary_pressed(),
            ctrl: input.modifiers.ctrl || input.modifiers.command,
            shift: input.modifiers.shift,
            alt: input.modifiers.alt,
        };
        // The presses of this frame as egui was told them (with the
        // frame's time: all there is where no host says the system's),
        // and what else a trace wants to know of.
        let mut heard = Heard::default();
        for event in &input.events {
            match event {
                egui::Event::PointerButton { pos, button, pressed, .. } => {
                    if *pressed {
                        heard.presses.push(gevrek_shellkit::Press { button: *button, at: *pos, time_ms: (input.time * 1000.0) as u64 as u32 });
                    } else if tracing {
                        heard.told.push(format!("release {button:?} at {:.0},{:.0}", pos.x + origin.x, pos.y + origin.y));
                    }
                }
                egui::Event::PointerGone if tracing => heard.told.push("the pointer is gone from the surface".to_string()),
                egui::Event::WindowFocused(has) if tracing => {
                    heard.told.push(format!("the surface {} the keyboard (it is the active window, or no longer)", if *has { "has" } else { "has lost" }))
                }
                _ => {}
            }
        }
        let mut keys = Vec::new();
        if input.focused {
            let (alt, shift, ctrl) = (input.modifiers.alt, input.modifiers.shift, input.modifiers.command);
            for (key, step, direction) in [
                (egui::Key::ArrowLeft, Step::Left, Direction::Left),
                (egui::Key::ArrowRight, Step::Right, Direction::Right),
                (egui::Key::ArrowUp, Step::Up, Direction::Up),
                (egui::Key::ArrowDown, Step::Down, Direction::Down),
            ] {
                if input.key_pressed(key) {
                    // Alone: among the items. With Alt: from zone to
                    // zone, and with Shift as well, taking what is
                    // chosen along.
                    keys.push(match (alt, shift) {
                        (true, true) => Key::Carry(direction),
                        (true, false) => Key::Zone(direction),
                        (false, extend) => Key::Step { step, extend },
                    });
                }
            }
            // (The window manager's own letters for the four ways.)
            for (key, direction) in [
                (egui::Key::H, Direction::Left),
                (egui::Key::J, Direction::Down),
                (egui::Key::K, Direction::Up),
                (egui::Key::L, Direction::Right),
            ] {
                if alt && input.key_pressed(key) {
                    keys.push(if shift { Key::Carry(direction) } else { Key::Zone(direction) });
                }
            }
            for (key, means) in [
                (egui::Key::Enter, Key::Enter),
                (egui::Key::Escape, Key::Escape),
                (egui::Key::Home, Key::Step { step: Step::First, extend: shift }),
                (egui::Key::End, Key::Step { step: Step::Last, extend: shift }),
                (egui::Key::Delete, Key::Delete),
                (egui::Key::F2, Key::Rename),
                (egui::Key::Backspace, Key::Erase),
            ] {
                if input.key_pressed(key) {
                    keys.push(means);
                }
            }
            if ctrl && input.key_pressed(egui::Key::A) {
                keys.push(Key::SelectAll);
            }
            if ctrl && input.key_pressed(egui::Key::R) {
                keys.push(Key::Roll);
            }
            if ctrl && input.key_pressed(egui::Key::Z) {
                keys.push(Key::Undo);
            }
            if ctrl && input.key_pressed(egui::Key::Tab) {
                keys.push(Key::NextTab { backwards: shift });
            }
            // Along the buttons of the title of the zone that has the
            // keyboard, and pressing the one the keyboard is on.
            if !ctrl && !alt && input.key_pressed(egui::Key::Tab) {
                keys.push(Key::Title { backwards: shift });
            }
            if !ctrl && !alt && input.key_pressed(egui::Key::Space) {
                keys.push(Key::Space);
            }
            // The key for menus (which the host tells as this), and what
            // is pressed in its place on a keyboard without one.
            if shift && input.key_pressed(egui::Key::F10) {
                keys.push(Key::Menu);
            }
        }
        // Letters: for what is looked for, a zone's filter, or going to
        // an item by the beginning of its name.
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
        (pointer, heard, input.smooth_scroll_delta.y, keys, typed)
    });
    for told in &heard.told {
        zones.traced(|| told.clone());
    }
    // What a double press is here: as the surface's host set it from the
    // system's own (where it did), else the toolkit's.
    let (within_ms, apart) =
        ui.ctx().options(|options| ((options.input_options.max_double_click_delay * 1000.0) as u32, options.input_options.max_click_dist));
    for press in zones.take_presses().unwrap_or(heard.presses) {
        // (On the desktop as a whole: a press is where it was made,
        // whatever the pointer did after it in the same frame.)
        let press = gevrek_shellkit::Press { at: press.at + origin.to_vec2(), ..press };
        let second = zones.pressed_twice(press, within_ms, apart);
        match press.button {
            egui::PointerButton::Primary => {
                pointer.at = Some(press.at);
                pointer.double |= second;
            }
            egui::PointerButton::Middle => {
                pointer.at = pointer.at.or(Some(press.at));
                pointer.middle_double |= second;
            }
            _ => {}
        }
    }
    if pointer.at.is_some() || pointer.released {
        zones.pointer(&pointer);
        if let (Some(at), true) = (pointer.at, wheel != 0.0) {
            zones.scroll(at, wheel);
        }
    } else if zones.hover.is_some_and(|at| desk.contains(at)) {
        // The pointer left this display.
        zones.pointer_left();
    }
    for key in keys {
        zones.key(key);
    }
    if !typed.trim().is_empty() {
        zones.typed(&typed);
    }
}

/// What a frame heard besides what the pointer's state says.
#[derive(Default)]
struct Heard {
    /// The buttons that went down, in the surface's own points.
    presses: Vec<gevrek_shellkit::Press>,
    /// What a trace is told of: buttons let go, the pointer gone, the
    /// keyboard come or gone.
    told: Vec<String>,
}

/// Draw the zones and the loose items of one display, and hear the
/// pointer on it. The surface's root `Ui` covers the display. Gives the
/// parts of the surface behind which the desktop is to be shown soft.
pub fn draw(zones: &mut Zones, ui: &mut egui::Ui, display: &DisplayId, look: &Look) -> Vec<BlurRegion> {
    zones.set_density(look.settings.density);
    zones.motion.still = look.settings.reduce_motion;
    zones.surface_scale(display, ui.ctx().pixels_per_point());
    let Some(desk) = zones.layout().desks.iter().find(|desk| &desk.display == display).map(|desk| desk.rect) else {
        // (What its host heard of presses is of a display that is not
        // there: not to be taken for another's.)
        zones.take_presses();
        return Vec::new();
    };
    hear(zones, ui, desk.min, desk);
    // The pointer says what a press here would take hold of: the arrows
    // of a side that is sized, the hand over what is pressed like a
    // button. (Only on the display the pointer is on.)
    let here = ui.input(|input| input.pointer.latest_pos().is_some());
    if let Some(at) = zones.hover.filter(|at| here && desk.contains(*at)) {
        let shape = zones.pointer_shape(at);
        ui.ctx().set_cursor_icon(shape);
    }
    let soft = paint(zones, ui, display, look);
    // While something is carried, drawn or on its way, the next frame
    // counts; at rest nothing asks to be drawn again.
    if zones.gestures.busy() || zones.motion.busy(zones.seconds()) {
        ui.ctx().request_repaint();
    }
    // What the button under the pointer does is said after a moment.
    if let Some(wait) = zones.hint_in() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait + 0.01));
    }
    soft
}

/// Everything of one display, drawn. No input is read.
pub fn paint(zones: &mut Zones, ui: &mut egui::Ui, display: &DisplayId, look: &Look) -> Vec<BlurRegion> {
    zones.motion.still = look.settings.reduce_motion;
    zones.layout();
    // (Taken out while it is drawn from, and put back: nothing is copied.)
    let layout = std::mem::take(&mut zones.layout);
    let before = zones.fading.take();
    let (soft, still_fading) = paint_from(zones, ui, display, look, &layout, before.as_ref());
    // What was painted says nothing to a screen reader: it is told.
    super::access::speak(zones, ui, &layout, display);
    zones.layout = layout;
    zones.fading = before.filter(|_| still_fading);
    soft
}

fn paint_from(
    zones: &mut Zones,
    ui: &mut egui::Ui,
    display: &DisplayId,
    look: &Look,
    layout: &Layout,
    before: Option<&Layout>,
) -> (Vec<BlurRegion>, bool) {
    let Some(desk) = layout.desks.iter().find(|desk| &desk.display == display) else {
        return (Vec::new(), false);
    };
    let now = zones.seconds();
    // From the desktop's units to this surface's points.
    let shift = -desk.rect.min.to_vec2();
    let screen = desk.rect.translate(shift);
    let painter = ui.painter().with_clip_rect(screen);
    let carried: Vec<(ItemId, Rect)> = match &zones.gestures.doing {
        Doing::Carrying { items, at } => gesture::carried(items, *at, layout),
        _ => Vec::new(),
    };
    let being_carried = |item: &ItemId| carried.iter().any(|(carried, _)| carried == item);
    // How much is seen of what the one key puts away, and how far the
    // zones are above the windows.
    let put_away = layout.hidden && !layout.peeking;
    let seen = zones.motion.toward(Part::Loose, if put_away { 0.0 } else { 1.0 }, FADE, now);
    let above = zones.motion.toward(Part::Peek, if layout.peeking { 1.0 } else { 0.0 }, PEEK, now);
    let coming = if put_away { 1.0 - seen } else { seen };
    let fading = before.is_some() && coming < 0.999;
    let before = before.filter(|_| fading);
    let grip = zones.grip_shown_in(layout);
    let mut scene = Scene { zones, ui, look, display, shift, now, soft: Vec::new(), grey: layout.grey, grip };
    if above > 0.0 {
        // Above the windows: they are dimmed, so that the zones are read.
        painter.rect_filled(screen, 0.0, Color32::from_black_alpha((if look.high_contrast() { 200.0 } else { 110.0 } * above) as u8));
    }
    let faded = |by: f32| {
        let mut faded = painter.clone();
        faded.multiply_opacity(by);
        faded
    };
    // What was there before and is no longer goes; what was not, comes.
    let old_desk = before.and_then(|before| before.desks.iter().find(|desk| &desk.display == display));
    let was_there: BTreeSet<&ItemId> = old_desk
        .map(|desk| desk.items.iter().chain(desk.stays.iter().map(|stay| &stay.shown)).map(|shown| &shown.item).collect())
        .unwrap_or_default();
    let is_there: BTreeSet<&ItemId> =
        desk.items.iter().chain(desk.stays.iter().map(|stay| &stay.shown)).map(|shown| &shown.item).collect();
    let bare = Style {
        view: View::Grid,
        icon_size: desk.icon_size,
        bare: true,
        tone: Tone::default().or_grey(layout.grey),
        labels: true,
        folder: false,
    };
    if desk.focused && !desk.items.iter().any(|item| item.focused) && (layout.peeking || ui.input(|input| input.focused)) {
        // The keyboard is among the loose items and on none of them yet:
        // the room they lie in says so, as a zone's rim does.
        let room = desk.area.translate(shift).shrink(3.0);
        painter.rect_stroke(room, 10.0, Stroke::new(1.5, look.palette.accent), StrokeKind::Inside);
    }
    if let Some(old) = old_desk {
        let going = faded(1.0 - coming);
        for item in old.items.iter().filter(|item| !is_there.contains(&item.item)) {
            scene.item(&going, item, &bare, false);
        }
    }
    let arriving = faded(coming);
    for item in &desk.items {
        let new = fading && !was_there.contains(&item.item);
        scene.item(if new { &arriving } else { &painter }, item, &bare, being_carried(&item.item));
    }
    for stay in &desk.stays {
        let style = Style { view: stay.view, icon_size: stay.icon_size, ..bare };
        let new = fading && !was_there.contains(&stay.shown.item);
        scene.item(if new { &arriving } else { &painter }, &stay.shown, &style, being_carried(&stay.shown.item));
    }
    if desk.masked {
        scene.covered_desk(&painter, desk);
    }
    if let Some(before) = before {
        let going = faded(1.0 - coming);
        for zone in before.zones.iter().filter(|zone| zone.rect.intersects(desk.rect)) {
            if layout.zone(&zone.zone).is_none() {
                scene.zone(&going, zone, 1.0, &being_carried);
            }
        }
    }
    for zone in layout.zones.iter().filter(|zone| zone.rect.intersects(desk.rect)) {
        let new = fading && before.is_some_and(|before| before.zone(&zone.zone).is_none());
        // How far it is open: a zone that is rolled up and shut is its
        // title, and on the way there what it was is drawn, cut short.
        let shut = zone.rolled_up && !zone.expanded;
        let open = scene.zones.motion.toward(Part::Open(zone.zone.clone()), if shut { 0.0 } else { 1.0 }, ROLL, now);
        let was = (shut && open > 0.01)
            .then(|| scene.zones.ghosts.get(&zone.zone).filter(|was| was.rect.min == zone.rect.min).cloned())
            .flatten();
        let painter = if new { &arriving } else { &painter };
        match &was {
            Some(was) => scene.zone(painter, was, open, &being_carried),
            None => scene.zone(painter, zone, if shut { 1.0 } else { open }, &being_carried),
        }
    }
    scene.over(&painter, layout, desk, &carried);
    if above > 0.0 && layout.peeking {
        scene.looked_for(&faded(above), layout, desk);
    }
    (std::mem::take(&mut scene.soft), fading)
}

/// How the items of one place are drawn.
#[derive(Debug, Clone, Copy)]
struct Style {
    view: View,
    icon_size: u32,
    /// On the wallpaper, not in a zone.
    bare: bool,
    tone: Tone,
    /// With their names.
    labels: bool,
    /// Files of a folder: one that a cloud keeps elsewhere says so.
    folder: bool,
}

/// What one frame of one display is drawn with.
struct Scene<'a> {
    zones: &'a mut Zones,
    ui: &'a egui::Ui,
    look: &'a Look<'a>,
    display: &'a DisplayId,
    shift: egui::Vec2,
    /// The time of this frame, for what moves.
    now: f64,
    /// The grounds behind which the desktop is to be shown soft.
    soft: Vec<BlurRegion>,
    /// Every icon is drawn grey.
    grey: bool,
    /// The side or corner of a zone the pointer would take hold of.
    grip: Option<(gevrek_zones::ZoneId, gevrek_zones::snap::Edges)>,
}

/// A soft shade under something that lies above the rest (a zone that
/// is open for now, one that is being moved).
fn shade(painter: &egui::Painter, rect: Rect, radius: f32) {
    for (out, alpha) in [(2.0, 34), (6.0, 20), (11.0, 11), (17.0, 5)] {
        painter.rect_filled(rect.expand(out).translate(vec2(0.0, 3.0 + out * 0.3)), radius + out, Color32::from_black_alpha(alpha));
    }
}

/// The small figures at the end of a title.
mod figures {
    use super::*;

    /// A padlock: nothing goes in or out.
    pub fn lock(painter: &egui::Painter, at: Pos2, colour: Color32) {
        painter.circle_stroke(at + vec2(0.0, -2.5), 2.5, Stroke::new(1.2, colour));
        painter.rect_filled(Rect::from_center_size(at + vec2(0.0, 1.5), vec2(8.0, 6.0)), 1.5, colour);
    }

    /// A pin: held where it is.
    pub fn pin(painter: &egui::Painter, at: Pos2, colour: Color32) {
        painter.circle_filled(at + vec2(0.0, -2.0), 3.0, colour);
        painter.line_segment([at + vec2(0.0, 0.5), at + vec2(0.0, 5.0)], Stroke::new(1.3, colour));
    }

    /// A small sun: stays when the rest is put away.
    pub fn stays(painter: &egui::Painter, at: Pos2, colour: Color32) {
        painter.circle_filled(at, 2.2, colour);
        painter.circle_stroke(at, 4.6, Stroke::new(1.0, colour));
    }

    /// An eye with a line through it: covered from view.
    pub fn covered(painter: &egui::Painter, at: Pos2, colour: Color32, size: f32) {
        let stroke = Stroke::new((size * 0.11).max(1.1), colour);
        let wide = size * 0.5;
        let lid = |down: f32| {
            let points: Vec<Pos2> = (0..=8)
                .map(|step| {
                    let along = step as f32 / 8.0;
                    at + vec2(-wide + 2.0 * wide * along, down * size * 0.3 * (std::f32::consts::PI * along).sin())
                })
                .collect();
            egui::Shape::line(points, stroke)
        };
        painter.add(lid(-1.0));
        painter.add(lid(1.0));
        painter.circle_filled(at, size * 0.12, colour);
        painter.line_segment([at + vec2(-wide * 0.8, size * 0.36), at + vec2(wide * 0.8, -size * 0.36)], stroke);
    }

    /// A small cloud on a disc: the file is kept elsewhere, and is
    /// fetched when it is opened.
    pub fn cloud(painter: &egui::Painter, at: Pos2, ground: Color32, colour: Color32) {
        painter.circle_filled(at, 8.0, ground);
        painter.circle_filled(at + vec2(-2.5, 0.5), 2.6, colour);
        painter.circle_filled(at + vec2(1.0, -1.2), 3.2, colour);
        painter.circle_filled(at + vec2(3.4, 1.0), 2.2, colour);
        painter.rect_filled(Rect::from_min_max(at + vec2(-2.5, 0.5), at + vec2(3.4, 3.2)), 1.0, colour);
    }

    /// A window's outline standing off a bar: windows keep off its place.
    pub fn keeps_off(painter: &egui::Painter, at: Pos2, colour: Color32) {
        painter.rect_filled(Rect::from_min_size(at + vec2(-5.0, -5.0), vec2(10.0, 2.5)), 1.0, colour);
        painter.rect_stroke(Rect::from_min_size(at + vec2(-5.0, -0.5), vec2(10.0, 6.0)), 1.0, Stroke::new(1.0, colour), StrokeKind::Inside);
    }

    /// An angle: there is more under the title (down), or it is open (up).
    pub fn angle(painter: &egui::Painter, at: Pos2, colour: Color32, up: bool) {
        let way = if up { -1.0 } else { 1.0 };
        let stroke = Stroke::new(1.3, colour);
        painter.line_segment([at + vec2(-3.5, -1.5 * way), at + vec2(0.0, 2.0 * way)], stroke);
        painter.line_segment([at + vec2(0.0, 2.0 * way), at + vec2(3.5, -1.5 * way)], stroke);
    }

    /// A lens: something is looked for.
    pub fn lens(painter: &egui::Painter, at: Pos2, colour: Color32, size: f32) {
        let stroke = Stroke::new((size * 0.12).max(1.2), colour);
        let glass = at + vec2(-size * 0.1, -size * 0.1);
        painter.circle_stroke(glass, size * 0.3, stroke);
        painter.line_segment([glass + vec2(size * 0.22, size * 0.22), at + vec2(size * 0.42, size * 0.42)], stroke);
    }
}

impl Scene<'_> {
    /// One zone. `open`: how much of it is there below its title, from 0
    /// (rolled up) to 1 (whole).
    fn zone(&mut self, painter: &egui::Painter, zone: &ZoneBox, open: f32, being_carried: &dyn Fn(&ItemId) -> bool) {
        let palette = self.look.palette;
        let strong = self.look.high_contrast();
        // A zone that refused what was asked of it shakes its head.
        let refusal = self.zones.motion.refusal(&zone.zone, self.now);
        let own_shift = self.shift;
        self.shift += vec2(refusal.map_or(0.0, |(aside, _)| aside), 0.0);
        let whole = zone.rect.translate(self.shift);
        let head = zone.head.translate(self.shift);
        let rect = Rect::from_min_size(
            whole.min,
            vec2(whole.width(), head.height() + (whole.height() - head.height()) * open.clamp(0.0, 1.0)),
        );
        let shut = zone.rolled_up && !zone.expanded && open <= 0.01;
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
        // The desktop shown soft behind it carries words on less ground;
        // where that is asked for and cannot be done, on more. (With high
        // contrast nothing shows through: words are on one ground.)
        let wants_soft = zone.look.blur.unwrap_or(false) && !strong;
        let usual = match (wants_soft, self.look.blur) {
            (true, true) => 52,
            (true, false) => 84,
            (false, _) => 70,
        };
        let hovered = self.zones.hover.is_some_and(|at| zone.rect.contains(at));
        let moved = matches!(&self.zones.gestures.doing, Doing::Moving { zone: moved, .. } if moved == &zone.zone);
        // What lies over other things (open for now, or in the hand) is
        // read against them, not through them: nearly solid.
        let lifted = zone.expanded || moved;
        let opacity = match f32::from(zone.look.opacity.unwrap_or(usual).min(100)) / 100.0 {
            _ if strong => 1.0,
            own if lifted => own.max(0.96),
            own => own,
        };
        // About to become a tab of the zone under it, it is seen through,
        // so that the zone it goes to is seen.
        let faint;
        let painter = if moved && self.zones.gestures.tab_onto().is_some() {
            faint = {
                let mut faint = painter.clone();
                faint.multiply_opacity(0.3);
                faint
            };
            &faint
        } else {
            painter
        };
        let dropping = self.zones.incoming.is_some_and(|(at, taken)| taken && zone.rect.contains(at))
            || matches!(&self.zones.gestures.doing, Doing::Carrying { at, .. }
                if zone.rect.contains(*at) && !zone.locked_items && zone.fills != view::Fills::Query);
        let tab_here = self.zones.gestures.tab_onto() == Some(&zone.zone);
        let with_keys = zone.focused && (self.zones.core.peeking() || self.ui.input(|input| input.focused));
        if lifted && !strong {
            shade(painter, rect, radius);
        }
        if wants_soft && self.look.blur && painter.opacity() > 0.99 {
            self.soft.push(BlurRegion::new(rect, radius));
        }
        painter.rect_filled(rect, radius, ground.gamma_multiply(opacity));
        let rim = if dropping || tab_here {
            Stroke::new(2.0, palette.accent)
        } else if with_keys {
            Stroke::new(if strong { 2.5 } else { 1.5 }, palette.accent)
        } else if strong {
            Stroke::new(1.0, palette.border_strong())
        } else {
            Stroke::new(1.0, palette.border.gamma_multiply(if hovered { 0.95 } else { 0.6 }))
        };
        painter.rect_stroke(rect, radius, rim, StrokeKind::Inside);
        self.grip(painter, zone, rect, radius, hovered);

        self.title(painter, zone, head, hovered, with_keys, shut, refusal.map(|(_, mark)| mark));
        if shut {
            self.shift = own_shift;
            return;
        }
        // Below the title everything is seen through what is open of it.
        let below = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        if zone.title_bar == TitleBar::Full {
            let hairline = Stroke::new(1.0, palette.border.gamma_multiply(if strong { 1.0 } else { 0.45 }));
            below.hline((rect.left() + 10.0)..=(rect.right() - 10.0), head.bottom() - 0.5, hairline);
        }
        let tabbing = match &self.zones.gestures.doing {
            Doing::Tabbing { zone } => Some(zone.clone()),
            _ => None,
        };
        for tab in &zone.tabs {
            let place = tab.rect.translate(self.shift);
            let lifted = tabbing.as_ref() == Some(&tab.zone);
            let ink = if tab.front { palette.text } else { palette.muted };
            let font = FontId::proportional(11.5);
            if lifted {
                below.rect_filled(place.shrink2(vec2(3.0, 2.0)), 6.0, palette.control_active());
            } else if !tab.front && self.zones.hover.is_some_and(|at| tab.rect.contains(at)) {
                below.rect_filled(place.shrink2(vec2(4.0, 3.0)), 6.0, palette.text.gamma_multiply(0.07));
            }
            let name = self.one_line(&tab.title, place.width() - 20.0, &font);
            below.text(pos2(place.left() + 12.0, place.center().y), Align2::LEFT_CENTER, name, font, ink);
            if tab.front {
                let under = Rect::from_min_max(
                    pos2(place.left() + 10.0, place.bottom() - 2.0),
                    pos2(place.right() - 10.0, place.bottom()),
                );
                below.rect_filled(under, 1.0, palette.accent);
            }
        }
        if let Some(busy) = &zone.busy {
            // Something that takes long is done with its files: a thin
            // line under the title says how far it has come.
            let track = Rect::from_min_max(pos2(rect.left() + 10.0, head.bottom() - 1.5), pos2(rect.right() - 10.0, head.bottom() + 1.5));
            below.rect_filled(track, 1.5, palette.text.gamma_multiply(0.14));
            match busy.part {
                Some(part) => {
                    let done = Rect::from_min_max(track.min, pos2(track.left() + track.width() * part, track.bottom()));
                    below.rect_filled(done, 1.5, palette.accent);
                }
                // (How far is not known: the whole line, quietly.)
                None => {
                    below.rect_filled(track, 1.5, palette.accent.gamma_multiply(0.45));
                }
            }
        }
        let content = zone.content.translate(self.shift);
        if zone.masked {
            self.covered(&below, content, zone.item_count);
            self.shift = own_shift;
            return;
        }
        match &zone.listing {
            Some(Listing::Loading) => self.notice(&below, content, "Looking\u{2026}", None),
            Some(Listing::Unavailable { reason }) => {
                self.notice(&below, content, "Not there now \u{00b7} press to look again", Some(reason))
            }
            // Nothing in it, or nothing that fits what is typed: said, so
            // that an empty zone is not taken for one that is still looking.
            Some(Listing::Ready) if zone.items.is_empty() && content.height() >= 40.0 => {
                let words = if zone.filter.is_some() { "Nothing here is called so" } else { "Nothing here" };
                below.text(content.center(), Align2::CENTER_CENTER, words, FontId::proportional(12.0), palette.faint);
            }
            _ => {}
        }
        // The items, seen through the zone's content only.
        let clipped = below.with_clip_rect(content.expand2(vec2(2.0, 2.0)).intersect(below.clip_rect()));
        let style = Style {
            view: zone.view,
            icon_size: zone.icon_size,
            bare: false,
            tone: Tone::of(&zone.look).or_grey(self.grey),
            labels: zone.look.labels.unwrap_or(true),
            folder: zone.fills == view::Fills::Folder,
        };
        for item in zone.items.iter().filter(|item| zone.content.intersects(item.rect)) {
            self.item(&clipped, item, &style, being_carried(&item.item));
        }
        if zone.scroll_extent > 0.0 && content.height() > 8.0 {
            // How much is seen of all there is, and where.
            let all = content.height() + zone.scroll_extent;
            let long = (content.height() * content.height() / all).max(18.0);
            let top = content.top() + (content.height() - long) * (zone.scroll / zone.scroll_extent);
            let bar = Rect::from_min_size(pos2(rect.right() - 6.0, top), vec2(3.0, long));
            below.rect_filled(bar, 1.5, palette.text.gamma_multiply(if hovered { 0.45 } else { 0.22 }));
        }
        self.shift = own_shift;
    }

    /// A zone's title: a short mark, the name, and at the end what the
    /// zone is besides, always in the same order from the right: whether
    /// there is more under the title, how many it holds, shut to items,
    /// held where it is, staying when the rest is put away, private,
    /// keeping windows off its place.
    #[allow(clippy::too_many_arguments)]
    fn title(
        &mut self,
        painter: &egui::Painter,
        zone: &ZoneBox,
        head: Rect,
        hovered: bool,
        with_keys: bool,
        shut: bool,
        refused: Option<f32>,
    ) {
        let palette = self.look.palette;
        // Rolled up and shut, the title is all there is: it is shown
        // whatever the zone says of its title bar.
        let bar = match zone.title_bar {
            TitleBar::Hidden if shut => TitleBar::Plain,
            bar => bar,
        };
        if bar == TitleBar::Hidden {
            // Nothing but, under the pointer, where to take hold of it
            // and its buttons, in the strip the title would be.
            if hovered {
                let dots = match zone.buttons.first() {
                    Some(first) => pos2((head.left() + first.rect.left() + self.shift.x) / 2.0, head.center().y),
                    None => head.center(),
                };
                for dot in -1..=1 {
                    painter.circle_filled(dots + vec2(dot as f32 * 7.0, 0.0), 1.5, palette.muted);
                }
                self.buttons(painter, zone, palette.muted, with_keys);
            }
            return;
        }
        if zone.look.title_on_hover.unwrap_or(false) && !hovered && !shut && !zone.focused {
            return;
        }
        let plain = bar == TitleBar::Plain;
        let ink = zone.look.title_colour.as_deref().and_then(gevrek_ui::color::parse).unwrap_or(if plain {
            palette.muted
        } else {
            palette.text
        });
        // From the right.
        let mut right = head.right() - 14.0;
        let middle = head.center().y;
        let quiet = palette.muted;
        let marked = |strength: Option<f32>| match strength {
            Some(strength) => gevrek_ui::theme::mix(quiet, palette.warn, strength),
            None => quiet,
        };
        // Under the pointer (and with the keyboard, and always where
        // everything is drawn to be seen) the end of the title is what
        // can be done with the zone: its buttons, each of which shows how
        // the zone stands as well. Otherwise it is what the zone is.
        // (A zone that just refused shows its mark, not its buttons: the
        // mark is what says why.)
        let with_buttons = !zone.buttons.is_empty() && (hovered || with_keys || self.look.high_contrast()) && refused.is_none();
        if with_buttons {
            self.buttons(painter, zone, ink, with_keys);
            right = zone.buttons.first().map_or(right, |first| first.rect.left() + self.shift.x - 6.0);
        }
        let with_figures = !with_buttons;
        if with_figures && zone.rolled_up {
            figures::angle(painter, pos2(right, middle), quiet, zone.expanded);
            right -= 16.0;
        }
        let count = match (&zone.filter, zone.item_count) {
            (_, 0) => String::new(),
            (Some(_), all) if !zone.masked => format!("{} of {all}", zone.items.len()),
            (_, all) => all.to_string(),
        };
        if with_figures && !count.is_empty() {
            let galley = painter.layout_no_wrap(count, FontId::proportional(11.0), palette.faint);
            let wide = galley.size().x;
            painter.galley(pos2(right + 4.0 - wide, middle - galley.size().y / 2.0), galley, palette.faint);
            right -= wide + 8.0;
        }
        if with_figures && zone.locked_items {
            figures::lock(painter, pos2(right, middle), marked(refused));
            right -= 15.0;
        }
        if with_figures && zone.locked_place {
            figures::pin(painter, pos2(right, middle - 1.0), marked(refused));
            right -= 14.0;
        }
        if with_figures && zone.always_visible {
            figures::stays(painter, pos2(right - 1.0, middle), quiet);
            right -= 16.0;
        }
        if with_figures && zone.private {
            figures::covered(painter, pos2(right - 2.0, middle), quiet, 12.0);
            right -= 18.0;
        }
        if with_figures && zone.reserves {
            figures::keeps_off(painter, pos2(right - 2.0, middle), quiet);
            right -= 17.0;
        }
        let left = if plain {
            head.left() + 12.0
        } else {
            // The mark says what fills the zone: a bar for one filled by
            // hand, a small folder for one that shows a folder, a
            // diamond for one that fills itself.
            let at = pos2(head.left() + 12.0, middle);
            match zone.fills {
                view::Fills::ByHand => {
                    painter.rect_filled(Rect::from_center_size(at, vec2(3.0, 12.0)), 1.5, palette.accent);
                }
                view::Fills::Folder => {
                    painter.rect_filled(Rect::from_min_size(at + vec2(-4.5, -5.0), vec2(5.0, 3.0)), 1.0, palette.accent);
                    painter.rect_filled(Rect::from_min_size(at + vec2(-4.5, -3.0), vec2(10.0, 7.5)), 1.5, palette.accent);
                }
                view::Fills::Query => {
                    let side = 4.5;
                    painter.add(egui::Shape::convex_polygon(
                        vec![at + vec2(0.0, -side), at + vec2(side, 0.0), at + vec2(0.0, side), at + vec2(-side, 0.0)],
                        palette.accent,
                        Stroke::NONE,
                    ));
                }
            }
            head.left() + 22.0
        };
        let font = self.look.title_font();
        // What is typed to show only some of its items stands after the name.
        let mut room = right - 4.0 - left;
        if let Some(typed) = &zone.filter {
            let chip_font = FontId::proportional(11.0);
            let text = self.one_line(typed, (room * 0.5).max(24.0), &chip_font);
            let galley = painter.layout_no_wrap(text, chip_font, palette.text);
            let chip = Rect::from_min_size(pos2(right - 4.0 - galley.size().x - 26.0, middle - 9.0), vec2(galley.size().x + 26.0, 18.0));
            painter.rect_filled(chip, 9.0, palette.accent.gamma_multiply(0.18));
            painter.rect_stroke(chip, 9.0, Stroke::new(1.0, palette.accent.gamma_multiply(0.8)), StrokeKind::Inside);
            figures::lens(painter, pos2(chip.left() + 10.0, middle), palette.text, 10.0);
            painter.galley(pos2(chip.left() + 19.0, middle - galley.size().y / 2.0), galley, palette.text);
            room = chip.left() - 6.0 - left;
        }
        if !zone.nav.is_empty() {
            // A portal that has gone somewhere shows its way in place of
            // its name: back and up, and the folders from its own to
            // where it stands, each to be pressed.
            self.way(painter, zone, ink, right - 4.0);
            return;
        }
        let name = self.one_line(&zone.title, room, &font);
        painter.text(pos2(left, middle), Align2::LEFT_CENTER, name, font, ink);
    }

    /// The buttons at the end of a title, in the title's colour: each a
    /// figure of the design system's own that shows how the zone stands
    /// (rolled up or not, held or not, as a grid or a list, covered or
    /// not). The one under the pointer has a ground; the one the
    /// keyboard is on has the accent's ring, as everything with the
    /// keyboard has.
    fn buttons(&mut self, painter: &egui::Painter, zone: &ZoneBox, ink: Color32, with_keys: bool) {
        use gevrek_ui::theme::icons;
        let palette = self.look.palette;
        let pressed = match &self.zones.gestures.doing {
            Doing::Pressed { hit: view::Hit::Button(of, button), .. } if of == &zone.zone => Some(*button),
            _ => None,
        };
        for shown in &zone.buttons {
            let place = shown.rect.translate(self.shift);
            let under = self.zones.hover.is_some_and(|at| shown.rect.contains(at)) && !self.zones.gestures.busy();
            let held = pressed == Some(shown.button);
            let on = with_keys && self.zones.gestures.title_at == Some((zone.zone.clone(), shown.button));
            if held {
                painter.rect_filled(place, 5.0, palette.text.gamma_multiply(0.2));
            } else if under {
                painter.rect_filled(place, 5.0, palette.text.gamma_multiply(0.11));
            }
            if on {
                painter.rect_stroke(place, 5.0, Stroke::new(if self.look.high_contrast() { 2.5 } else { 1.5 }, palette.accent), StrokeKind::Inside);
            }
            let figure = match shown.button {
                view::Button::Roll if zone.rolled_up => icons::CARET_DOWN,
                view::Button::Roll => icons::CARET_UP,
                view::Button::Lock => match zone.own_lock {
                    (false, false) => icons::LOCK_SIMPLE_OPEN,
                    (true, false) => icons::PUSH_PIN,
                    _ => icons::LOCK_SIMPLE,
                },
                view::Button::View if zone.view == View::Grid => icons::SQUARES_FOUR,
                view::Button::View => icons::LIST_BULLETS,
                view::Button::Sort => icons::SORT_ASCENDING,
                view::Button::Cover if zone.masked || zone.private => icons::EYE_SLASH,
                view::Button::Cover => icons::EYE,
                view::Button::More => icons::DOTS_THREE,
            };
            let colour = if under || held || on { ink } else { ink.gamma_multiply(0.72) };
            painter.text(place.center(), Align2::CENTER_CENTER, figure, FontId::proportional(14.5), colour);
        }
    }

    /// Where a zone is taken hold of to size it: a small mark in its
    /// lower right corner while the pointer is on the zone, and the side
    /// or corner the pointer would take (or has taken) drawn in the
    /// accent colour. A zone held where it is shows neither.
    fn grip(&mut self, painter: &egui::Painter, zone: &ZoneBox, rect: Rect, radius: f32, hovered: bool) {
        if zone.locked_place {
            return;
        }
        let palette = self.look.palette;
        let taken = self.grip.as_ref().filter(|(of, _)| of == &zone.zone).map(|(_, edges)| *edges);
        let rolled = zone.rolled_up && !zone.expanded;
        if (hovered || taken.is_some()) && !rolled && rect.height() >= 40.0 {
            // Three short slanting lines, as on a window's corner.
            let corner = taken.is_some_and(|edges| edges.right && edges.bottom);
            let colour = if corner { palette.accent } else { palette.muted.gamma_multiply(if self.look.high_contrast() { 1.0 } else { 0.8 }) };
            let stroke = Stroke::new(1.3, colour);
            let at = rect.right_bottom() - vec2(4.0 + radius * 0.18, 4.0 + radius * 0.18);
            for step in 0..3 {
                let out = 3.5 + step as f32 * 3.5;
                painter.line_segment([at + vec2(-out, 0.0), at + vec2(0.0, -out)], stroke);
            }
        }
        let Some(edges) = taken else {
            return;
        };
        // The sides that would move, each along its straight part.
        let stroke = Stroke::new(2.0, palette.accent);
        let inset = radius.max(4.0);
        if edges.left {
            painter.line_segment([pos2(rect.left() + 1.0, rect.top() + inset), pos2(rect.left() + 1.0, rect.bottom() - inset)], stroke);
        }
        if edges.right {
            painter.line_segment([pos2(rect.right() - 1.0, rect.top() + inset), pos2(rect.right() - 1.0, rect.bottom() - inset)], stroke);
        }
        if edges.top {
            painter.line_segment([pos2(rect.left() + inset, rect.top() + 1.0), pos2(rect.right() - inset, rect.top() + 1.0)], stroke);
        }
        if edges.bottom {
            painter.line_segment([pos2(rect.left() + inset, rect.bottom() - 1.0), pos2(rect.right() - inset, rect.bottom() - 1.0)], stroke);
        }
    }

    /// The way of a portal in its title.
    fn way(&mut self, painter: &egui::Painter, zone: &ZoneBox, ink: Color32, right: f32) {
        let palette = self.look.palette;
        let font = self.look.title_font();
        let painter = painter.with_clip_rect(
            Rect::from_min_max(zone.head.min + self.shift, pos2(right, zone.head.bottom() + self.shift.y)).intersect(painter.clip_rect()),
        );
        let mut last_right = None;
        for part in &zone.nav {
            let place = part.rect.translate(self.shift);
            let under = self.zones.hover.is_some_and(|at| part.rect.contains(at)) && !part.here;
            if under {
                painter.rect_filled(place, 6.0, palette.text.gamma_multiply(0.1));
            }
            let quiet = if under { palette.text } else { palette.muted };
            let stroke = Stroke::new(1.4, quiet);
            let centre = place.center();
            match &part.nav {
                view::Nav::Back => {
                    painter.line_segment([centre + vec2(2.0, -4.0), centre + vec2(-2.5, 0.0)], stroke);
                    painter.line_segment([centre + vec2(-2.5, 0.0), centre + vec2(2.0, 4.0)], stroke);
                }
                view::Nav::Up => {
                    painter.line_segment([centre + vec2(-4.0, 1.0), centre + vec2(0.0, -3.5)], stroke);
                    painter.line_segment([centre + vec2(0.0, -3.5), centre + vec2(4.0, 1.0)], stroke);
                    painter.line_segment([centre + vec2(0.0, -3.0), centre + vec2(0.0, 4.5)], stroke);
                }
                view::Nav::To(_) => {
                    if let Some(before) = last_right {
                        // Between two folders of the way.
                        let between = pos2((before + place.left()) / 2.0, centre.y);
                        let thin = Stroke::new(1.1, palette.faint);
                        painter.line_segment([between + vec2(-1.5, -3.0), between + vec2(1.5, 0.0)], thin);
                        painter.line_segment([between + vec2(1.5, 0.0), between + vec2(-1.5, 3.0)], thin);
                    }
                    let name = self.one_line(&part.label, place.width() - 8.0, &font);
                    let colour = if part.here { ink } else { quiet };
                    painter.text(pos2(place.left() + 4.0, centre.y), Align2::LEFT_CENTER, name, font.clone(), colour);
                    last_right = Some(place.right());
                }
            }
        }
    }

    /// What is shown in place of items that are covered from view: that
    /// there are some, and how many, and nothing of what they are.
    fn covered(&mut self, painter: &egui::Painter, content: Rect, count: usize) {
        let palette = self.look.palette;
        if content.height() < 20.0 {
            return;
        }
        // A weave of slanting lines, as a cloth laid over it.
        let cloth = painter.with_clip_rect(content.intersect(painter.clip_rect()));
        let line = Stroke::new(1.0, palette.text.gamma_multiply(0.05));
        let mut along = content.left() - content.height();
        while along < content.right() {
            cloth.line_segment([pos2(along, content.bottom()), pos2(along + content.height(), content.top())], line);
            along += 9.0;
        }
        let centre = content.center();
        let roomy = content.height() >= 64.0;
        if roomy {
            figures::covered(&cloth, centre - vec2(0.0, 12.0), palette.muted, 22.0);
        }
        let words = match count {
            0 => "Covered".to_string(),
            1 => "1 item, covered".to_string(),
            count => format!("{count} items, covered"),
        };
        let at = if roomy { centre + vec2(0.0, 12.0) } else { centre };
        cloth.text(at, Align2::CENTER_CENTER, words, FontId::proportional(12.0), palette.muted);
        if content.height() >= 96.0 {
            // How to see it: a press on the cover asks for that.
            cloth.text(centre + vec2(0.0, 30.0), Align2::CENTER_CENTER, "Press to show", FontId::proportional(11.0), palette.faint);
        }
    }

    /// The loose items of a display, covered.
    fn covered_desk(&mut self, painter: &egui::Painter, desk: &DeskBox) {
        if desk.item_count == 0 {
            return;
        }
        let palette = self.look.palette;
        let area = desk.area.translate(self.shift);
        let chip = Rect::from_min_size(area.min + vec2(12.0, 12.0), vec2(190.0, 30.0));
        painter.rect_filled(chip, 15.0, palette.surface.gamma_multiply(if self.look.high_contrast() { 1.0 } else { 0.8 }));
        figures::covered(painter, pos2(chip.left() + 18.0, chip.center().y), palette.muted, 14.0);
        let words = if desk.item_count == 1 { "1 item, covered".to_string() } else { format!("{} items, covered", desk.item_count) };
        painter.text(pos2(chip.left() + 34.0, chip.center().y), Align2::LEFT_CENTER, words, FontId::proportional(12.0), palette.muted);
    }

    /// A few words in the middle of a zone that has nothing to show for
    /// the moment, and perhaps why.
    fn notice(&mut self, painter: &egui::Painter, content: Rect, words: &str, why: Option<&str>) {
        let palette = self.look.palette;
        if content.height() < 20.0 {
            return;
        }
        let centre = content.center();
        let two = why.is_some() && content.height() >= 48.0;
        let at = if two { centre - vec2(0.0, 9.0) } else { centre };
        painter.text(at, Align2::CENTER_CENTER, words, FontId::proportional(12.5), if why.is_some() { palette.warn } else { palette.muted });
        if let (Some(why), true) = (why, two) {
            let font = FontId::proportional(11.0);
            let line = self.one_line(why, content.width() - 24.0, &font);
            painter.text(centre + vec2(0.0, 10.0), Align2::CENTER_CENTER, line, font, palette.faint);
        }
    }

    /// One item: its ground when it is chosen or under the pointer, its
    /// picture, its name.
    fn item(&mut self, painter: &egui::Painter, shown: &ItemBox, style: &Style, carried: bool) {
        let palette = self.look.palette;
        let rect = shown.rect.translate(self.shift);
        if !painter.clip_rect().intersects(rect) {
            return;
        }
        let selected = shown.selected;
        let focused = shown.focused && (self.zones.core.peeking() || self.ui.input(|input| input.focused));
        let hovered = self.zones.hover.is_some_and(|at| shown.rect.contains(at)) && !self.zones.gestures.busy();
        // What is looked for dims what does not fit; what is carried is
        // faint where it was.
        let looked_for = match &self.zones.search {
            Some(search) if !search.typed.trim().is_empty() => search.typed.as_str(),
            _ => self.zones.filter.as_str(),
        };
        let found = looked_for.trim().is_empty() || gevrek_zones::search::score(&shown.name, looked_for).is_some();
        let alpha = if carried { 0.4 } else if found { 1.0 } else { 0.38 };
        let ground = rect.shrink(1.0);
        if selected {
            painter.rect_filled(ground, 8.0, palette.accent.gamma_multiply(0.22));
            painter.rect_stroke(ground, 8.0, Stroke::new(1.0, palette.accent.gamma_multiply(0.75)), StrokeKind::Inside);
        } else if hovered {
            painter.rect_filled(ground, 8.0, palette.text.gamma_multiply(if style.bare { 0.12 } else { 0.07 }));
        }
        if focused {
            painter.rect_stroke(ground.expand(1.0), 9.0, Stroke::new(if self.look.high_contrast() { 2.5 } else { 1.5 }, palette.accent), StrokeKind::Outside);
        }
        let ink = if selected { palette.text } else { palette.text.gamma_multiply(0.92) }.gamma_multiply(alpha);
        let font = self.look.label_font();
        match style.view {
            View::Grid => {
                let side = style.icon_size as f32;
                // (Without its name, the picture stands in the middle.)
                let top = if style.labels { rect.top() + 4.0 } else { rect.center().y - side / 2.0 };
                let icon = Rect::from_min_size(pos2(rect.center().x - side / 2.0, top), vec2(side, side));
                self.picture(painter, shown, icon, style.icon_size, alpha, style.tone);
                if style.folder && self.zones.in_the_cloud(&shown.item) {
                    figures::cloud(painter, icon.right_bottom() - vec2(7.0, 7.0), palette.surface, palette.muted);
                }
                if style.labels {
                    let lines = self.label(shown, rect.width() - 6.0, &font);
                    for (number, line) in lines.iter().enumerate() {
                        let at = pos2(rect.center().x, icon.bottom() + 4.0 + number as f32 * 14.0);
                        if style.bare && !selected {
                            // On a picture, words need a shade to stand on.
                            painter.text(at + vec2(0.0, 1.0), Align2::CENTER_TOP, line, font.clone(), Color32::from_black_alpha((170.0 * alpha) as u8));
                        }
                        painter.text(at, Align2::CENTER_TOP, line, font.clone(), ink);
                    }
                }
            }
            View::List => {
                let side = (rect.height() - 4.0).clamp(8.0, 32.0);
                let icon = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - side / 2.0), vec2(side, side));
                self.picture(painter, shown, icon, side as u32, alpha, style.tone);
                if style.folder && self.zones.in_the_cloud(&shown.item) {
                    figures::cloud(painter, pos2(rect.right() - 14.0, rect.center().y), palette.surface, palette.muted);
                }
                if style.labels {
                    let left = icon.right() + 8.0;
                    let name = self.one_line(&shown.name, rect.right() - 6.0 - left, &font);
                    painter.text(pos2(left, rect.center().y), Align2::LEFT_CENTER, name, font, ink);
                }
            }
        }
        if shown.stays && self.zones.put_away() {
            // What stays while the rest is put away says why it is there.
            figures::stays(painter, rect.right_top() + vec2(-9.0, 9.0), palette.accent);
        }
    }

    /// An item's name in two lines at most. Broken once for a width, and
    /// kept until the layout changes: not measured anew with every frame.
    fn label(&mut self, shown: &ItemBox, width: f32, font: &FontId) -> std::rc::Rc<[String]> {
        let wide = width.round() as u32;
        if let Some(lines) = self.zones.labels.get(&wide).and_then(|of_width| of_width.get(&shown.item)) {
            return std::rc::Rc::clone(lines);
        }
        let ui = self.ui;
        let mut measure = |letter: char| ui.fonts_mut(|fonts| fonts.glyph_width(font, letter));
        let mut lines = view::label_lines(&shown.name, width, 2, &mut measure);
        lines.truncate(2);
        let lines: std::rc::Rc<[String]> = lines.into();
        self.zones.labels.entry(wide).or_default().insert(shown.item.clone(), std::rc::Rc::clone(&lines));
        lines
    }

    /// As much of a text as fits on one line, with an ellipsis for the rest.
    fn one_line(&self, text: &str, width: f32, font: &FontId) -> String {
        let ui = self.ui;
        let mut measure = |letter: char| ui.fonts_mut(|fonts| fonts.glyph_width(font, letter));
        view::label_lines(text, width.max(8.0), 1, &mut measure).pop().unwrap_or_default()
    }

    /// An item's picture: the system's when it has come, and until then
    /// (or where there is none) a figure of our own for what it is.
    fn picture(&mut self, painter: &egui::Painter, shown: &ItemBox, place: Rect, points: u32, alpha: f32, tone: Tone) {
        self.picture_of(painter, &shown.item, &shown.name, shown.kind, place, points, alpha, tone);
    }

    #[allow(clippy::too_many_arguments)]
    fn picture_of(
        &mut self,
        painter: &egui::Painter,
        item: &ItemId,
        name: &str,
        kind: ItemKind,
        place: Rect,
        points: u32,
        alpha: f32,
        tone: Tone,
    ) {
        let context = self.ui.ctx().clone();
        let zones = &mut *self.zones;
        let texture = zones.pictures.of(&context, &mut *zones.desk, self.display, item, points.clamp(8, 256), tone);
        match texture {
            Some(texture) => {
                let whole = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(texture.id(), place, whole, Color32::WHITE.linear_multiply(alpha));
            }
            None => figure(painter, self.look.palette, name, kind, place, alpha, tone),
        }
    }

    /// What lies over everything: items in the hand, the rectangle drawn
    /// to choose with, the lines a moved zone meets, a drop from outside.
    fn over(&mut self, painter: &egui::Painter, layout: &Layout, desk: &DeskBox, carried: &[(ItemId, Rect)]) {
        let palette = self.look.palette;
        for (item, rect) in carried {
            if !rect.intersects(desk.rect) {
                continue;
            }
            // (Where it came from decides how it is drawn while in the hand.)
            let from = layout.zone_of(item);
            let Some(was) = layout.item(item) else {
                continue;
            };
            let style = Style {
                view: from.map_or(View::Grid, |zone| zone.view),
                icon_size: from.map_or(desk.icon_size, |zone| zone.icon_size),
                bare: from.is_none(),
                tone: from.map_or(Tone::default(), |zone| Tone::of(&zone.look)).or_grey(self.grey),
                labels: from.is_none_or(|zone| zone.look.labels.unwrap_or(true)),
                folder: false,
            };
            let shown = ItemBox { rect: *rect, focused: false, ..was.clone() };
            painter.rect_filled(rect.translate(self.shift).shrink(1.0), 8.0, palette.surface.gamma_multiply(0.55));
            self.item(painter, &shown, &style, false);
        }
        if let Some(band) = self.zones.gestures.band() {
            let band = band.translate(self.shift);
            painter.rect_filled(band, 3.0, palette.accent.gamma_multiply(0.14));
            painter.rect_stroke(band, 3.0, Stroke::new(1.0, palette.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        }
        // Let go here, the zone that is dragged becomes a tab of this one:
        // said over its title, above the zone in the hand.
        let onto = self.zones.gestures.tab_onto().and_then(|zone| layout.zone(zone));
        if let Some(onto) = onto.filter(|onto| onto.rect.intersects(desk.rect)) {
            let dragged = match &self.zones.gestures.doing {
                Doing::Moving { zone, .. } => layout.zone(zone).map(|zone| zone.title.clone()),
                _ => None,
            };
            let wash = onto.head.translate(self.shift).shrink2(vec2(5.0, 3.0));
            painter.rect_filled(wash, 7.0, palette.accent);
            let font = FontId::proportional(12.0);
            let words = match dragged {
                Some(title) => format!("Let go: \u{201c}{title}\u{201d} becomes a tab here"),
                None => "Let go to make it a tab here".to_string(),
            };
            let words = self.one_line(&words, wash.width() - 16.0, &font);
            painter.text(wash.center(), Align2::CENTER_CENTER, words, font, palette.on_accent());
        }
        for guide in &self.zones.guides {
            let line = [guide.from + self.shift, guide.to + self.shift];
            painter.extend(egui::Shape::dashed_line(&line, Stroke::new(1.0, palette.accent), 6.0, 4.0));
        }
        for mark in &self.zones.grid_marks {
            // A small cross where a corner rests on the grid.
            let at = *mark + self.shift;
            let stroke = Stroke::new(1.0, palette.accent);
            painter.line_segment([at - vec2(5.0, 0.0), at + vec2(5.0, 0.0)], stroke);
            painter.line_segment([at - vec2(0.0, 5.0), at + vec2(0.0, 5.0)], stroke);
        }
        // What takes long and belongs to no zone that is shown: said at
        // the foot of the display the loose items are on.
        let first = self.zones.core.displays().iter().find(|display| display.primary).or(self.zones.core.displays().first());
        if first.is_some_and(|first| first.id == desk.display) {
            let area = desk.area.translate(self.shift);
            for (number, busy) in layout.under_way.iter().enumerate() {
                let chip = Rect::from_min_size(pos2(area.left() + 12.0, area.bottom() - 42.0 - 36.0 * number as f32), vec2(280.0, 30.0));
                painter.rect_filled(chip, 8.0, palette.surface.gamma_multiply(if self.look.high_contrast() { 1.0 } else { 0.9 }));
                painter.rect_stroke(chip, 8.0, Stroke::new(1.0, palette.border), StrokeKind::Inside);
                let font = FontId::proportional(12.0);
                let words = match busy.part {
                    Some(part) => format!("{} \u{00b7} {:.0} %", busy.what, part * 100.0),
                    None => busy.what.clone(),
                };
                let words = self.one_line(&words, chip.width() - 20.0, &font);
                painter.text(pos2(chip.left() + 10.0, chip.center().y - 2.0), Align2::LEFT_CENTER, words, font, palette.text);
                let track = Rect::from_min_max(pos2(chip.left() + 10.0, chip.bottom() - 6.0), pos2(chip.right() - 10.0, chip.bottom() - 4.0));
                painter.rect_filled(track, 1.0, palette.text.gamma_multiply(0.14));
                let done = track.width() * busy.part.unwrap_or(1.0);
                let colour = if busy.part.is_some() { palette.accent } else { palette.accent.gamma_multiply(0.45) };
                painter.rect_filled(Rect::from_min_size(track.min, vec2(done, track.height())), 1.0, colour);
            }
        }
        if let Some((at, taken)) = self.zones.incoming {
            // On the bare desktop a drop that is taken shows where.
            if taken && desk.rect.contains(at) && !layout.zones.iter().any(|zone| zone.rect.contains(at)) {
                let mark = Rect::from_center_size(at + self.shift, vec2(28.0, 28.0));
                painter.rect_stroke(mark, 8.0, Stroke::new(1.5, palette.accent), StrokeKind::Inside);
            }
        }
    }

    /// While the zones are looked at above the windows: the line that is
    /// typed into, and under it what was found everywhere, the best
    /// first. On the display the loose items would be on by default.
    fn looked_for(&mut self, painter: &egui::Painter, layout: &Layout, desk: &DeskBox) {
        let palette = self.look.palette;
        let strong = self.look.high_contrast();
        let first = self.zones.core.displays().iter().find(|display| display.primary).or(self.zones.core.displays().first());
        if first.is_some_and(|first| first.id != desk.display) {
            return;
        }
        let search = self.zones.search.clone().unwrap_or_default();
        let found: Vec<_> = search.hits.iter().take(super::MOST_FOUND).cloned().collect();
        let screen = desk.rect.translate(self.shift);
        let wide = 520.0_f32.min(screen.width() - 48.0);
        let row = 34.0;
        let typed = !search.typed.is_empty();
        let high = 46.0 + if typed { 10.0 + row * found.len().max(1) as f32 } else { 0.0 };
        let sheet = Rect::from_min_size(pos2(screen.center().x - wide / 2.0, screen.top() + (screen.height() * 0.16).max(24.0)), vec2(wide, high));
        let radius = f32::from(self.look.settings.radius) + 7.0;
        if !strong {
            shade(painter, sheet, radius);
        }
        painter.rect_filled(sheet, radius, palette.surface);
        painter.rect_stroke(sheet, radius, Stroke::new(if strong { 2.0 } else { 1.0 }, if strong { palette.border_strong() } else { palette.border }), StrokeKind::Inside);
        let line = Rect::from_min_size(sheet.min, vec2(wide, 46.0));
        figures::lens(painter, pos2(line.left() + 24.0, line.center().y), palette.muted, 15.0);
        let font = FontId::proportional(15.0);
        if typed {
            let text = self.one_line(&search.typed, wide - 110.0, &font);
            let galley = painter.layout_no_wrap(text, font, palette.text);
            let end = line.left() + 44.0 + galley.size().x;
            painter.galley(pos2(line.left() + 44.0, line.center().y - galley.size().y / 2.0), galley, palette.text);
            // Where the next letter goes.
            painter.vline(end + 2.0, (line.center().y - 9.0)..=(line.center().y + 9.0), Stroke::new(1.5, palette.accent));
        } else {
            painter.text(pos2(line.left() + 44.0, line.center().y), Align2::LEFT_CENTER, "Type to find anything in the zones", font, palette.faint);
        }
        painter.text(pos2(line.right() - 16.0, line.center().y), Align2::RIGHT_CENTER, "Esc", FontId::proportional(11.0), palette.faint);
        if !typed {
            return;
        }
        painter.hline((sheet.left() + 12.0)..=(sheet.right() - 12.0), line.bottom(), Stroke::new(1.0, palette.border.gamma_multiply(0.6)));
        if found.is_empty() {
            let at = pos2(sheet.center().x, line.bottom() + 5.0 + row / 2.0);
            painter.text(at, Align2::CENTER_CENTER, "Nothing is called so", FontId::proportional(12.5), palette.muted);
            return;
        }
        for (number, hit) in found.iter().enumerate() {
            let place = Rect::from_min_size(pos2(sheet.left() + 6.0, line.bottom() + 5.0 + row * number as f32), vec2(wide - 12.0, row));
            let chosen = number == search.at;
            if chosen {
                painter.rect_filled(place, 8.0, palette.accent.gamma_multiply(0.22));
                painter.rect_stroke(place, 8.0, Stroke::new(1.0, palette.accent.gamma_multiply(0.75)), StrokeKind::Inside);
            }
            let icon = Rect::from_center_size(pos2(place.left() + 22.0, place.center().y), vec2(22.0, 22.0));
            self.picture_of(painter, &hit.item, &hit.name, hit.kind, icon, 22, 1.0, Tone::default().or_grey(self.grey));
            // Where it is: the zone that shows it, or the bare desktop.
            let place_name = match &hit.zone {
                Some(zone) => layout
                    .shown_for(zone)
                    .and_then(|shown| {
                        shown.tabs.iter().find(|tab| &tab.zone == zone).map(|tab| tab.title.clone()).or(Some(shown.title.clone()))
                    })
                    .or_else(|| self.zones.core.config().zone(zone).map(|zone| zone.title.clone()))
                    .unwrap_or_default(),
                None => "Desktop".to_string(),
            };
            let small = FontId::proportional(11.5);
            let where_galley = painter.layout_no_wrap(place_name, small, palette.muted);
            let where_wide = where_galley.size().x;
            painter.galley(pos2(place.right() - 12.0 - where_wide, place.center().y - where_galley.size().y / 2.0), where_galley, palette.muted);
            let font = FontId::proportional(13.0);
            let name = self.one_line(&hit.name, place.width() - 44.0 - where_wide - 30.0, &font);
            painter.text(pos2(place.left() + 42.0, place.center().y), Align2::LEFT_CENTER, name, font, palette.text);
        }
    }
}


/// A figure for an item whose picture is not there: a folder, a sheet
/// with its extension written on it, a sheet with an arrow for a
/// shortcut, a rounded square for what is no file. Coloured by what it
/// is, so that a zone of papers and pictures can be told apart at a look.
fn figure(painter: &egui::Painter, palette: &Palette, name: &str, kind: ItemKind, place: Rect, alpha: f32, tone: Tone) {
    let tones = [palette.blue, palette.purple, palette.ok, palette.warn, palette.accent];
    // (What a file is called after its last dot says what it is.)
    let extension = match (kind, name.rsplit_once('.')) {
        (ItemKind::Folder | ItemKind::Virtual, _) | (_, None) => String::new(),
        (_, Some((_, extension))) => extension.to_lowercase(),
    };
    let hue = match kind {
        ItemKind::Folder => palette.accent,
        ItemKind::Virtual => palette.muted,
        _ => {
            let sum: usize = extension.bytes().map(usize::from).sum();
            tones[sum % tones.len()]
        }
    };
    // In the colours the zone gives its icons.
    let ink = |colour: Color32, by: f32| tone.colour(colour).gamma_multiply(by * alpha);
    let tone = hue;
    let side = place.width().min(place.height());
    let unit = side / 16.0;
    let centre = place.center();
    let round = (2.0 * unit).max(1.5);
    match kind {
        ItemKind::Folder => {
            let body = Rect::from_center_size(centre + vec2(0.0, unit), vec2(13.0 * unit, 9.5 * unit));
            let tab = Rect::from_min_size(body.left_top() - vec2(0.0, 2.0 * unit), vec2(5.5 * unit, 3.5 * unit));
            painter.rect_filled(tab, round * 0.7, ink(tone, 0.75));
            painter.rect_filled(body, round, ink(tone, 0.95));
            painter.hline((body.left() + round)..=(body.right() - round), body.top() + 1.5 * unit, Stroke::new(unit.max(1.0) * 0.6, ink(palette.surface, 0.35)));
        }
        ItemKind::Launcher => {
            // A program: a tile in a colour of its own, with the letter
            // its name begins with.
            let sum: usize = name.bytes().map(usize::from).sum();
            let own = tones[sum % tones.len()];
            let body = Rect::from_center_size(centre, vec2(12.5 * unit, 12.5 * unit));
            painter.rect_filled(body, 3.2 * unit, ink(gevrek_ui::theme::mix(palette.control, own, 0.55), 1.0));
            painter.rect_stroke(body, 3.2 * unit, Stroke::new(1.0, ink(own, 0.9)), StrokeKind::Inside);
            let letter: String = name.chars().find(|letter| letter.is_alphanumeric()).map(|letter| letter.to_uppercase().collect()).unwrap_or_default();
            if side >= 16.0 {
                painter.text(centre, Align2::CENTER_CENTER, letter, FontId::proportional((7.0 * unit).clamp(8.0, 40.0)), ink(palette.text, 1.0));
            }
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
            if kind == ItemKind::Shortcut || kind == ItemKind::Launcher {
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
    use gevrek_zones::item::Item;
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
        fn begin_listing(&mut self, location: &Location) -> bool {
            self.backend.begin_listing(location)
        }
        fn list_source(&mut self, source: &gevrek_zones::config::SmartSource) -> Result<Vec<Item>, BackendError> {
            self.backend.list_source(source)
        }
        fn watch_source(&mut self, source: &gevrek_zones::config::SmartSource) -> Result<WatchId, BackendError> {
            self.backend.watch_source(source)
        }
        fn begin_operation(&mut self, request: u64, operation: &FileOperation) -> bool {
            self.backend.begin_operation(request, operation)
        }
        fn cancel_operation(&mut self, request: u64) {
            self.backend.cancel_operation(request)
        }
        fn screen_shared(&mut self) -> Option<bool> {
            self.backend.screen_shared()
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

        fn has_system_icons(&self) -> bool {
            true
        }

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
        pictured_as(zones, scale, preset, false)
    }

    /// As `pictured`; `soft`: as where the host shows the desktop soft
    /// behind the grounds that ask for it.
    pub fn pictured_as(
        zones: &mut Zones,
        scale: f32,
        preset: Option<gevrek_ui::Preset>,
        soft: bool,
    ) -> (Vec<u8>, (u32, u32), gevrek_ui::picture::Painter) {
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
                paint(zones, ui, &display, &Look { palette: &palette, settings: &settings, blur: soft });
            });
            // (What moves has come to rest in the picture that is kept.)
            zones.later(0.6);
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
                paint(zones, ui, &display, &Look { palette: &palette, settings: &settings, blur: false });
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
        let asked = |zones: &Zones| -> Vec<u32> { zones.pictures.sizes_asked() };
        assert!(asked(&zones).contains(&48) && asked(&zones).contains(&64), "{:?}", asked(&zones));
        assert!(!asked(&zones).contains(&96));
        // At one and a half, 64 points are 96 pixels and 48 are 72.
        let at_more = painted(&mut zones, 1.5);
        assert!(asked(&zones).contains(&96) && asked(&zones).contains(&72), "{:?}", asked(&zones));
        assert!(at_more >= at_one);
        // A picture that came is drawn as a picture: a texture was made.
        assert!(zones.pictures.textures_made() > 0);
        // Hidden with one key: only what is always visible is left to paint.
        zones.command(Command::QuickHide { hidden: Some(true) });
        // What goes fades for a moment, from the first frame that shows it going...
        assert!(painted(&mut zones, 1.0) > at_one / 2);
        assert!(zones.motion.busy(zones.seconds()));
        // ...and then nothing of it is painted, and nothing moves.
        zones.later(1.0);
        assert!(painted(&mut zones, 1.0) < at_one / 10);
        assert!(!zones.motion.busy(zones.seconds()) && zones.fading.is_none());
        // With movement reduced it is gone at once.
        zones.motion.still = true;
        zones.command(Command::QuickHide { hidden: Some(false) });
        assert!(zones.fading.is_none());
    }

    /// Frames of the one display's surface, each with what its host
    /// heard of the pointer since the one before, a twentieth of a second
    /// apart: drawn and heard as on a real surface, without a graphics
    /// device.
    pub fn frames(zones: &mut Zones, context: &egui::Context, from: f64, frames: Vec<Vec<egui::Event>>) {
        let settings = gevrek_ui::Settings::default();
        let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
        let display = DisplayId::new("picture-1");
        for (step, events) in frames.into_iter().enumerate() {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                time: Some(from + step as f64 * 0.05),
                events,
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                draw(zones, ui, &display, &Look { palette: &palette, settings: &settings, blur: false });
            });
            output.textures_delta.clear();
        }
    }

    /// A button going down or coming up at a point, as a host tells it.
    pub fn button(at: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// The main button pressed twice in quick succession at a point: the
    /// pointer comes there, and each press and each letting go is a frame.
    pub fn pressed_twice(zones: &mut Zones, context: &egui::Context, from: f64, at: Pos2) {
        let main = egui::PointerButton::Primary;
        frames(
            zones,
            context,
            from,
            vec![
                vec![egui::Event::PointerMoved(at)],
                vec![button(at, main, true)],
                vec![button(at, main, false)],
                vec![button(at, main, true)],
                vec![button(at, main, false)],
            ],
        );
    }

    #[test]
    fn two_presses_on_the_desktop_bring_back_what_was_hidden_wherever_they_are() {
        let hidden = |zones: &Zones| zones.core().scene(0).quick_hidden;
        // On the bare desktop, and where a zone lies that is hidden (the
        // papers): hidden, it is not there to be pressed.
        for (name, at) in [("back-bare", pos2(1100.0, 700.0)), ("back-zone", pos2(300.0, 150.0))] {
            let mut zones = a_working_desktop(name);
            let context = egui::Context::default();
            gevrek_ui::theme::install_fonts(&context, false);
            zones.command(Command::QuickHide { hidden: Some(true) });
            assert!(hidden(&zones) && zones.layout().zones.is_empty());
            pressed_twice(&mut zones, &context, 10.0, at);
            assert!(!hidden(&zones), "{name}: two presses at {at:?} brought nothing back");
            assert!(!zones.layout().zones.is_empty());
        }
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
        zones.command(Command::Select {
            items: vec![ItemId::new("/desk/Budget.xlsx"), ItemId::new("/desk/Offer.pdf")],
            mode: gevrek_zones::focus::SelectMode::Replace,
        });
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
        let asked: Vec<u32> = zones.pictures.sizes_asked();
        assert!(asked.contains(&96) && asked.contains(&64) && asked.contains(&72) && asked.contains(&48), "{asked:?}");
    }

    /// One picture of the working desktop after something was done to it.
    fn shot(name: &str, scale: f32, high_contrast: bool, soft: bool, arrange: impl FnOnce(&mut Zones)) -> std::path::PathBuf {
        let folder = pictures_folder();
        std::fs::create_dir_all(&folder).unwrap();
        let mut zones = a_working_desktop(name);
        arrange(&mut zones);
        let preset = high_contrast.then(|| gevrek_ui::Preset::ALL.into_iter().find(|preset| preset.is_high_contrast()).unwrap());
        let (pixels, _, painter) = pictured_as(&mut zones, scale, preset, soft);
        let file = folder.join(format!("{name}.png"));
        painter.save(&pixels, &file).expect("the picture is written");
        println!("drawn: {}", file.display());
        file
    }

    fn zone_called(zones: &Zones, title: &str) -> ZoneId {
        zones.core().config().zones.iter().find(|zone| zone.title == title).unwrap().id.clone()
    }

    /// What milestone 3 added, drawn into pictures to be looked at:
    ///
    ///     cargo test -p gevrek-shell milestone_three_is_drawn -- --ignored
    ///
    /// writes `m3-*.png` into `target/zones-pictures/`.
    #[test]
    #[ignore = "draws pictures to look at; needs a graphics device that computes on the processor"]
    fn milestone_three_is_drawn_to_be_looked_at() {
        use gevrek_zones::core::{Effect, ZonesEvent};
        use gevrek_zones::focus::SelectMode;
        let desk_item = |name: &str| ItemId::new(format!("/desk/{name}"));
        // A rolled-up zone open for now, over the zone below it; the
        // pointer in it, and the keyboard on one of its items.
        shot("m3-rolled-up-open", 1.0, false, false, |zones| {
            let later = zone_called(zones, "For later");
            for name in ["Minutes.txt", "Offer.pdf", "Slides.pptx"] {
                zones.command(Command::AddItem { zone: later.clone(), item: desk_item(name), index: None, cell: None });
            }
            zones.command(Command::Expand { zone: later, expanded: Some(true) });
            zones.hover = Some(pos2(700.0, 330.0));
        });
        // A zone dragged onto the title of another: it would become its tab.
        shot("m3-tab-drop", 1.0, false, false, |zones| {
            let (code, pictures) = (zone_called(zones, "Code"), zone_called(zones, "Pictures"));
            zones.command(Command::Lock { zone: code.clone(), geometry: Some(false), members: None });
            zones.command(Command::MoveZone { zone: code.clone(), x: 560, y: 46, snap: false });
            zones.gestures.doing = Doing::Moving { zone: code, grab: vec2(60.0, 12.0), onto: Some(pictures) };
            zones.hover = Some(pos2(620.0, 58.0));
        });
        // A tab pulled off its row: a zone of its own, held by its title,
        // with the lines it meets.
        shot("m3-tab-split", 1.0, false, false, |zones| {
            let home = zone_called(zones, "Home");
            zones.command(Command::SplitTab { zone: home.clone(), x: 924, y: 290, snap: true });
            zones.gestures.doing = Doing::Moving { zone: home, grab: vec2(48.0, 13.0), onto: None };
            zones.watch_guides();
            zones.hover = Some(pos2(972.0, 303.0));
        });
        // Everything put away with the one key: a zone and two items stay.
        shot("m3-hidden", 1.0, false, false, |zones| {
            let pictures = zone_called(zones, "Pictures");
            zones.command(Command::AlwaysVisible { zone: pictures, on: Some(true) });
            zones.command(Command::ItemAlwaysVisible { item: desk_item("Budget.xlsx"), on: Some(true) });
            zones.command(Command::ItemAlwaysVisible { item: ItemId::new("::computer"), on: Some(true) });
            zones.command(Command::QuickHide { hidden: Some(true) });
        });
        // Looked at above the windows, with something typed.
        let peek = |zones: &mut Zones| {
            zones.command(Command::Peek { on: true });
            zones.typed("no");
            zones.key(Key::Step { step: Step::Down, extend: false });
        };
        shot("m3-peek-search", 1.0, false, false, peek);
        shot("m3-peek-search-high-contrast", 1.0, true, false, peek);
        // The looks a zone can be given, what it says of itself at the
        // end of its title, and the keyboard on a zone.
        let looks = |zones: &mut Zones| {
            use serde_json::json;
            let change = |zones: &mut Zones, title: &str, changes: serde_json::Value| {
                let zone = zone_called(zones, title);
                let refused = zones.command(Command::ModifyZone { zone, changes });
                assert!(!refused.iter().any(|effect| matches!(effect, Effect::Notify(ZonesEvent::Refused { .. }))), "{refused:?}");
            };
            change(zones, "Papers", json!({"look": {"blur": true}, "always_visible": true, "lock": {"members": true}}));
            change(zones, "Downloads", json!({"look": {"title_bar": "plain", "icon_saturation": 0}}));
            change(zones, "Pictures", json!({"look": {"title_bar": "hidden", "labels": false, "corner_radius": 18}}));
            change(zones, "Code", json!({"look": {"background": "#2b2440", "opacity": 90, "icon_tint": "#40e0d066", "icon_saturation": 0}}));
            change(zones, "Work", json!({"look": {"corner_radius": 0, "opacity": 45}}));
            let papers = zone_called(zones, "Papers");
            zones.command(Command::Filter { zone: Some(papers.clone()), typed: "re".to_string() });
            zones.command(Command::FocusItem { item: Some(ItemId::new("/desk/Annual report 2026.pdf")) });
            zones.command(Command::Select { items: vec![ItemId::new("/desk/Annual report 2026.pdf")], mode: SelectMode::Replace });
            zones.hover = Some(pos2(700.0, 60.0));
        };
        shot("m3-looks", 1.0, false, true, looks);
        shot("m3-looks-1.5x", 1.5, false, false, looks);
        shot("m3-looks-high-contrast", 1.0, true, false, looks);
    }

    /// A desktop of folders and of zones that fill themselves: a portal
    /// that has gone two folders in, one whose folder is away, one that
    /// is still being read, the files used last, the programs that run
    /// and the pinned ones.
    pub fn a_desk_of_folders(name: &str) -> Zones {
        use gevrek_zones::config::SmartSource;
        use serde_json::json;
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![Display { primary: true, ..Display::new("picture-1", gevrek_zones::Rect::new(0, 0, 1280, 800)) }]);
        backend.add_folder("/home/ada/Downloads");
        for file in ["gevrek-0.6.0.tar.gz", "setup.exe", "notes.md"] {
            backend.add_file_in("/home/ada/Downloads", file);
        }
        backend.add_folder_in("/home/ada/Downloads", "2026");
        backend.add_folder_in("/home/ada/Downloads/2026", "Invoices");
        backend.add_folder_in("/home/ada/Downloads/2026", "Receipts");
        backend.add_file_in("/home/ada/Downloads/2026", "Tax return.pdf");
        for file in ["Invoice 2026-09.pdf", "Invoice 2026-10.pdf", "Invoice 2026-11.pdf", "Reminder.pdf", "Summary.xlsx"] {
            backend.add_file_in("/home/ada/Downloads/2026/Invoices", file);
        }
        backend.add_folder("//nas/films");
        backend.set_reachable("//nas/films", false);
        backend.add_folder("/mnt/archive");
        backend.set_slow("/mnt/archive", true);
        for file in ["Letter.docx", "Plan.md"] {
            backend.add("/desk", Item::file(&format!("/desk/{file}")));
        }
        let program = |id: &str, name: &str| Item { id: ItemId::new(id), kind: ItemKind::Launcher, name: name.to_string(), ..Item::default() };
        backend.set_source(
            SmartSource::Recent,
            ["/home/ada/Thesis/chapter-3.md", "/home/ada/Pictures/Harbour.jpg", "/home/ada/Budget.xlsx", "/home/ada/todo.txt"]
                .map(Item::file)
                .to_vec(),
        );
        backend.set_source(
            SmartSource::RunningApps,
            vec![program("running:editor", "Editor"), program("running:terminal", "Terminal"), program("running:browser", "Browser")],
        );
        backend.set_source(
            SmartSource::Pinned,
            vec![program("app:files", "Files"), program("app:mail", "Mail"), program("app:music", "Music"), program("app:editor", "Editor")],
        );
        let (mut zones, _) = zones_on(backend, name);
        let downloads = zone(&mut zones, "Downloads", ZoneKind::Portal { directory: "/home/ada/Downloads".to_string() }, (124, 40, 470, 260));
        zone(&mut zones, "Films", ZoneKind::Portal { directory: "//nas/films".to_string() }, (124, 320, 225, 170));
        zone(&mut zones, "Archive", ZoneKind::Portal { directory: "/mnt/archive".to_string() }, (369, 320, 225, 170));
        let smart = ZoneKind::Smart { query: Vec::new() };
        zones.make_zone("Recent", smart.clone(), Some(gevrek_zones::Rect::new(624, 40, 300, 170)), Some(json!({"source": {"from": "recent"}, "view": "list"})));
        zones.make_zone("Running", smart.clone(), Some(gevrek_zones::Rect::new(624, 230, 300, 130)), Some(json!({"source": {"from": "running_apps"}})));
        zones.make_zone("Pinned", smart, Some(gevrek_zones::Rect::new(624, 380, 380, 130)), Some(json!({"source": {"from": "pinned"}})));
        zones.tick();
        // Into 2026, and into its invoices.
        for folder in ["/home/ada/Downloads/2026", "/home/ada/Downloads/2026/Invoices"] {
            zones.command(Command::EnterFolder { zone: downloads.clone(), item: ItemId::new(folder) });
            zones.tick();
        }
        zones
    }

    /// What milestone 4 added, drawn into pictures to be looked at:
    ///
    ///     cargo test -p gevrek-shell milestone_four_is_drawn -- --ignored
    ///
    /// writes `m4-*.png` into `target/zones-pictures/`.
    #[test]
    #[ignore = "draws pictures to look at; needs a graphics device that computes on the processor"]
    fn milestone_four_is_drawn_to_be_looked_at() {
        let folder = pictures_folder();
        std::fs::create_dir_all(&folder).unwrap();
        for (name, scale, strong, typed) in [
            ("m4-portals-and-smart-zones", 1.0, false, ""),
            ("m4-portal-filter", 1.0, false, "inv"),
            ("m4-portals-and-smart-zones-1.5x", 1.5, false, ""),
            ("m4-portals-and-smart-zones-high-contrast", 1.0, true, "inv"),
        ] {
            let mut zones = a_desk_of_folders(name);
            let downloads = zone_called(&zones, "Downloads");
            if !typed.is_empty() {
                zones.command(Command::FocusZone { zone: Some(downloads) });
                zones.typed(typed);
            }
            zones.hover = Some(pos2(204.0, 54.0));
            let preset = strong.then(|| gevrek_ui::Preset::ALL.into_iter().find(|preset| preset.is_high_contrast()).unwrap());
            let (pixels, _, painter) = pictured_as(&mut zones, scale, preset, false);
            let file = folder.join(format!("{name}.png"));
            painter.save(&pixels, &file).unwrap();
            println!("drawn: {}", file.display());
        }
        // The sheet of what the rules would do, as its surface shows it.
        for (name, strong) in [("m4-rules-preview", false), ("m4-rules-preview-high-contrast", true)] {
            let zones = a_desk_with_rules(name);
            let preview = zones.rules_preview(gevrek_zones::rules::Scope::All);
            assert!(preview.lines.len() >= 5, "{:?}", preview.lines);
            let size = (600, 460);
            let mut painter = gevrek_ui::picture::Painter::sized(size).unwrap();
            let context = egui::Context::default();
            let mut settings = gevrek_ui::Settings::default();
            if strong {
                settings.preset = gevrek_ui::Preset::ALL.into_iter().find(|preset| preset.is_high_contrast()).unwrap();
            }
            gevrek_ui::theme::install_fonts(&context, false);
            let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
            gevrek_ui::theme::apply(&context, &palette, &settings);
            let mut pixels = Vec::new();
            for step in 0..2 {
                pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.2, Vec::new(), |ui| {
                    crate::zones::surfaces::rules_sheet(ui, &palette, &preview, None);
                });
            }
            let file = folder.join(format!("{name}.png"));
            painter.save(&pixels, &file).unwrap();
            println!("drawn: {}", file.display());
        }
    }

    /// A desktop with rules of every sort: into zones, a file moved, an
    /// item held back by a lock, one whose rule names no zone, and a rule
    /// that asks how old a file is.
    pub fn a_desk_with_rules(name: &str) -> Zones {
        use gevrek_zones::config::{Matcher, Rule, RuleAction};
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![Display { primary: true, ..Display::new("picture-1", gevrek_zones::Rect::new(0, 0, 1280, 800)) }]);
        backend.add_folder("/home/ada/Archive");
        for file in ["Holiday.png", "Harbour at night.jpg", "Annual report 2026.pdf", "Budget.xlsx", "setup.exe", "old-notes.txt", "main.rs", "song.mp3"] {
            backend.add("/desk", Item::file(&format!("/desk/{file}")));
        }
        let (mut zones, _) = zones_on(backend, name);
        let pictures = zone(&mut zones, "Pictures", ZoneKind::Manual, (60, 40, 300, 200));
        let papers = zone(&mut zones, "Papers", ZoneKind::Manual, (380, 40, 300, 200));
        let code = zone(&mut zones, "Code", ZoneKind::Manual, (700, 40, 300, 200));
        zones.command(Command::Lock { zone: code.clone(), geometry: None, members: Some(true) });
        let extension = |any_of: &[&str]| Matcher::Extension { any_of: any_of.iter().map(|text| text.to_string()).collect() };
        let rule = |name: &str, matcher: Matcher, action: RuleAction| Rule { name: name.to_string(), matchers: vec![matcher], action, ..Rule::default() };
        let into = |zone: &ZoneId| RuleAction::AssignToZone { zone: zone.clone() };
        let mut config = zones.core().config().clone();
        config.rules = vec![
            rule("Pictures", extension(&["png", "jpg"]), into(&pictures)),
            rule("Papers", extension(&["pdf", "xlsx"]), into(&papers)),
            rule("Sources", extension(&["rs"]), into(&code)),
            rule("Installers", extension(&["exe"]), RuleAction::MoveToFolder { folder: "/home/ada/Archive".to_string() }),
            rule("Music", extension(&["mp3"]), into(&ZoneId::new("music"))),
            rule("Stale", Matcher::Untouched { days: 30 }, into(&papers)),
        ];
        zones.configure(&config);
        zones
    }

    /// Draw one frame of a display without a graphics device, as a host
    /// would ask for it; what came of it, and how long it took.
    fn one_frame(
        zones: &mut Zones,
        context: &egui::Context,
        display: &str,
        size: egui::Vec2,
        time: f64,
    ) -> (egui::FullOutput, std::time::Duration) {
        let settings = gevrek_ui::Settings::default();
        let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
        let display = DisplayId::new(display);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(time),
            focused: true,
            ..Default::default()
        };
        let began = std::time::Instant::now();
        let mut output = context.run_ui(input, |ui| {
            paint(zones, ui, &display, &Look { palette: &palette, settings: &settings, blur: false });
        });
        let took = began.elapsed();
        output.textures_delta.clear();
        (output, took)
    }

    /// Two thousand items: fifteen hundred in one zone that is scrolled
    /// through, five hundred loose on the desktop. A frame of it is
    /// timed (the numbers are printed, and written into the milestone's
    /// note by hand; they are not held to, beyond being far from a
    /// second), and what must not be done anew with every frame is held
    /// to exactly: laying out, asking for pictures, breaking names into
    /// lines.
    #[test]
    fn a_frame_of_two_thousand_items_does_nothing_twice() {
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![Display { primary: true, ..Display::new("big", gevrek_zones::Rect::new(0, 0, 2560, 1440)) }]);
        for number in 0..2000 {
            backend.add("/desk", Item::file(&format!("/desk/Document number {number:04}.pdf")));
        }
        let (mut zones, _) = zones_on(backend, "two-thousand");
        let all = zone(&mut zones, "Everything", ZoneKind::Manual, (700, 60, 1400, 1100));
        for number in 0..1500 {
            let item = ItemId::new(format!("/desk/Document number {number:04}.pdf"));
            zones.command(Command::AddItem { zone: all.clone(), item, index: None, cell: None });
        }
        zones.tick();
        let context = egui::Context::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let size = vec2(2560.0, 1440.0);
        let laying_out = std::time::Instant::now();
        let shown = zones.layout().zones[0].items.len() + zones.layout().desks[0].items.len();
        let laid_out_in = laying_out.elapsed();
        assert_eq!(shown, 2000);
        // The first frame asks for what is seen, and breaks its names.
        let (first, cold) = one_frame(&mut zones, &context, "big", size, 10.0);
        zones.tick();
        let (_, second) = one_frame(&mut zones, &context, "big", size, 10.1);
        zones.tick();
        let asked = zones.pictures.sizes_asked().len();
        let names = zones.labels.values().map(HashMap::len).sum::<usize>();
        let laid_out = zones.laid_out;
        // Lazy: only what is seen is asked for (the zone shows a few
        // rows of its fifteen hundred).
        assert!(asked > 100 && asked < 1200, "{asked} pictures asked for of 2000 items");
        // From here on a frame finds everything as it left it.
        let mut warm = Vec::new();
        for step in 0..5 {
            let (_, took) = one_frame(&mut zones, &context, "big", size, 10.2 + f64::from(step) * 0.1);
            warm.push(took);
        }
        assert_eq!(zones.laid_out, laid_out, "nothing changed: nothing is laid out again");
        assert_eq!(zones.pictures.sizes_asked().len(), asked, "no picture is asked for twice");
        assert_eq!(zones.labels.values().map(HashMap::len).sum::<usize>(), names, "no name is broken into lines twice");
        assert!(!zones.motion.busy(zones.seconds()), "at rest nothing asks for the next frame");
        warm.sort();
        let usual = warm[warm.len() / 2];
        println!(
            "2000 items at 2560x1440: laid out in {laid_out_in:?}; first frame {cold:?}, second {second:?}, \
             then {usual:?} a frame ({} shapes; {asked} pictures asked for, {names} names broken)",
            first.shapes.len()
        );
        assert!(usual < std::time::Duration::from_millis(900), "a frame takes {usual:?}");
        // Scrolled by a wheel's notch: laid out once, and only the rows
        // that came into sight are asked for.
        zones.scroll(pos2(1400.0, 600.0), -600.0);
        one_frame(&mut zones, &context, "big", size, 11.0);
        assert_eq!(zones.laid_out, laid_out + 1);
        let more = zones.pictures.sizes_asked().len();
        assert!(more > asked && more < asked + 400, "{asked} then {more}");
    }

    #[test]
    fn a_screen_reader_is_told_the_zones_and_their_items_and_nothing_is_made_for_nobody() {
        use egui::accesskit::{Role, Toggled};
        let _ = Toggled::True;
        let mut zones = a_working_desktop("spoken");
        let papers = zone_called(&zones, "Papers");
        let later = zone_called(&zones, "For later");
        zones.command(Command::FocusItem { item: Some(ItemId::new("/desk/Budget.xlsx")) });
        zones.command(Command::Select {
            items: vec![ItemId::new("/desk/Budget.xlsx"), ItemId::new("/desk/Offer.pdf")],
            mode: gevrek_zones::focus::SelectMode::Replace,
        });
        zones.command(Command::Lock { zone: papers.clone(), geometry: None, members: Some(true) });
        let context = egui::Context::default();
        gevrek_ui::theme::install_fonts(&context, false);
        // Nobody listens: no tree, and nothing is kept for one.
        let (silent, _) = one_frame(&mut zones, &context, "picture-1", vec2(1280.0, 800.0), 10.0);
        assert!(silent.platform_output.accesskit_update.is_none());
        assert!(zones.said.is_none() && zones.said_ids.is_empty());
        // A screen reader asks.
        context.enable_accesskit();
        one_frame(&mut zones, &context, "picture-1", vec2(1280.0, 800.0), 10.1);
        let (told, _) = one_frame(&mut zones, &context, "picture-1", vec2(1280.0, 800.0), 10.2);
        let update = told.platform_output.accesskit_update.expect("a tree for the screen reader");
        let named = |name: &str| {
            update.nodes.iter().find(|(_, node)| node.label() == Some(name)).unwrap_or_else(|| panic!("no node is called {name}"))
        };
        // A zone is a list, with what else there is to say of it.
        let (papers_id, list) = named("Papers");
        assert_eq!(list.role(), Role::ListBox);
        let description = list.description().unwrap_or_default().to_string();
        assert!(description.contains("8 items") && description.contains("locked"), "{description}");
        // Its items are the list's options: which one of how many,
        // chosen or not; and they are the list's children.
        let (budget_id, budget) = named("Budget.xlsx");
        assert_eq!(budget.role(), Role::ListBoxOption);
        assert_eq!((budget.position_in_set(), budget.size_of_set()), (Some(2), Some(8)));
        assert_eq!(budget.is_selected(), Some(true));
        assert_eq!(named("Minutes.txt").1.is_selected(), Some(false));
        assert!(list.children().contains(budget_id), "the item is told as being in its zone");
        // The keyboard is on the item the core says it is on.
        assert_eq!(update.focus, *budget_id);
        // A zone that is rolled up says so, and its items are not told.
        let (_, rolled) = named("For later");
        assert_eq!(rolled.is_expanded(), Some(false));
        assert!(!update.nodes.iter().any(|(_, node)| node.label() == Some("todo.txt")));
        // Tabs: the one in front is told, the other is not there to walk to.
        assert_eq!(named("Work").1.role(), Role::ListBox);
        assert!(!update.nodes.iter().any(|(_, node)| node.label() == Some("Home")));
        // The loose items are a list of their own.
        let (_, loose) = named("Desktop items");
        assert_eq!(loose.role(), Role::ListBox);
        assert_eq!(named("This PC").1.role(), Role::ListBoxOption);
        // What the screen reader asks for by a node is done: the
        // keyboard goes to an item, and a zone opens that is rolled up.
        let ask = |zones: &mut Zones, target: egui::accesskit::NodeId, action: egui::accesskit::Action, time: f64| {
            let request = egui::accesskit::ActionRequest {
                action,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: target,
                data: None,
            };
            let settings = gevrek_ui::Settings::default();
            let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                time: Some(time),
                focused: true,
                events: vec![egui::Event::AccessKitActionRequest(request)],
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                paint(zones, ui, &DisplayId::new("picture-1"), &Look { palette: &palette, settings: &settings, blur: false });
            });
            output.textures_delta.clear();
        };
        // The buttons of a zone's title are buttons of its list, each
        // called by what pressing it does now; pressed for the person,
        // it does that.
        let of_papers = |words: &str| {
            update
                .nodes
                .iter()
                .find(|(id, node)| node.role() == Role::Button && node.label() == Some(words) && list.children().contains(id))
                .map(|(id, _)| *id)
        };
        for words in ["Cover what it holds", "Shown and ordered", "Show as a list", "Nothing in or out. Press: let go", "Roll up", "Menu of the zone"] {
            assert!(of_papers(words).is_some(), "no button of Papers says {words:?}");
        }
        let roll = of_papers("Roll up").unwrap();
        ask(&mut zones, roll, egui::accesskit::Action::Click, 10.25);
        assert!(zones.core().config().zone(&papers).unwrap().rolled_up, "the button was pressed for the person");
        zones.command(Command::RollUp { zone: papers.clone(), rolled_up: Some(false) });
        one_frame(&mut zones, &context, "picture-1", vec2(1280.0, 800.0), 10.27);
        let minutes = named("Minutes.txt").0;
        ask(&mut zones, minutes, egui::accesskit::Action::Focus, 10.3);
        let on = zones.core().scene(0).focus.and_then(|on| on.item);
        assert_eq!(on, Some(ItemId::new("/desk/Minutes.txt")));
        let _ = (papers_id, later);
        // Covered from view, a zone tells its name and that it is
        // covered, and none of what it holds.
        zones.command(Command::Mask { masked: Some(true) });
        one_frame(&mut zones, &context, "picture-1", vec2(1280.0, 800.0), 10.4);
        let (covered, _) = one_frame(&mut zones, &context, "picture-1", vec2(1280.0, 800.0), 10.5);
        let update = covered.platform_output.accesskit_update.unwrap();
        assert!(update.nodes.iter().any(|(_, node)| node.label() == Some("Papers")));
        assert!(!update.nodes.iter().any(|(_, node)| node.label() == Some("Budget.xlsx")));
    }

    #[test]
    fn two_displays_of_different_scale_each_get_their_own_sharp_pictures() {
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![
            Display { primary: true, ..Display::new("left", gevrek_zones::Rect::new(0, 0, 1280, 800)) },
            Display { scale: 1.5, ..Display::new("right", gevrek_zones::Rect::new(1280, 0, 1280, 800)) },
        ]);
        for name in ["Holiday.png", "Garden.png"] {
            backend.add("/desk", Item::file(&format!("/desk/{name}")));
        }
        let (mut zones, _) = zones_on(backend, "two-scales");
        // A zone on each display, a picture in each.
        let across = zone(&mut zones, "On the left", ZoneKind::Manual, (100, 100, 400, 300));
        let other = zone(&mut zones, "On the right", ZoneKind::Manual, (1500, 500, 300, 200));
        for (zone, name) in [(&across, "Holiday.png"), (&other, "Garden.png")] {
            zones.command(Command::AddItem { zone: zone.clone(), item: ItemId::new(format!("/desk/{name}")), index: None, cell: None });
        }
        zones.tick();
        let (left, right) = (egui::Context::default(), egui::Context::default());
        right.set_pixels_per_point(1.5);
        for context in [&left, &right] {
            gevrek_ui::theme::install_fonts(context, false);
        }
        for step in 0..3 {
            let time = 10.0 + f64::from(step) * 0.1;
            let (on_left, _) = one_frame(&mut zones, &left, "left", vec2(1280.0, 800.0), time);
            let (on_right, _) = one_frame(&mut zones, &right, "right", vec2(1280.0, 800.0), time);
            zones.tick();
            // Each display draws its own zone, in its own points.
            assert!(on_left.shapes.len() > 5 && on_right.shapes.len() > 5);
        }
        // 48 units are 48 pixels on the one and 72 on the other: each
        // picture asked for in the pixels of the surface that shows it.
        let mut asked = zones.pictures.sizes_asked();
        asked.sort();
        assert_eq!(asked, [48, 72]);
        assert_eq!(zones.pictures.textures_made(), 2);
        assert!(zones.surface_scale(&DisplayId::new("right"), 1.5) && zones.surface_scale(&DisplayId::new("left"), 1.0));
        // A zone dragged from the one to the other is one movement in
        // the desktop's own units, whatever the scales.
        zones.pointer(&Pointer { at: Some(pos2(200.0, 112.0)), pressed: true, down: true, ..Pointer::default() });
        zones.pointer(&Pointer { at: Some(pos2(1900.0, 212.0)), down: true, alt: true, ..Pointer::default() });
        zones.pointer(&Pointer { at: Some(pos2(1900.0, 212.0)), released: true, ..Pointer::default() });
        assert_eq!(zones.core().zone_rect(&across), Some(gevrek_zones::Rect::new(1800, 200, 400, 300)));
        assert_eq!(zones.layout().zone(&across).unwrap().display, DisplayId::new("right"));
        // There its picture is asked for anew, in that surface's pixels:
        // sharp on the display it has come to, not stretched from the
        // one it left.
        for step in 0..3 {
            one_frame(&mut zones, &right, "right", vec2(1280.0, 800.0), 10.5 + f64::from(step) * 0.1);
            zones.tick();
        }
        let mut asked = zones.pictures.sizes_asked();
        asked.sort();
        assert_eq!(asked, [48, 72, 72]);
        assert_eq!(zones.pictures.textures_made(), 3);
        // Carried items are drawn on whichever display they are over.
        let item = zones.layout().item(&ItemId::new("/desk/Holiday.png")).unwrap().rect.center();
        zones.pointer(&Pointer { at: Some(item), pressed: true, down: true, ..Pointer::default() });
        zones.pointer(&Pointer { at: Some(pos2(600.0, 400.0)), down: true, ..Pointer::default() });
        assert!(zones.gestures.carrying());
        let (while_carried, _) = one_frame(&mut zones, &left, "left", vec2(1280.0, 800.0), 11.0);
        assert!(while_carried.shapes.len() > 2, "the carried item is drawn on the display it is over");
        zones.pointer(&Pointer { at: Some(pos2(600.0, 400.0)), released: true, ..Pointer::default() });
        assert_eq!(zones.core().zone_of(&ItemId::new("/desk/Holiday.png")), None);
        assert_eq!(zones.layout().desks[0].items.iter().filter(|shown| shown.name == "Holiday.png").count(), 1);
    }

    /// What milestone 5 added, drawn into pictures to be looked at:
    ///
    ///     cargo test -p gevrek-shell milestone_five_is_drawn -- --ignored
    ///
    /// writes `m5-*.png` into `target/zones-pictures/`.
    #[test]
    #[ignore = "draws pictures to look at; needs a graphics device that computes on the processor"]
    fn milestone_five_is_drawn_to_be_looked_at() {
        use serde_json::json;
        // Two private zones, one of them asked to be seen; the others as they are.
        let private = |zones: &mut Zones| {
            for title in ["Papers", "Downloads"] {
                let zone = zone_called(zones, title);
                zones.command(Command::ModifyZone { zone, changes: json!({ "private": true }) });
            }
            let downloads = zone_called(zones, "Downloads");
            zones.command(Command::Reveal { zone: Some(downloads), revealed: Some(true) });
        };
        shot("m5-private", 1.0, false, false, private);
        shot("m5-private-high-contrast", 1.0, true, false, private);
        // Everything covered by hand: the zones and the loose items.
        shot("m5-everything-covered", 1.0, false, false, |zones| {
            zones.command(Command::Mask { masked: Some(true) });
        });
        // The keyboard on a zone and on one of its items, another chosen
        // as well; at one and at one and a half.
        let keys = |zones: &mut Zones| {
            zones.command(Command::FocusItem { item: Some(ItemId::new("/desk/Budget.xlsx")) });
            zones.command(Command::MoveItemFocus { step: gevrek_zones::focus::Step::Right, extend: true });
            zones.command(Command::MoveItemFocus { step: gevrek_zones::focus::Step::Down, extend: true });
        };
        shot("m5-keyboard", 1.0, false, false, keys);
        shot("m5-keyboard-1.5x", 1.5, false, false, keys);
        shot("m5-keyboard-high-contrast", 1.0, true, false, keys);
        // A zone that keeps tiled windows off its place says so at the
        // end of its title, as it says everything else it is.
        shot("m5-zone-keeps-windows-off", 1.0, false, false, |zones| {
            let work = zone_called(zones, "Work");
            zones.command(Command::ModifyZone { zone: work, changes: json!({ "reserves_space": true, "always_visible": true }) });
        });
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
        let off = ZonesConfig { enabled: false, ..ZonesConfig::default() };
        zones.configure(&off);
        assert!(zones.spots().is_empty(), "no surface is asked for");
        assert!(zones.layout().zones.is_empty());
    }

    /// One frame of the one display's surface as a host gives it: what
    /// was heard since the frame before, and, where the host says it,
    /// the system's own time of each press. Gives the pointer's shape the
    /// frame asked for.
    fn frame_as_hosted(
        zones: &mut Zones,
        context: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        presses: Option<Vec<gevrek_shellkit::Press>>,
    ) -> egui::CursorIcon {
        let settings = gevrek_ui::Settings::default();
        let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
        let display = DisplayId::new("picture-1");
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        if let Some(presses) = presses {
            zones.pressed_at(presses);
        }
        let mut output = context.run_ui(input, |ui| {
            draw(zones, ui, &display, &Look { palette: &palette, settings: &settings, blur: false });
        });
        output.textures_delta.clear();
        output.platform_output.cursor_icon
    }

    fn papers(zones: &mut Zones) -> Rect {
        zones.layout().zones.iter().find(|zone| zone.title == "Papers").unwrap().rect
    }

    #[test]
    fn a_zone_is_sized_by_its_rim_as_a_surface_hears_it_and_the_pointer_shows_the_way() {
        use egui::CursorIcon;
        let mut zones = a_working_desktop("sized-by-rim");
        // (Written to the log as it goes, as on a desktop where
        // something does not react: nothing of it changes what happens.)
        zones.trace = true;
        let context = egui::Context::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let main = egui::PointerButton::Primary;
        let before = papers(&mut zones);
        // Over the lower right corner, a little inside: the arrows that
        // slant that way, before anything is pressed.
        let corner = pos2(before.right() - 3.0, before.bottom() - 3.0);
        let mut time = 10.0;
        let mut frame = |zones: &mut Zones, events: Vec<egui::Event>| {
            time += 0.05;
            frame_as_hosted(zones, &context, time, events, None)
        };
        assert_eq!(frame(&mut zones, vec![egui::Event::PointerMoved(corner)]), CursorIcon::ResizeNwSe);
        assert_eq!(zones.grip_shown().map(|(_, edges)| edges), Some(gevrek_zones::snap::Edges::BOTTOM_RIGHT));
        // Pressed and dragged, as Windows tells it: a press, then moves.
        frame(&mut zones, vec![button(corner, main, true)]);
        let mut shape = CursorIcon::Default;
        for step in 1..=9 {
            shape = frame(&mut zones, vec![egui::Event::PointerMoved(corner + vec2(step as f32 * 5.0, step as f32 * 4.0))]);
        }
        assert_eq!(shape, CursorIcon::ResizeNwSe, "while it is sized too");
        frame(&mut zones, vec![button(corner + vec2(45.0, 36.0), main, false)]);
        let after = papers(&mut zones);
        assert_eq!(after.min, before.min, "the corner that was not taken stays");
        assert!(after.width() > before.width() + 30.0 && after.height() > before.height() + 20.0, "{before:?} -> {after:?}");
        // By its left side alone, from a little outside it.
        let side = pos2(after.left() - 3.0, after.center().y);
        assert_eq!(frame(&mut zones, vec![egui::Event::PointerMoved(side)]), CursorIcon::ResizeHorizontal);
        frame(&mut zones, vec![button(side, main, true)]);
        frame(&mut zones, vec![egui::Event::PointerMoved(side + vec2(40.0, 0.0))]);
        frame(&mut zones, vec![button(side + vec2(40.0, 0.0), main, false)]);
        let narrower = papers(&mut zones);
        assert!(narrower.width() < after.width() - 20.0, "{after:?} -> {narrower:?}");
        assert_eq!((narrower.right(), narrower.height()), (after.right(), after.height()));
        // Off the rim the pointer is the arrow again, and no side is shown.
        assert_eq!(frame(&mut zones, vec![egui::Event::PointerMoved(narrower.left_top() + vec2(40.0, 60.0))]), CursorIcon::Default);
        assert_eq!(zones.grip_shown(), None);
        // Over a button of the title: the hand.
        let more = zones.layout().zones.iter().find(|zone| zone.title == "Pictures").unwrap().buttons.last().unwrap().rect.center();
        assert_eq!(frame(&mut zones, vec![egui::Event::PointerMoved(more)]), CursorIcon::PointingHand);
    }

    #[test]
    fn two_presses_are_a_double_press_at_the_second_whatever_else_the_frames_carry() {
        let hidden = |zones: &Zones| zones.core().scene(0).quick_hidden;
        let main = egui::PointerButton::Primary;
        let at = pos2(1100.0, 700.0);
        let gone = egui::Event::PointerGone;
        // (what the surface hears, frame by frame)
        let cases: Vec<(&str, Vec<Vec<egui::Event>>)> = vec![
            (
                "each press and each letting go a frame",
                vec![vec![button(at, main, true)], vec![button(at, main, false)], vec![button(at, main, true)], vec![button(at, main, false)]],
            ),
            (
                "the pointer told gone in the frame of the second press",
                vec![vec![button(at, main, true)], vec![button(at, main, false)], vec![button(at, main, true), gone.clone()]],
            ),
            (
                "the pointer told gone between the two",
                vec![vec![button(at, main, true)], vec![button(at, main, false), gone.clone()], vec![button(at, main, true)], vec![button(at, main, false), gone.clone()]],
            ),
            ("the button never told to have come up", vec![vec![button(at, main, true)], vec![button(at, main, true)]]),
            (
                "the keyboard gone to another window between the two",
                vec![
                    vec![egui::Event::WindowFocused(true), button(at, main, true)],
                    vec![button(at, main, false), egui::Event::WindowFocused(false)],
                    vec![button(at, main, true)],
                    vec![button(at, main, false)],
                ],
            ),
            ("all of it in one frame", vec![vec![button(at, main, true), button(at, main, false), button(at, main, true), button(at, main, false)]]),
        ];
        for (number, (what, heard)) in cases.into_iter().enumerate() {
            let mut zones = a_working_desktop(&format!("twice-{number}"));
            zones.trace = true;
            let context = egui::Context::default();
            gevrek_ui::theme::install_fonts(&context, false);
            assert!(!hidden(&zones));
            frames(&mut zones, &context, 10.0, heard);
            assert!(hidden(&zones), "{what}: two presses on the bare desktop hid nothing");
            // And twice again brings everything back.
            pressed_twice(&mut zones, &context, 20.0, at);
            assert!(!hidden(&zones), "{what}: two more brought nothing back");
        }
    }

    #[test]
    fn a_double_press_goes_by_when_the_system_saw_the_presses_not_by_when_the_frames_were_drawn() {
        let hidden = |zones: &Zones| zones.core().scene(0).quick_hidden;
        let main = egui::PointerButton::Primary;
        let at = pos2(1100.0, 700.0);
        let seen = |time_ms: u32| Some(vec![gevrek_shellkit::Press { button: main, at, time_ms }]);
        // A shell that was busy: the two frames are two seconds apart,
        // the two presses a fifth of a second.
        let mut zones = a_working_desktop("twice-late");
        let context = egui::Context::default();
        gevrek_ui::theme::install_fonts(&context, false);
        frame_as_hosted(&mut zones, &context, 10.0, vec![button(at, main, true)], seen(5_000));
        frame_as_hosted(&mut zones, &context, 10.1, vec![button(at, main, false)], Some(Vec::new()));
        frame_as_hosted(&mut zones, &context, 12.1, vec![button(at, main, true)], seen(5_200));
        assert!(hidden(&zones), "the presses were a fifth of a second apart");
        // And the other way: frames close together, presses far apart.
        let mut zones = a_working_desktop("twice-far");
        frame_as_hosted(&mut zones, &context, 20.0, vec![button(at, main, true)], seen(5_000));
        frame_as_hosted(&mut zones, &context, 20.05, vec![button(at, main, false)], Some(Vec::new()));
        frame_as_hosted(&mut zones, &context, 20.1, vec![button(at, main, true)], seen(9_000));
        assert!(!hidden(&zones), "four seconds apart is two presses");
        // Two presses far from each other on the desktop are two presses.
        let mut zones = a_working_desktop("twice-apart");
        let there = pos2(1140.0, 700.0);
        frames(&mut zones, &context, 30.0, vec![vec![button(at, main, true)], vec![button(at, main, false)], vec![button(there, main, true)]]);
        assert!(!hidden(&zones));
        // On a zone's title they roll it up, as before.
        let mut zones = a_working_desktop("twice-title");
        let title = pos2(300.0, 52.0);
        pressed_twice(&mut zones, &context, 40.0, title);
        assert!(zones.layout().zones.iter().find(|zone| zone.title == "Papers").unwrap().rolled_up);
    }

    #[test]
    fn the_middle_button_twice_on_the_bare_desktop_makes_every_icon_grey_and_twice_again_lets_them_be() {
        let middle = egui::PointerButton::Middle;
        let at = pos2(1100.0, 700.0);
        let twice = || vec![vec![button(at, middle, true)], vec![button(at, middle, false)], vec![button(at, middle, true)], vec![button(at, middle, false)]];
        let mut zones = a_working_desktop("grey-twice");
        let context = egui::Context::default();
        gevrek_ui::theme::install_fonts(&context, false);
        // Once is nothing.
        frames(&mut zones, &context, 10.0, vec![vec![button(at, middle, true)], vec![button(at, middle, false)]]);
        assert!(!zones.core().grey_icons());
        frames(&mut zones, &context, 20.0, twice());
        assert!(zones.core().grey_icons() && zones.layout().grey);
        // What the zones say of their own icons is as it was: this lies
        // over it, and is taken off again.
        assert!(zones.core().config().zones.iter().all(|zone| zone.look.icon_saturation.is_none()));
        assert!(zones.core().config().look.icon_saturation.is_none());
        frames(&mut zones, &context, 30.0, twice());
        assert!(!zones.core().grey_icons() && !zones.layout().grey);
        // On a zone it is nothing: the gesture is the bare desktop's.
        let on_a_zone = pos2(300.0, 200.0);
        frames(
            &mut zones,
            &context,
            40.0,
            vec![vec![button(on_a_zone, middle, true)], vec![button(on_a_zone, middle, false)], vec![button(on_a_zone, middle, true)]],
        );
        assert!(!zones.core().grey_icons());
        // The main and the middle button are counted each by itself.
        let main = egui::PointerButton::Primary;
        frames(&mut zones, &context, 50.0, vec![vec![button(at, main, true)], vec![button(at, main, false)], vec![button(at, middle, true)]]);
        assert!(!zones.core().grey_icons() && !zones.core().scene(0).quick_hidden);
    }

    #[test]
    fn grey_is_laid_over_what_a_zone_says_of_its_icons_and_changes_nothing_of_it() {
        let coloured = Tone { saturation: 100, tint: [10, 20, 30, 102] };
        assert_eq!(coloured.or_grey(false), coloured);
        assert_eq!(coloured.or_grey(true), Tone::GREY);
        assert_eq!(Tone::default().or_grey(true).apply([200, 40, 40]), [88, 88, 88]);
        assert_eq!(Tone::default().or_grey(false).apply([200, 40, 40]), [200, 40, 40]);
    }

    /// What was added after the first days on a real desktop, drawn into
    /// pictures to be looked at:
    ///
    ///     cargo test -p gevrek-shell milestone_six_is_drawn -- --ignored
    ///
    /// writes `m6-*.png` into `target/zones-pictures/`.
    #[test]
    #[ignore = "draws pictures to look at; needs a graphics device that computes on the processor"]
    fn milestone_six_is_drawn_to_be_looked_at() {
        use crate::zones::menu::{self, Chain};
        use crate::zones::surfaces;
        let folder = pictures_folder();
        std::fs::create_dir_all(&folder).unwrap();
        // The buttons of a title under the pointer (one of them under it
        // exactly), the corner a zone is sized by, and beside it a zone
        // whose title is as it is without the pointer.
        let hover = |zones: &mut Zones| {
            let roll = zones.layout().zones.iter().find(|zone| zone.title == "Papers").unwrap().buttons[4].rect.center();
            zones.hover = Some(roll);
        };
        shot("m6-title-buttons", 1.0, false, false, hover);
        shot("m6-title-buttons-1.5x", 1.5, false, false, hover);
        // With high contrast every title shows its buttons always.
        shot("m6-title-buttons-high-contrast", 1.0, true, false, hover);
        // The keyboard on a button of the title of the zone that has it.
        shot("m6-title-buttons-keyboard", 1.0, false, false, |zones| {
            let papers = zone_called(zones, "Papers");
            zones.command(Command::FocusZone { zone: Some(papers.clone()) });
            zones.layout();
            zones.gestures.title_at = Some((papers, view::Button::Lock));
        });
        // The pointer on a zone's lower right corner: the side and the
        // corner it would take, in the accent colour.
        shot("m6-resize-grip", 1.0, false, false, |zones| {
            let rect = zones.layout().zones.iter().find(|zone| zone.title == "Pictures").unwrap().rect;
            zones.hover = Some(rect.right_bottom() - vec2(3.0, 3.0));
        });
        // Every icon grey.
        shot("m6-grey-icons", 1.0, false, false, |zones| {
            zones.command(Command::GreyIcons { on: Some(true) });
        });
        // Menus that opened from each other: the desktop's, the look of
        // all zones beside its line, and the ground beside that.
        for (name, strong) in [("m6-menus", false), ("m6-menus-high-contrast", true)] {
            let mut zones = a_working_desktop(name);
            let size = (1280, 800);
            let mut painter = gevrek_ui::picture::Painter::sized(size).unwrap();
            let context = egui::Context::default();
            let mut settings = gevrek_ui::Settings::default();
            if strong {
                settings.preset = gevrek_ui::Preset::ALL.into_iter().find(|preset| preset.is_high_contrast()).unwrap();
            }
            gevrek_ui::theme::install_fonts(&context, false);
            let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
            gevrek_ui::theme::apply(&context, &palette, &settings);
            let display = DisplayId::new("picture-1");
            let at = pos2(574.0, 482.0);
            let mut chain = Chain::of(surfaces::desk_menu(&zones, &display, at, &palette), false);
            let line = |chain: &Chain, level: usize, label: &str| {
                chain.levels[level].lines.iter().position(|line| matches!(line, surfaces::Line::Sub { label: has, .. } if has == label)).unwrap()
            };
            let look = line(&chain, 0, "Look of all zones");
            chain.pressed(0, look);
            let ground = line(&chain, 1, "Ground");
            chain.pressed(1, ground);
            chain.pointed(2, Some(3), 0.0);
            // Each beside the line it opened from, as the host places
            // them: to its right, on its other side where the display
            // ends, and slid up where it would leave the display below.
            let first = menu::menu_size(&chain.levels[0].lines);
            let mut places = vec![Rect::from_min_size(pos2(at.x, (at.y + 4.0).min(800.0 - first.y)), first)];
            for level in 1..chain.levels.len() {
                let from = chain.levels[level - 1].open.unwrap();
                let row = menu::rows(&chain.levels[level - 1].lines)[from].translate(places[level - 1].min.to_vec2());
                let size = menu::menu_size(&chain.levels[level].lines);
                let left = if row.right() - 3.0 + size.x <= 1280.0 { row.right() - 3.0 } else { row.left() + 3.0 - size.x };
                places.push(Rect::from_min_size(pos2(left, row.top().min(800.0 - size.y)), size));
            }
            let mut pixels = Vec::new();
            for step in 0..3 {
                zones.tick();
                pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.2, Vec::new(), |ui| {
                    wallpaper(ui, vec2(1280.0, 800.0));
                    paint(&mut zones, ui, &display, &Look { palette: &palette, settings: &settings, blur: false });
                    for (level, place) in chain.levels.iter().zip(&places) {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(*place).id_salt(level.serial), |ui| {
                            surfaces::menu(ui, &palette, level);
                        });
                    }
                });
                zones.later(0.6);
            }
            let file = folder.join(format!("{name}.png"));
            painter.save(&pixels, &file).unwrap();
            println!("drawn: {}", file.display());
        }
        // The sheet of the zones that would be made by type, as its
        // surface shows it; and what the desktop is after they were made.
        for (name, strong) in [("m6-zones-by-type", false), ("m6-zones-by-type-high-contrast", true)] {
            let mut zones = a_desk_to_sort(name);
            let display = DisplayId::new("picture-1");
            let preview = zones.auto_preview(gevrek_zones::rules::Scope::Loose, &display);
            assert!(preview.plan.zones.len() >= 4, "{:?}", preview.plan);
            let size = (640, 520);
            let mut painter = gevrek_ui::picture::Painter::sized(size).unwrap();
            let context = egui::Context::default();
            let mut settings = gevrek_ui::Settings::default();
            if strong {
                settings.preset = gevrek_ui::Preset::ALL.into_iter().find(|preset| preset.is_high_contrast()).unwrap();
            }
            gevrek_ui::theme::install_fonts(&context, false);
            let palette = settings.resolve(egui::Theme::Dark, Color32::from_rgb(64, 224, 208));
            gevrek_ui::theme::apply(&context, &palette, &settings);
            let mut pixels = Vec::new();
            for step in 0..2 {
                pixels = painter.frame_of(&context, 10.0 + f64::from(step) * 0.2, Vec::new(), |ui| {
                    surfaces::auto_sheet(ui, &palette, &preview, true, None);
                });
            }
            let file = folder.join(format!("{name}.png"));
            painter.save(&pixels, &file).unwrap();
            println!("drawn: {}", file.display());
        }
        let mut zones = a_desk_to_sort("m6-zones-by-type-made");
        zones.command(Command::AutoCreate { scope: gevrek_zones::rules::Scope::Loose, min_items: None, display: None, add_rules: None, dry_run: false });
        zones.tick();
        let (pixels, _, painter) = pictured(&mut zones, 1.0, None);
        let file = folder.join("m6-zones-by-type-made.png");
        painter.save(&pixels, &file).unwrap();
        println!("drawn: {}", file.display());
    }

    /// A desktop nobody has tidied: files of many types lying loose
    /// beside one zone that was made by hand.
    pub fn a_desk_to_sort(name: &str) -> Zones {
        let mut backend = FakeBackend::new();
        backend.set_displays(vec![Display { primary: true, ..Display::new("picture-1", gevrek_zones::Rect::new(0, 0, 1280, 800)) }]);
        for file in [
            "Annual report 2026.pdf", "Budget.xlsx", "Letter to the council.docx", "Minutes.txt", "Offer.pdf",
            "Holiday.png", "Harbour at night.jpg", "Garden.png", "Scan_0042.png",
            "Concert.mp4", "Lecture.mkv",
            "song.mp3", "interview.flac", "jingle.wav",
            "gevrek-0.6.0.tar.gz", "photos-2025.zip",
            "main.rs", "build.py", "notes.toml",
            "strange.xyz",
        ] {
            backend.add("/desk", Item::file(&format!("/desk/{file}")));
        }
        backend.add("/desk", Item::folder("/desk/Projects"));
        backend.add("/desk", Item::folder("/desk/Archive"));
        for name in ["Editor.lnk", "Terminal.lnk"] {
            let mut item = Item::file(&format!("/desk/{name}"));
            item.kind = ItemKind::Shortcut;
            backend.add("/desk", item);
        }
        let (mut zones, _) = zones_on(backend, name);
        zone(&mut zones, "Today", ZoneKind::Manual, (900, 520, 340, 220));
        zones.tick();
        zones
    }
}
