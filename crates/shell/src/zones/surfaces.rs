//! The surfaces the zones are drawn on: one for each display, below the
//! windows and as large as the display, and beside them a menu or the box
//! a name is typed into while one is asked for.
//!
//! **The whole surface is ours.** Nothing of it lets a press through to
//! what lies under it. The bare desktop between the zones is where the
//! loose items are, where a rectangle is drawn to select with, where the
//! desktop's menu opens and where a double click hides everything; and
//! on Windows the system's own icons, which would lie under it, are put
//! away while zones are shown (design.md, decision A). A surface that let
//! presses through wherever no zone is could do none of that.
//!
//! **It has the keyboard only when it is pressed** (`Keyboard::OnDemand`),
//! as a press on any desktop takes the keys from the window that had
//! them: then the arrows, Enter, F2, Delete and letters go to the items.
//! It never takes the keyboard by itself: not when it is shown, not when
//! the pointer is over it, not when it is drawn again. Nothing in the
//! drawing asks for it (a test holds that). The box for a name and the one
//! for what is looked for are surfaces of their own, which have the
//! keyboard for as long as they are open.

use super::draw::{self, Look};
use super::menu::{self, menu_size, Chain};
use super::{Ask, DeskAction, Spot, Zones};
use egui::{pos2, vec2, Pos2, Rect};
use gevrek_shellkit::dnd::{DragAction, DragEvent, DragOutcome, DragPayload, DragStart, DropAnswer};
use gevrek_shellkit::host::{Control, SurfaceId};
use gevrek_zones::backend::{DragActions, DropPayload};
use gevrek_shellkit::{Edge, Flow, Keyboard, Level, Place};
use gevrek_ui::theme::Palette;
use gevrek_zones::config::{Sort, View, ZoneKind};
use gevrek_zones::core::Command;
use gevrek_zones::rules::Scope;
use gevrek_zones::{DisplayId, ZoneId};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The zones, as the surfaces share them.
pub type Shared = Rc<RefCell<Zones>>;

/// The colours and fonts of a frame, as the shell's own surfaces have
/// them: each half of the shell knows how (`themed`).
pub type Themed = Rc<dyn Fn(&egui::Ui, &mut bool) -> (Palette, gevrek_ui::Settings)>;

/// The name the surfaces are known by to the desktop (for its rules). On
/// Windows it is the end of their window class, by which the window
/// manager knows a press on them for a press on the desktop
/// (`gevrek_win32::windows_key`): the two must change together.
pub const SURFACE: &str = "gevrek-zones";

/// The surfaces that are open.
#[derive(Default)]
pub struct Surfaces {
    /// Of each display, with where it was put.
    displays: BTreeMap<DisplayId, (SurfaceId, Spot)>,
    /// The box that has the keyboard, while there is one.
    typing: Option<SurfaceId>,
    /// The menu that is open, while there is one, with the menus that
    /// opened from it: never two that did not. (Their surfaces may have
    /// closed themselves since, or been closed by the host:
    /// `Control::is_open` is asked before one is closed.)
    menu: Option<Rc<RefCell<OpenMenus>>>,
    /// What is said of the button of a title the pointer rests on, while
    /// it is said: the surface that says it, and the display and the
    /// words it was opened for.
    hint: Option<(SurfaceId, DisplayId, String)>,
    /// The sheet that shows what the rules would do, while it is open.
    sheet: Option<SurfaceId>,
    /// Whether nothing of the zones was to be seen when this was last
    /// looked at (`Zones::put_away`).
    away: bool,
    /// Whether the displays' surfaces stand above the windows (the zones
    /// are looked at), as they were last told to.
    above: bool,
    /// The parts of each display's surface behind which the desktop is
    /// shown soft, as its host was last told.
    soft: BTreeMap<DisplayId, Vec<gevrek_shellkit::look::BlurRegion>>,
    /// Whether the displays' surfaces are left out of pictures taken of
    /// the screen, as they were last told to be.
    unpictured: bool,
    /// The room kept clear of tiled windows for zones that ask for it:
    /// by display and side, the surface that keeps it and how deep.
    rooms: BTreeMap<(DisplayId, Side), (SurfaceId, u32)>,
}

/// The menus that are open: how they stand (`menu::Chain`), and the
/// surface of each, the first menu's first; each with what tells its menu
/// from every other (a menu that took another's place beside the same
/// menu is another menu, and has another surface).
pub struct OpenMenus {
    pub chain: Chain,
    surfaces: Vec<(u64, SurfaceId)>,
    /// How the menus stood when their surfaces were last asked to draw
    /// (each menu, the line the keys are on, the line that leads on):
    /// they are asked again only when that is another, so that menus at
    /// rest draw nothing.
    drawn: Vec<(u64, Option<usize>, Option<usize>)>,
}

impl Surfaces {
    /// The surface of the menu that is open (of the first, where menus
    /// opened from it).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn menu(&self) -> Option<SurfaceId> {
        self.menu.as_ref().and_then(|open| open.borrow().surfaces.first().map(|(_, surface)| *surface))
    }

    /// The surfaces of all menus that are open, the first menu's first.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn menus(&self) -> Vec<SurfaceId> {
        self.menu.as_ref().map(|open| open.borrow().surfaces.iter().map(|(_, surface)| *surface).collect()).unwrap_or_default()
    }
}

/// A side of a display that room is kept at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// The room to keep clear of tiled windows, from the regions the core
/// names: for each display and side one strip, as deep as the deepest
/// region kept there (strips of one side lie over each other, they do
/// not add up).
pub fn rooms_for(regions: &[gevrek_zones::reserve::ReservedRegion]) -> BTreeMap<(DisplayId, Side), u32> {
    use gevrek_zones::geometry::Edge as At;
    let mut rooms: BTreeMap<(DisplayId, Side), u32> = BTreeMap::new();
    for region in regions.iter().filter(|region| region.thickness > 0) {
        let side = match region.edge {
            At::Top => Side::Top,
            At::Bottom => Side::Bottom,
            At::Left => Side::Left,
            At::Right => Side::Right,
        };
        let depth = rooms.entry((region.display.clone(), side)).or_default();
        *depth = (*depth).max(region.thickness as u32);
    }
    rooms
}

/// Open a surface with nothing on it that keeps `depth` units clear at a
/// side of a display: tiled windows keep off it as they keep off a
/// panel. The pointer goes through it. (A zones surface fills its
/// display and can keep no room itself.)
///
/// On Windows nothing is opened: there a surface that keeps room becomes
/// an application bar, which changes the work area of every program.
/// The room is told to the window manager with the zones' snapshot
/// (`reserved`) instead, for it to keep.
fn open_room(control: &mut Control, side: Side, depth: u32, output: &str) -> Option<SurfaceId> {
    #[cfg(windows)]
    {
        let _ = (control, side, depth, output);
        None
    }
    #[cfg(not(windows))]
    {
        let edge = match side {
            Side::Top => Edge::Top,
            Side::Bottom => Edge::Bottom,
            Side::Left => Edge::Left,
            Side::Right => Edge::Right,
        };
        let place = Place::reserved_strip("gevrek-zones-room", edge, depth, Some(output.to_string()));
        let surface = control.open(place, |_, _| Flow::Continue);
        control.reach(surface, Vec::new());
        Some(surface)
    }
}

/// Where a display's surface stands among the windows, and how it comes
/// by the keyboard: below them, with the keyboard only at a press; and
/// while the zones are looked at (`Command::Peek`), above them with the
/// keyboard for itself, so that what is typed is looked for at once.
/// This is the one way the surface ever has the keyboard without a
/// press: by a command the person gave to look at the zones.
pub fn stands(looked_at: bool) -> (Level, Keyboard) {
    if looked_at {
        (Level::Overlay, Keyboard::Exclusive)
    } else {
        (Level::Below, Keyboard::OnDemand)
    }
}

/// Put away the menu and the box that are open beside the displays'
/// surfaces, if any is.
///
/// The host closes a menu at a press outside the shell's surfaces, and
/// the box closes itself when it loses the keyboard. Neither is enough
/// here: the displays' surfaces cover every display, so a press beside a
/// menu is a press on one of the shell's surfaces, which the host leaves
/// to that surface to act on; and where the box holds the keyboard for
/// itself alone, a press on the desktop takes nothing from it. So the
/// displays' surfaces say it themselves: at every press on one of them,
/// at Escape, and when everything is hidden.
pub fn dismiss(zones: &Shared, surfaces: &Rc<RefCell<Surfaces>>, control: &mut Control) {
    let (menu, typing, sheet) = {
        let mut open = surfaces.borrow_mut();
        (open.menu.take(), open.typing.take(), open.sheet.take())
    };
    if let Some(menu) = menu {
        close_menus(&menu, control);
    }
    for surface in typing.into_iter().chain(sheet) {
        if control.is_open(surface) {
            control.close(surface);
        }
    }
    if typing.is_some() {
        // (What was found stays found, as when the box loses the keyboard.)
        zones.borrow_mut().renaming = None;
    }
}

/// What a display's surface asks of the desktop.
fn place(spot: &Spot) -> Place {
    // (Another instance of gevrek on the machine has its own name, as its
    // panel has.)
    let instance = gevrek_model::paths::instance();
    let name = if instance == gevrek_model::paths::MAIN_INSTANCE {
        SURFACE.to_string()
    } else {
        format!("{SURFACE}.{instance}")
    };
    Place {
        name,
        edge: Edge::Fill,
        level: Level::Below,
        // (All of the display, whatever its size becomes.)
        width: 0,
        height: 0,
        reserve: false,
        // The keyboard comes with a press on the surface, and no other way.
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: Some(spot.output.clone()),
    }
}

/// Open the surface of a display. Windows puts a surface where it is
/// told, in the screen's pixels; a Wayland compositor puts it on the
/// display that is named.
fn open(
    control: &mut Control,
    spot: &Spot,
    draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
) -> SurfaceId {
    #[cfg(windows)]
    {
        let (x, y, width, height) = spot.pixels;
        control.open_at(place(spot), gevrek_shellkit::win::place::Area::new(x, y, width, height), draw)
    }
    #[cfg(not(windows))]
    {
        control.open(place(spot), draw)
    }
}

/// Make the open surfaces those the zones want: one for each display
/// there is while zones are switched on, none while they are off.
pub fn sync(zones: &Shared, surfaces: &Rc<RefCell<Surfaces>>, control: &mut Control, themed: &Themed) {
    let (wanted, away, looked_at, unpictured, rooms) = {
        let mut zones = zones.borrow_mut();
        let shown = zones.core().enabled();
        (
            zones.spots(),
            zones.put_away(),
            shown && zones.core().peeking(),
            shown && zones.kept_from_pictures(),
            if shown { rooms_for(&zones.core().reserved_regions()) } else { BTreeMap::new() },
        )
    };
    // Everything was just hidden, or switched off: a menu that is still
    // open would be of a desktop that is no longer shown.
    let was_away = std::mem::replace(&mut surfaces.borrow_mut().away, away);
    if away && !was_away {
        dismiss(zones, surfaces, control);
    }
    let mut open_now = surfaces.borrow_mut();
    // Gone, moved, or closed by the host itself (its display went).
    let stale: Vec<DisplayId> = open_now
        .displays
        .iter()
        .filter(|(display, (surface, spot))| {
            !control.is_open(*surface) || !wanted.iter().any(|(wanted, at)| wanted == *display && at == spot)
        })
        .map(|(display, _)| display.clone())
        .collect();
    for display in stale {
        open_now.soft.remove(&display);
        if let Some((surface, _)) = open_now.displays.remove(&display) {
            if control.is_open(surface) {
                log::debug!("the zones' surface of {display} is closed: it is not wanted there any more, or is elsewhere");
                control.close(surface);
            } else {
                log::debug!("the zones' surface of {display} was closed by the system (its display went)");
            }
        }
    }
    // Something private is shown, or no longer: every surface that is
    // open is left out of pictures of the screen, or let into them again.
    if open_now.unpictured != unpictured {
        log::debug!("the zones' surfaces are {} pictures of the screen", if unpictured { "left out of" } else { "let into" });
        open_now.unpictured = unpictured;
        for (surface, _) in open_now.displays.values() {
            control.exclude_from_capture(*surface, unpictured);
        }
    }
    // The room kept clear of tiled windows: a strip at a display's side
    // for each zone that asks, made as deep as the core says, and gone
    // with the zone (or its display).
    let gone: Vec<(DisplayId, Side)> = open_now
        .rooms
        .iter()
        .filter(|(at, (surface, _))| {
            !control.is_open(*surface) || !rooms.contains_key(at) || !wanted.iter().any(|(display, _)| display == &at.0)
        })
        .map(|(at, _)| at.clone())
        .collect();
    for at in gone {
        if let Some((surface, _)) = open_now.rooms.remove(&at) {
            if control.is_open(surface) {
                control.close(surface);
            }
        }
    }
    for (at, depth) in rooms {
        let Some((_, spot)) = wanted.iter().find(|(display, _)| display == &at.0) else {
            continue;
        };
        match open_now.rooms.get_mut(&at) {
            Some((surface, kept)) if *kept != depth => {
                let (wide, high) = if matches!(at.1, Side::Left | Side::Right) { (depth, 0) } else { (0, depth) };
                control.resize(*surface, wide, high);
                *kept = depth;
            }
            Some(_) => {}
            None => {
                if let Some(surface) = open_room(control, at.1, depth, &spot.output) {
                    open_now.rooms.insert(at, (surface, depth));
                }
            }
        }
    }
    // Looked at, or no longer: every surface that is open goes above the
    // windows and takes the keyboard, or goes back below and lets it go.
    if open_now.above != looked_at {
        log::debug!("the zones' surfaces go {} the windows", if looked_at { "above" } else { "back below" });
        open_now.above = looked_at;
        let (level, keyboard) = stands(looked_at);
        for (surface, _) in open_now.displays.values() {
            control.set_level(*surface, level);
            control.set_keyboard(*surface, keyboard);
        }
    }
    for (display, spot) in wanted {
        if open_now.displays.contains_key(&display) {
            continue;
        }
        let shared = Rc::clone(zones);
        let all = Rc::clone(surfaces);
        let look = Rc::clone(themed);
        let here = display.clone();
        let mut fonts = false;
        let surface = open(control, &spot, move |ui, control| {
            let (palette, settings) = look(ui, &mut fonts);
            // While a menu is open the keys that walk it are the menu's,
            // whichever surface hears them: where menus never have the
            // keyboard (Windows) it is this one. Taken before anything
            // else looks at the keys, so that an arrow moves in the menu
            // and not among the items, and Escape goes one menu back.
            let open_menu = all.borrow().menu.clone();
            if let Some(open_menu) = open_menu {
                let keys = ui.input_mut(menu::keys);
                if !menu_keys(&shared, &open_menu, &look, keys, control) {
                    all.borrow_mut().menu = None;
                }
            }
            // A press here, with either button, is a press beside the
            // menu or the box that is open: it puts them away, and then
            // does what it does (the other button opens the next menu,
            // below). Escape too, while this surface has the keyboard.
            if ui.input(pressed_elsewhere) {
                dismiss(&shared, &all, control);
            }
            // What the system shows on the zones' behalf while this
            // surface is worked on (its menu, its progress) belongs to it.
            #[cfg(windows)]
            {
                let own = all.borrow().displays.get(&here).map(|(surface, _)| *surface);
                if let Some(window) = own.and_then(|own| control.native_window(own)) {
                    shared.borrow_mut().desk.set_owner(window);
                }
            }
            let soft_shown = control.blur_available();
            let own = all.borrow().displays.get(&here).map(|(surface, _)| *surface);
            let (asks, others, soft, said) = {
                let mut zones = shared.borrow_mut();
                // When the system saw each press that this frame hears
                // of: what a double press is counted by.
                if let Some(own) = own {
                    zones.pressed_at(control.presses(own));
                }
                let look = Look { palette: &palette, settings: &settings, blur: soft_shown };
                let soft = draw::draw(&mut zones, ui, &here, &look);
                let said = zones.hint_on(&here);
                (zones.take_asks(), std::mem::take(&mut zones.redraw), soft, said)
            };
            if let Some(own) = own {
                say(&all, &look, &here, own, said, ui, control);
            }
            if let Some(own) = own {
                // The grounds behind which the desktop is shown soft: said
                // again only when one of them is elsewhere.
                let mut all = all.borrow_mut();
                if all.soft.get(&here).map(Vec::as_slice).unwrap_or_default() != soft.as_slice() {
                    control.set_blur_regions(own, &soft);
                    all.soft.insert(here.clone(), soft);
                }
            }
            if let Some(own) = own {
                drags(&shared, &here, own, ui.ctx().pixels_per_point(), control);
            }
            let others = others || std::mem::take(&mut shared.borrow_mut().redraw);
            // What changed here may be seen on the other displays too.
            if others {
                for (surface, _) in all.borrow().displays.values() {
                    if Some(*surface) != own {
                        control.redraw(*surface);
                    }
                }
            }
            if let Some(own) = own {
                for ask in asks {
                    answer(&shared, &all, &look, &palette, ask, &here, own, control);
                }
            }
            Flow::Continue
        });
        // Until a drawing says more: our own items, moved. (The answer a
        // drawing gives is seen a frame later; most of what is dragged
        // over the desktop is the desktop's own.)
        control.accept_drops(surface, Some(STANDING));
        // (Opened while the zones are looked at: above with the others.)
        if looked_at {
            let (level, keyboard) = stands(true);
            control.set_level(surface, level);
            control.set_keyboard(surface, keyboard);
        }
        if unpictured {
            control.exclude_from_capture(surface, true);
        }
        log::debug!("the zones' surface of {display} is opened on {} as surface {surface:?}", spot.output);
        open_now.displays.insert(display, (surface, spot));
    }
}

/// How large a hint is for its words, in points.
fn hint_size(ui: &egui::Ui, words: &str) -> egui::Vec2 {
    let wide = ui.painter().layout_no_wrap(words.to_string(), egui::FontId::proportional(12.0), egui::Color32::WHITE).size().x;
    vec2((wide + 20.0).ceil(), 26.0)
}

/// Say what the button of a title is that the pointer rests on, on a
/// small surface of its own under the button; and take it away when the
/// pointer has gone on. One at a time, and none while nothing is rested
/// on. (Only to be read: the pointer and its presses go through it.)
fn say(
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    display: &DisplayId,
    own: SurfaceId,
    said: Option<(Rect, String)>,
    ui: &egui::Ui,
    control: &mut Control,
) {
    let mut open = surfaces.borrow_mut();
    let same = match (&open.hint, &said) {
        (Some((surface, on, words)), Some((_, wanted))) => on == display && words == wanted && control.is_open(*surface),
        // (What another display says is that display's to take away.)
        (Some((_, on, _)), None) => on != display,
        (None, None) => true,
        (None, Some(_)) => false,
    };
    if same {
        return;
    }
    if let Some((surface, ..)) = open.hint.take() {
        if control.is_open(surface) {
            control.close(surface);
        }
    }
    let Some((anchor, words)) = said else {
        return;
    };
    let look = Rc::clone(themed);
    let shown = words.clone();
    let mut fonts = false;
    let surface = control.hint(own, anchor, hint_size(ui, &words), false, move |ui, _| {
        let (palette, _) = look(ui, &mut fonts);
        let all = ui.max_rect();
        ui.painter().rect_filled(all, 6.0, palette.surface);
        ui.painter().rect_stroke(all, 6.0, egui::Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
        ui.painter().text(all.center(), egui::Align2::CENTER_CENTER, &shown, egui::FontId::proportional(12.0), palette.text);
        Flow::Continue
    });
    open.hint = Some((surface, display.clone(), words));
}

/// Whether what a display's surface heard since its last frame puts away
/// the menu and the box beside it: a button of the pointer went down on
/// it, or Escape was pressed while it has the keyboard.
fn pressed_elsewhere(input: &egui::InputState) -> bool {
    input.pointer.any_pressed() || (input.focused && input.key_pressed(egui::Key::Escape))
}

/// What a surface takes before its drawing has looked at a drag.
const STANDING: DropAnswer = DropAnswer {
    accept: gevrek_shellkit::dnd::DragActions { copy: false, moving: true, link: false },
    prefer: Some(DragAction::Move),
};

fn to_zones(actions: gevrek_shellkit::dnd::DragActions) -> DragActions {
    DragActions { copy: actions.copy, moving: actions.moving, link: actions.link }
}

/// What the zones would do, as an answer to the system.
fn answer_of(would: DragActions) -> DropAnswer {
    let accept = gevrek_shellkit::dnd::DragActions { copy: would.copy, moving: would.moving, link: false };
    match (would.moving, would.copy) {
        (true, _) => DropAnswer::preferring(accept, DragAction::Move),
        (false, true) => DropAnswer::accept(accept),
        (false, false) => DropAnswer::REFUSE,
    }
}

thread_local! {
    /// What the drag that is over one of our surfaces carries, as far as
    /// it is known before it is let go.
    static OVER: RefCell<Option<DropPayload>> = const { RefCell::new(None) };
}

/// The drags of one surface: begin the one the zones ask for, and take in
/// what the system says of drags over it.
fn drags(zones: &Shared, display: &DisplayId, own: SurfaceId, scale: f32, control: &mut Control) {
    let mut zones = zones.borrow_mut();
    if let Some((items, paths, picture)) = zones.take_drag_out() {
        let image = picture.map(|picture| gevrek_shellkit::dnd::DragImage {
            hotspot: (picture.width as i32 / 2, picture.height as i32 / 2),
            width: picture.width,
            height: picture.height,
            rgba: picture.pixels,
            scale,
        });
        // With the backend's own description of the items where it has
        // one (it names what is no file too, and we know our items by it
        // when they are let go on a zone); else as files, by their paths.
        if !zones.desk.begin_drag(&items, control, own, image.clone()) {
            log::debug!("the drag is handed to the system as {}", gevrek_zones::timed::count(paths.len(), "file", "files"));
            let mut payload = DragPayload::files(paths);
            // (A link to a file of the desktop is not ours to offer.)
            payload.allowed.link = false;
            control.start_drag(own, DragStart { payload, image });
        }
    }
    let events = control.drag_events(own);
    if events.is_empty() {
        return;
    }
    let Some(origin) = zones.layout().desks.iter().find(|desk| &desk.display == display).map(|desk| desk.rect.min) else {
        return;
    };
    for event in events {
        match event {
            DragEvent::Entered { at, offered } => {
                // The paths are known at once on Windows; a Wayland
                // source says them when it is let go. Until then: files
                // of ours if we are dragging, of someone else if not.
                let payload = match offered.payload {
                    Some(payload) => {
                        // (What is no file is named only in the system's own data.)
                        let mut named = Vec::new();
                        for native in &payload.native {
                            named.extend(zones.desk.items_named(&native.kind, &native.bytes));
                        }
                        zones.payload_with(named, &payload.paths, payload.text, to_zones(offered.allowed))
                    }
                    None => {
                        let ours = zones.payload_of(&[], None, to_zones(offered.allowed));
                        let dragging = zones.outgoing_items();
                        if dragging.is_empty() {
                            DropPayload { paths: vec![String::new()], ..ours }
                        } else {
                            DropPayload { items: dragging, ..ours }
                        }
                    }
                };
                let would = zones.drag_over(Some(at + origin.to_vec2()), &payload);
                log::debug!("a drag of {} came over the zones of {display}; there it would be {would:?}", payload.summary());
                OVER.with(|over| *over.borrow_mut() = Some(payload));
                control.answer_drag(own, answer_of(would));
            }
            DragEvent::Moved { at } => {
                let payload = OVER.with(|over| over.borrow().clone()).unwrap_or_default();
                let would = zones.drag_over(Some(at + origin.to_vec2()), &payload);
                control.answer_drag(own, answer_of(would));
            }
            DragEvent::Left => {
                log::debug!("the drag left the zones of {display}");
                OVER.with(|over| *over.borrow_mut() = None);
                zones.drag_over(None, &DropPayload::default());
            }
            DragEvent::Dropped { at, payload, action } => {
                OVER.with(|over| *over.borrow_mut() = None);
                // What was agreed is ours to do, a move in full: the
                // source deletes nothing.
                let allowed = match action {
                    DragAction::Move => DragActions { moving: true, ..DragActions::default() },
                    DragAction::Copy => DragActions { copy: true, ..DragActions::default() },
                    DragAction::Link => DragActions::default(),
                };
                // The backend reads the system's own description where
                // there is one: our items exactly, and what has no path.
                let payload = match zones.desk.dropped(control, own, allowed) {
                    Some(read) => zones.payload_with(read.items, &read.paths, read.text, allowed),
                    None => zones.payload_of(&payload.paths, payload.text, allowed),
                };
                let came = zones.drop_at(at + origin.to_vec2(), &payload);
                log::debug!("a drop of {} ({action:?}) on the zones of {display}: {came:?}", payload.summary());
            }
            DragEvent::Ended { outcome } => {
                log::debug!("the drag that began on the zones of {display} ended: {outcome:?}");
                zones.drag_ended(matches!(outcome, DragOutcome::Taken { remove: true, .. }));
            }
        }
    }
}

/// Draw every surface again (something changed from outside).
pub fn redraw(surfaces: &Rc<RefCell<Surfaces>>, control: &mut Control) {
    for (surface, _) in surfaces.borrow().displays.values() {
        control.redraw(*surface);
    }
}

/// The zones as a half of the shell holds them: started when they are
/// first switched on, and from then on told of every configuration and
/// every wake-up.
pub struct Running {
    pub zones: Shared,
    surfaces: Rc<RefCell<Surfaces>>,
    themed: Themed,
}

impl Running {
    /// The running configuration is this. Zones that are switched on are
    /// started if they are not (where this system has a backend), and
    /// their surfaces are made to fit; zones that are switched off close
    /// theirs. `wake` gets the shell to call [`Running::woken`].
    pub fn configured(
        running: &mut Option<Running>,
        config: &gevrek_zones::config::ZonesConfig,
        control: &mut Control,
        wake: impl Fn() + Send + Sync + Clone + 'static,
        themed: impl FnOnce() -> Themed,
        starter: impl FnOnce() -> super::Starter,
        hide_icons: bool,
    ) {
        if running.is_none() && config.enabled {
            let Some(desk) = super::desk(wake.clone(), starter()) else {
                log::warn!("desktop.zones.enabled is set, and this build cannot show zones yet");
                return;
            };
            let folder = gevrek_model::paths::state_dir().join("zones");
            let first = wake.clone();
            log::debug!("the zones are switched on: they start, with a surface for each display");
            let mut zones = Zones::start(config, desk, folder, wake, hide_icons);
            // (Looked at once at the start: what there is is told to
            // whoever listens.)
            first();
            // (On real surfaces a drag of files is the system's.)
            zones.system_drags = true;
            *running = Some(Running {
                zones: Rc::new(RefCell::new(zones)),
                surfaces: Rc::default(),
                themed: themed(),
            });
        }
        if let Some(running) = running {
            running.zones.borrow_mut().configure(config);
            running.zones.borrow_mut().set_hide_icons(hide_icons);
            sync(&running.zones, &running.surfaces, control, &running.themed);
            redraw(&running.surfaces, control);
        }
    }

    /// The shell was woken for the zones. Gives back the zones' part of
    /// the configuration when they changed it, to be written.
    pub fn woken(&self, control: &mut Control) -> Option<gevrek_zones::config::ZonesConfig> {
        let tick = self.zones.borrow_mut().tick();
        // (A display came or went: its surface with it.)
        sync(&self.zones, &self.surfaces, control, &self.themed);
        if tick.redraw {
            redraw(&self.surfaces, control);
        }
        tick.config
    }

    /// The window manager's state is this: the displays the zones are on
    /// are taken from it, and the workspace that is shown.
    pub fn told(&self, state: &serde_json::Value) {
        self.zones.borrow_mut().told(state, shown_workspace(state));
    }

    /// The compositor says which displays are shown to others now (the
    /// `screen.shared` event's fields): the zones cover what is not for
    /// other eyes while any is, as the configuration says.
    // (Only a compositor of ours tells; Windows gives no such sign.)
    #[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
    pub fn screen_shared(&self, fields: &[(String, String)]) {
        self.zones.borrow_mut().screen_shared(shared_now(fields));
    }

    /// Tell the window manager what happened to the zones and how they
    /// are now, for whoever asks it (`gevrek zones ...`, the event
    /// stream). Called when the shell was woken for the zones: a drag
    /// tells when it ends, not with every movement.
    pub fn publish(&self, link: &gevrek_link::Link) {
        let mut zones = self.zones.borrow_mut();
        let heard = zones.take_heard();
        if heard.is_empty() {
            return;
        }
        let told = gevrek_zones::timed::count(heard.len(), "event", "events");
        // (After each wake that heard something, a drag's too: said at
        // trace, and as a warning where it holds the thread.)
        gevrek_zones::timed::timed_quietly(
            || format!("telling the window manager of {told} and how the zones are"),
            || {
                for event in heard {
                    link.publish_zones_event(event);
                }
                let mut snapshot = gevrek_zones::ipc::ZonesSnapshot::of(zones.core(), super::now());
                snapshot.not_shown = zones.not_shown();
                link.publish_zones(snapshot);
            },
            |_| "told".to_string(),
        );
    }
}

/// Whether any display is shown to others, by the fields of the
/// compositor's `screen.shared` event: `shared` names the displays that
/// are (commas between; empty when none is).
pub fn shared_now(fields: &[(String, String)]) -> bool {
    fields.iter().any(|(name, value)| name == "shared" && !value.trim().is_empty())
}

/// The workspace that is shown on the display that has the keyboard (on
/// the first display, where none has): the one arrangements that differ
/// by workspace go by.
pub fn shown_workspace(state: &serde_json::Value) -> Option<String> {
    let views: Vec<crate::panel::View> = state["monitors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(crate::panel::view_of)
        .collect();
    let view = views.iter().find(|view| view.focused).or(views.first())?;
    view.tabs.iter().find(|tab| tab.shown).map(|tab| tab.name.clone())
}

/// Something was asked of the zones over the line to the window manager
/// (a key bound to a zones command, `gevrek zones ...`): carry it out and
/// answer, if an answer is waited for. Also without zones running: then
/// the answer says so.
pub fn asked(running: &Option<Running>, link: &gevrek_link::Link, ask: gevrek_link::ZonesAsk) {
    use gevrek_zones::ipc::{Answer, Request};
    log::debug!(
        "the zones are asked over the line to the window manager{}",
        ask.request.as_ref().map_or_else(|| ", and no answer is waited for".to_string(), |request| format!(" (request {request:?})"))
    );
    let answer = match (running, ask.ask) {
        (Some(running), Request::Command(command)) => {
            let effects = running.zones.borrow_mut().command(command);
            // (What followed is told and drawn when the shell looks next.)
            let zones = running.zones.borrow();
            (zones.wake_later)(std::time::Duration::ZERO);
            Answer::of(&effects)
        }
        (Some(running), Request::RestoreSystemIcons) => match running.zones.borrow_mut().restore_system_icons() {
            Ok(()) => Answer::done(),
            Err(why) => Answer::refused(why),
        },
        // Switched off, or never on: the system's icons can be put back
        // all the same, where this system has a backend at all.
        (None, Request::RestoreSystemIcons) => match super::desk(|| {}, super::Starter::none()) {
            Some(mut desk) => match super::explorer::put_back(&mut *desk, &Default::default()) {
                Ok(_) => Answer::done(),
                Err(why) => Answer::refused(why),
            },
            None => Answer::refused("this build cannot show zones yet"),
        },
        (None, Request::Command(_)) => Answer::refused("zones are switched off (desktop.zones.enabled)"),
    };
    if let Some(request) = ask.request {
        log::debug!("the zones answer request {request:?}: {}", answer.error.as_deref().map_or_else(|| "done".to_string(), |error| format!("not done: {error}")));
        link.answer_zones(request, answer);
    }
}

/// One line of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// Something to choose, perhaps marked as what is in force.
    Choice { label: String, marked: bool, does: Does },
    /// A line that leads to a menu of its own, which opens beside it.
    Sub { label: String, lines: Vec<Line> },
    /// A little room between two groups of lines.
    Gap,
}

/// What choosing a line of a menu does.
#[derive(Debug, Clone, PartialEq)]
pub enum Does {
    Command(Command),
    /// Ask for a box to type into, or a sheet to look something over on.
    Ask(Ask),
    /// A line the backend gave for some items.
    Act { items: Vec<gevrek_zones::ItemId>, action: String },
    /// Make a zone, with what it is configured as besides its kind.
    Make { title: String, kind: ZoneKind, rect: Option<gevrek_zones::Rect>, changes: Option<serde_json::Value> },
}

fn choice(label: &str, does: Does) -> Line {
    Line::Choice { label: label.to_string(), marked: false, does }
}

fn marked(label: &str, marked: bool, command: Command) -> Line {
    Line::Choice { label: label.to_string(), marked, does: Does::Command(command) }
}

fn sub(label: &str, lines: Vec<Line>) -> Line {
    Line::Sub { label: label.to_string(), lines }
}

/// Every line that can be chosen, of a menu and of the menus its lines
/// lead to, each with the way to it ("Look › Ground › Solid ground").
#[cfg_attr(not(test), allow(dead_code))]
pub fn every_line(lines: &[Line]) -> Vec<(String, &Line)> {
    let mut all = Vec::new();
    for line in lines {
        match line {
            Line::Choice { label, .. } => all.push((label.clone(), line)),
            Line::Sub { label, lines } => {
                all.extend(every_line(lines).into_iter().map(|(way, line)| (format!("{label} \u{203a} {way}"), line)));
            }
            Line::Gap => {}
        }
    }
    all
}

/// The menu of the bare desktop, pressed at a point of it on a display.
pub fn desk_menu(zones: &Zones, display: &DisplayId, at: Pos2, palette: &Palette) -> Vec<Line> {
    if zones.hidden() {
        return hidden_menu();
    }
    let here = gevrek_zones::Rect::new(at.x.round() as i32, at.y.round() as i32, 320, 240);
    // Every way to a new zone, under one line: an empty one here, one
    // that shows a folder, the ones that fill themselves, and one for
    // each type of what lies loose on the desktop.
    let mut new_zone = vec![
        choice(
            "Empty, here",
            Does::Command(Command::CreateZone { title: "New zone".to_string(), kind: ZoneKind::Manual, rect: Some(here) }),
        ),
        choice("Of a folder\u{2026}", Does::Ask(Ask::NewPortal { at })),
        sub("That fills itself", smart_menu(at)),
        Line::Gap,
        choice("Make zones by type\u{2026}", Does::Ask(Ask::AutoZones { display: display.clone() })),
    ];
    if zones.core().can_undo_auto() {
        new_zone.push(choice("Take the automatic zones back", Does::Command(Command::UndoAutoCreate)));
    }
    let mut lines = vec![sub("New zone", new_zone), Line::Gap, choice("Find an item\u{2026}", Does::Ask(Ask::Filter))];
    if let Some(parent) = zones.desktop_known() {
        lines.push(choice("New folder on the desktop\u{2026}", Does::Ask(Ask::NewFolderIn { parent })));
    }
    let mut rules = Vec::new();
    if !zones.core().config().rules.is_empty() || zones.core().config().catch_all.is_some() {
        rules.push(choice("What the rules would do\u{2026}", Does::Ask(Ask::Rules { scope: Scope::Loose })));
        rules.push(choice("Sort the desktop by the rules", Does::Command(Command::ApplyRules { scope: Scope::Loose })));
    }
    if zones.core().state().undo.is_some() {
        rules.push(choice("Take the sorting back", Does::Command(Command::UndoRules)));
    }
    if !rules.is_empty() {
        lines.push(sub("Rules", rules));
    }
    lines.push(choice("Take back the last change to a file", Does::Command(Command::UndoFileOperation)));
    // What takes long can be called off while it is under way.
    for operation in &zones.core().scene(super::now()).operations {
        lines.push(choice("Call off what is being copied or moved", Does::Command(Command::CancelOperation { request: operation.request })));
    }
    lines.push(Line::Gap);
    lines.push(choice("Hide the zones", Does::Command(Command::QuickHide { hidden: Some(true) })));
    let grey = zones.core().grey_icons();
    lines.push(marked("Every icon grey", grey, Command::GreyIcons { on: Some(!grey) }));
    // Everything covered by hand (for when the screen is shown to
    // others and the system does not say so), or shown again.
    let covered = zones.core().scene(super::now()).masked;
    lines.push(marked("Everything covered from view", covered, Command::Mask { masked: Some(!covered) }));
    lines.push(Line::Gap);
    let held = zones.core().config().lock;
    lines.push(sub(
        "Held",
        vec![
            marked("Every zone held where it is", held.geometry, Command::LockAll { geometry: Some(!held.geometry), members: None }),
            marked("Nothing in or out of any zone", held.members, Command::LockAll { geometry: None, members: Some(!held.members) }),
        ],
    ));
    lines.push(sub("Look of all zones", look_menu(zones, None, palette)));
    lines.push(sub("Arrangements", layouts_menu(zones)));
    lines
}

/// The menu of the arrangements: this one kept under a name, the ones
/// that were kept (each leads to what can be done with it), and an
/// arrangement written to a file or taken from one.
pub fn layouts_menu(zones: &Zones) -> Vec<Line> {
    let mut lines = vec![choice("Keep this arrangement as\u{2026}", Does::Ask(Ask::SaveLayout))];
    let kept = &zones.core().state().snapshots;
    if !kept.is_empty() {
        lines.push(Line::Gap);
    }
    // The ones the person kept first, then the ones the zones kept
    // themselves before something replaced the arrangement.
    for snapshot in kept.iter().filter(|snapshot| !snapshot.automatic).chain(kept.iter().filter(|snapshot| snapshot.automatic)) {
        lines.push(sub(&format!("\u{201c}{}\u{201d}", snapshot.name), layout_menu(&snapshot.name)));
    }
    lines.push(Line::Gap);
    lines.push(choice("Write this arrangement to a file\u{2026}", Does::Ask(Ask::ExportLayout { name: None })));
    lines.push(choice("Take an arrangement from a file\u{2026}", Does::Ask(Ask::ImportLayout)));
    lines
}

/// What can be done with one arrangement that was kept.
pub fn layout_menu(name: &str) -> Vec<Line> {
    let name = || name.to_string();
    vec![
        choice("Arrange the zones like this", Does::Command(Command::RestoreSnapshot { name: name() })),
        choice("Rename\u{2026}", Does::Ask(Ask::RenameLayout { name: name() })),
        choice("Write it to a file\u{2026}", Does::Ask(Ask::ExportLayout { name: Some(name()) })),
        Line::Gap,
        choice("Forget it", Does::Command(Command::DeleteSnapshot { name: name() })),
    ]
}

/// The zones that fill themselves, each made at a point with what it
/// looks through and how it orders what it finds. They are ordinary
/// zones afterwards: renamed, sized and given a look as any other, and
/// what they ask is in the configuration to be changed.
pub fn smart_menu(at: Pos2) -> Vec<Line> {
    use gevrek_zones::config::{DateField, Matcher};
    use serde_json::json;
    let here = |width: i32, height: i32| Some(gevrek_zones::Rect::new(at.x.round() as i32, at.y.round() as i32, width, height));
    let make = |label: &str, title: &str, query: Vec<Matcher>, rect: Option<gevrek_zones::Rect>, changes: serde_json::Value| {
        choice(
            label,
            Does::Make { title: title.to_string(), kind: ZoneKind::Smart { query }, rect, changes: Some(changes) },
        )
    };
    let today = Matcher::Date {
        field: DateField::Modified,
        after: None,
        before: None,
        newer_than_days: Some(1),
        older_than_days: None,
    };
    let pictures = Matcher::Extension {
        any_of: ["png", "jpg", "jpeg", "gif", "webp", "bmp"].map(str::to_string).to_vec(),
    };
    vec![
        make("The files used last", "Recent", Vec::new(), here(320, 240), json!({"source": {"from": "recent"}, "limit": 12, "view": "list"})),
        make(
            "What was downloaded today",
            "Downloaded today",
            vec![today],
            here(320, 240),
            json!({"source": {"from": "downloads"}, "sort": "date", "sort_descending": true, "view": "list"}),
        ),
        make("The programs that run", "Running", Vec::new(), here(360, 130), json!({"source": {"from": "running_apps"}})),
        make("The pinned programs", "Pinned", Vec::new(), here(360, 130), json!({"source": {"from": "pinned"}})),
        make("The pictures on the desktop", "Pictures", vec![pictures], here(320, 240), json!({"icon_size": 64})),
    ]
}

/// The menu of the desktop while everything is hidden: the way back, and
/// nothing else (a zone made now, or a sorting, would not be seen).
pub fn hidden_menu() -> Vec<Line> {
    vec![choice("Show the zones", Does::Command(Command::QuickHide { hidden: Some(false) }))]
}

/// The menu of a zone: its name, how it shows and orders what it holds,
/// what of it is held fast, and taking it away. What has several values
/// to pick from (how it is shown, how it looks, what is held) is a menu
/// of its own beside its line.
pub fn zone_menu(zones: &Zones, zone: &ZoneId, palette: &Palette) -> Vec<Line> {
    let Some(definition) = zones.core().config().zone(zone) else {
        return Vec::new();
    };
    let id = || zone.clone();
    let mut lines = vec![
        choice("Rename\u{2026}", Does::Ask(Ask::Rename { zone: id() })),
        marked("Rolled up", definition.rolled_up, Command::RollUp { zone: id(), rolled_up: None }),
        sub("Shown and ordered", view_menu(zones, zone)),
        sub("Look", look_menu(zones, Some(zone), palette)),
        Line::Gap,
    ];
    lines.push(sub(
        "Held",
        vec![
            marked(
                "Held where it is",
                definition.lock.geometry,
                Command::Lock { zone: id(), geometry: Some(!definition.lock.geometry), members: None },
            ),
            marked(
                "Nothing in or out",
                definition.lock.members,
                Command::Lock { zone: id(), geometry: None, members: Some(!definition.lock.members) },
            ),
        ],
    ));
    lines.push(marked(
        "Stays when the rest is hidden",
        definition.always_visible,
        Command::AlwaysVisible { zone: id(), on: Some(!definition.always_visible) },
    ));
    if definition.tab_group.is_some() {
        lines.push(choice("Out of its tabs", Does::Command(Command::Untab { zone: id() })));
    }
    lines.push(Line::Gap);
    // Covered until the person asks to see it; and, where it is covered
    // now or was asked to be seen, the other of that.
    lines.push(marked(
        "Private",
        definition.private,
        Command::ModifyZone { zone: id(), changes: serde_json::json!({ "private": !definition.private }) },
    ));
    let masked = zones.core().is_masked(zone);
    if masked {
        lines.push(choice("Show what it holds", Does::Command(Command::Reveal { zone: Some(id()), revealed: Some(true) })));
    } else if definition.private || zones.core().config().privacy.everything {
        lines.push(choice("Cover it again", Does::Command(Command::Reveal { zone: Some(id()), revealed: Some(false) })));
    }
    lines.push(marked(
        "Windows keep off its place",
        definition.reserves_space,
        Command::ModifyZone { zone: id(), changes: serde_json::json!({ "reserves_space": !definition.reserves_space }) },
    ));
    // What a zone that shows a folder, or looks through something, has
    // besides: a folder made in it, and asking again what is there.
    if !matches!(definition.kind, ZoneKind::Manual) {
        lines.push(Line::Gap);
    }
    if matches!(definition.kind, ZoneKind::Portal { .. }) {
        lines.push(choice("New folder in it\u{2026}", Does::Ask(Ask::NewFolder { zone: id() })));
    }
    if !matches!(definition.kind, ZoneKind::Manual) {
        lines.push(choice("Look again what is there", Does::Command(Command::RefreshZone { zone: id() })));
    }
    lines.push(Line::Gap);
    lines.push(choice("Take this zone away", Does::Command(Command::DeleteZone { zone: id() })));
    lines
}

/// The menu of how a zone shows what it holds: as a grid or a list, in
/// which order, how large.
pub fn view_menu(zones: &Zones, zone: &ZoneId) -> Vec<Line> {
    let Some(definition) = zones.core().config().zone(zone) else {
        return Vec::new();
    };
    let id = || zone.clone();
    let view = |view: View| Command::SetView { zone: id(), view: Some(view), sort: None, descending: None, icon_size: None };
    let sort = |sort: Sort| Command::SetView { zone: id(), view: None, sort: Some(sort), descending: None, icon_size: None };
    let size = |size: u32| Command::SetView { zone: id(), view: None, sort: None, descending: None, icon_size: Some(size) };
    let mut lines = vec![
        marked("As a grid", definition.view == View::Grid, view(View::Grid)),
        marked("As a list", definition.view == View::List, view(View::List)),
        Line::Gap,
    ];
    for (label, by) in [
        ("In my own order", Sort::Manual),
        ("By name", Sort::Name),
        ("By type", Sort::Type),
        ("By date", Sort::Date),
        ("By size", Sort::Size),
    ] {
        lines.push(marked(label, definition.sort == by, sort(by)));
    }
    lines.push(marked(
        "The other way round",
        definition.sort_descending,
        Command::SetView { zone: id(), view: None, sort: None, descending: Some(!definition.sort_descending), icon_size: None },
    ));
    lines.push(Line::Gap);
    for (label, side) in [("Small icons", 32), ("Medium icons", 48), ("Large icons", 72)] {
        lines.push(marked(label, definition.icon_size == side, size(side)));
    }
    lines
}

/// The menu of how a zone looks (or, without a zone, how all of them do
/// where they say nothing of their own): its title, its ground and its
/// icons, each a menu of its own. Every line lays a change over the
/// configuration; "as the others" takes a zone's own value away, so that
/// it follows the look of all zones, and that the theme. The colours
/// offered are the theme's own, so a zone that is given one still belongs
/// to the desktop it is on.
pub fn look_menu(zones: &Zones, zone: Option<&ZoneId>, palette: &Palette) -> Vec<Line> {
    use gevrek_zones::config::TitleBar;
    use serde_json::{json, Value};
    let config = zones.core().config();
    let look = match zone {
        Some(zone) => match config.zone(zone) {
            Some(definition) => &definition.look,
            None => return Vec::new(),
        },
        None => &config.look,
    };
    let change = |look: Value| match zone {
        Some(zone) => Command::ModifyZone { zone: zone.clone(), changes: json!({ "look": look }) },
        None => Command::ModifySettings { changes: json!({ "look": look }) },
    };
    let usual = if zone.is_some() { "as the others" } else { "as the theme" };
    let mut title = Vec::new();
    for (label, bar) in [("Title bar", TitleBar::Full), ("Title alone", TitleBar::Plain), ("No title", TitleBar::Hidden)] {
        let now = look.title_bar == Some(bar) || (look.title_bar.is_none() && zone.is_none() && bar == TitleBar::Full);
        title.push(marked(label, now, change(json!({ "title_bar": bar }))));
    }
    let mut ground = Vec::new();
    let soft = look.blur.unwrap_or(false);
    ground.push(marked("The desktop soft behind it", soft, change(json!({ "blur": !soft }))));
    ground.push(Line::Gap);
    for (label, opacity) in [("Light ground", Some(45)), ("Solid ground", Some(95))] {
        ground.push(marked(label, look.opacity == opacity, change(json!({ "opacity": opacity }))));
    }
    for (label, radius) in [("Square corners", Some(0)), ("Round corners", Some(18))] {
        ground.push(marked(label, look.corner_radius == radius, change(json!({ "corner_radius": radius }))));
    }
    ground.push(Line::Gap);
    // The theme's own hues, each laid thinly over its ground colour.
    for (label, hue) in [
        ("Ground in the accent colour", palette.accent),
        ("Blue ground", palette.blue),
        ("Purple ground", palette.purple),
        ("Green ground", palette.ok),
        ("Amber ground", palette.warn),
    ] {
        let colour = gevrek_ui::color::format(gevrek_ui::theme::mix(palette.surface, hue, 0.3));
        ground.push(marked(label, look.background.as_deref() == Some(colour.as_str()), change(json!({ "background": colour }))));
    }
    let mut icons = Vec::new();
    let labels = look.labels.unwrap_or(true);
    icons.push(marked("Names under the icons", labels, change(json!({ "labels": !labels }))));
    icons.push(Line::Gap);
    icons.push(marked("Grey icons", look.icon_saturation == Some(0) && look.icon_tint.is_none(), change(json!({ "icon_saturation": 0, "icon_tint": null }))));
    let [red, green, blue, _] = palette.accent.to_srgba_unmultiplied();
    let tint = format!("#{red:02x}{green:02x}{blue:02x}66");
    icons.push(marked(
        "Icons in the accent colour",
        look.icon_tint.as_deref() == Some(tint.as_str()),
        change(json!({ "icon_saturation": 0, "icon_tint": tint })),
    ));
    icons.push(marked(
        "Icons in their own colours",
        look.icon_saturation.is_none() && look.icon_tint.is_none(),
        change(json!({ "icon_saturation": null, "icon_tint": null })),
    ));
    vec![
        sub("Title", title),
        sub("Ground", ground),
        sub("Icons", icons),
        Line::Gap,
        choice(
            &format!("Everything {usual}"),
            Does::Command(change(json!({
                "title_bar": null, "labels": null, "blur": null, "opacity": null, "corner_radius": null,
                "background": null, "icon_saturation": null, "icon_tint": null, "title_colour": null, "title_on_hover": null,
            }))),
        ),
    ]
}

/// The lines the backend gave for some items, as a menu. The line that
/// leads to every program the items can be opened with has those
/// programs beside it, where there are any.
fn item_menu(items: &[gevrek_zones::ItemId], lines: &[DeskAction], programs: &[DeskAction]) -> Vec<Line> {
    let act = |line: &DeskAction| choice(&line.label, Does::Act { items: items.to_vec(), action: line.id.clone() });
    lines
        .iter()
        .map(|line| {
            if line.id == super::CHOOSE && !programs.is_empty() {
                sub(line.label.trim_end_matches('\u{2026}'), programs.iter().map(act).collect())
            } else {
                act(line)
            }
        })
        .collect()
}

/// Draw one menu: its lines, the one the pointer or the keys are on, and
/// the one whose own menu is open beside it. Says the line the pointer
/// is on, and the line that was pressed.
///
/// A screen reader is told a menu and its items (marked or not, leading
/// to a menu or not), and pressing an item for the person is a press.
pub fn menu(ui: &mut egui::Ui, palette: &Palette, shown: &menu::Level) -> (Option<usize>, Option<usize>) {
    use egui::accesskit::{Action, HasPopup, Role, Toggled};
    let all = ui.max_rect();
    ui.painter().rect_filled(all, 0.0, palette.surface);
    // (Menus lie over each other by a little: each has its own rim.)
    ui.painter().rect_stroke(all, 0.0, egui::Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| node.set_role(Role::Menu));
    let rows = menu::rows(&shown.lines);
    // First where the pointer is, then the drawing: the line it is on is
    // the one that is marked, also before the keys' line has caught up.
    let mut pointed = None;
    let mut pressed = None;
    for (index, (line, row)) in shown.lines.iter().zip(&rows).enumerate() {
        let (label, is_marked, leads_on) = match line {
            Line::Choice { label, marked, .. } => (label, *marked, false),
            Line::Sub { label, .. } => (label, false, true),
            Line::Gap => continue,
        };
        let row = row.translate(all.min.to_vec2());
        let response = ui.interact(row, ui.unique_id().with(("menu-line", index)), egui::Sense::click());
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_role(if is_marked { Role::MenuItemCheckBox } else { Role::MenuItem });
            node.set_label(label.trim_end_matches('\u{2026}').to_string());
            if is_marked {
                node.set_toggled(Toggled::True);
            }
            if leads_on {
                node.set_has_popup(HasPopup::Menu);
                node.set_expanded(shown.open == Some(index));
            }
            node.add_action(Action::Click);
        });
        if response.hovered() {
            pointed = Some(index);
        }
        if response.clicked() {
            pressed = Some(index);
        }
    }
    let at = pointed.or(shown.at);
    for (index, (line, row)) in shown.lines.iter().zip(&rows).enumerate() {
        let row = row.translate(all.min.to_vec2());
        let (label, is_marked, leads_on) = match line {
            Line::Choice { label, marked, .. } => (label, *marked, false),
            Line::Sub { label, .. } => (label, false, true),
            Line::Gap => {
                ui.painter().hline((row.left() + 6.0)..=(row.right() - 6.0), row.center().y, egui::Stroke::new(1.0, palette.border.gamma_multiply(0.6)));
                continue;
            }
        };
        if at == Some(index) {
            ui.painter().rect_filled(row, 6.0, palette.control_hover());
        } else if shown.open == Some(index) {
            // The line the menu beside this one opened from.
            ui.painter().rect_filled(row, 6.0, palette.control_hover().gamma_multiply(0.6));
        }
        if is_marked {
            // The same short mark a zone's title has.
            let mark = Rect::from_center_size(pos2(row.left() + 9.0, row.center().y), vec2(3.0, 12.0));
            ui.painter().rect_filled(mark, 1.5, palette.accent);
        }
        // (A name longer than its line is cut, with an ellipsis.)
        let room = row.width() - 20.0 - if leads_on { 22.0 } else { 8.0 };
        let mut job = egui::text::LayoutJob::simple_singleline(label.clone(), egui::FontId::proportional(13.0), palette.text);
        job.wrap = egui::text::TextWrapping { max_width: room, max_rows: 1, break_anywhere: true, ..Default::default() };
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(row.left() + 20.0, row.center().y - galley.size().y / 2.0), galley, palette.text);
        if leads_on {
            // An arrow at its end: there is a menu beside this line.
            let tip = pos2(row.right() - 11.0, row.center().y);
            let stroke = egui::Stroke::new(1.3, if at == Some(index) || shown.open == Some(index) { palette.text } else { palette.muted });
            ui.painter().line_segment([tip + vec2(-3.0, -4.0), tip + vec2(1.0, 0.0)], stroke);
            ui.painter().line_segment([tip + vec2(1.0, 0.0), tip + vec2(-3.0, 4.0)], stroke);
        }
    }
    (pointed, pressed)
}

/// Do what a line of a menu does.
fn carry_out(zones: &Shared, does: Does) {
    let mut zones = zones.borrow_mut();
    match does {
        Does::Command(command) => {
            zones.command(command);
        }
        Does::Ask(ask) => zones.ask(ask),
        Does::Act { items, action } => zones.act(&items, &action),
        Does::Make { title, kind, rect, changes } => {
            zones.make_zone(&title, kind, rect, changes);
        }
    }
    // (What follows is seen when the shell next looks at the zones.)
    (zones.wake_later)(std::time::Duration::ZERO);
}

/// Close the surfaces of menus, the last opened first.
fn close_menus(open: &Rc<RefCell<OpenMenus>>, control: &mut Control) {
    let mut open = open.borrow_mut();
    open.chain.close_from(0);
    for (_, surface) in open.surfaces.drain(..).rev() {
        if control.is_open(surface) {
            control.close(surface);
        }
    }
}

/// Make the open surfaces fit how the menus stand: the surface of a menu
/// that is no longer open is closed, a menu that opened gets its surface
/// beside the line it opened from, and each that stays is drawn again
/// (the line the keys are on may be another). A menu the host closed
/// itself (Escape where the host hears it, a press elsewhere) is no
/// longer open, nor is any that opened from it.
fn fit(open: &Rc<RefCell<OpenMenus>>, zones: &Shared, themed: &Themed, control: &mut Control) {
    let mut now = open.borrow_mut();
    if let Some(gone) = now.surfaces.iter().position(|(_, surface)| !control.is_open(*surface)) {
        now.chain.close_from(gone);
    }
    // The surfaces that still show the menu that is open at their depth
    // stay; from the first that does not, they go.
    let kept = now
        .surfaces
        .iter()
        .zip(&now.chain.levels)
        .take_while(|((serial, _), level)| *serial == level.serial)
        .count();
    while now.surfaces.len() > kept {
        if let Some((_, surface)) = now.surfaces.pop() {
            if control.is_open(surface) {
                control.close(surface);
            }
        }
    }
    while now.surfaces.len() < now.chain.levels.len() {
        let depth = now.surfaces.len();
        // (The first menu is opened where it was asked for: `answer`.)
        let (Some(parent), Some(from)) = (depth.checked_sub(1).map(|before| now.surfaces[before].1), depth.checked_sub(1)) else {
            now.chain.close_from(0);
            break;
        };
        let Some(line) = now.chain.levels[from].open else {
            now.chain.close_from(depth);
            break;
        };
        let anchor = menu::rows(&now.chain.levels[from].lines)[line];
        let level = &now.chain.levels[depth];
        let serial = level.serial;
        let surface = control.submenu(parent, anchor, menu_size(&level.lines), menu_surface(open, zones, themed, serial));
        now.surfaces.push((serial, surface));
    }
    let stands: Vec<(u64, Option<usize>, Option<usize>)> = now.chain.levels.iter().map(|level| (level.serial, level.at, level.open)).collect();
    if stands != now.drawn {
        for (_, surface) in &now.surfaces {
            control.redraw(*surface);
        }
        now.drawn = stands;
    }
}

/// What draws the surface of one menu, and hears the pointer and (where
/// a menu has the keyboard, as on Wayland) the keys on it.
fn menu_surface(
    open: &Rc<RefCell<OpenMenus>>,
    zones: &Shared,
    themed: &Themed,
    serial: u64,
) -> impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static {
    let open = Rc::clone(open);
    let zones = Rc::clone(zones);
    let themed = Rc::clone(themed);
    let mut fonts = false;
    move |ui, control| {
        let (palette, _) = themed(ui, &mut fonts);
        let chosen = {
            let mut now = open.borrow_mut();
            if now.chain.level_of(serial).is_none() {
                return Flow::Close;
            }
            let mut chosen = None;
            for key in ui.input_mut(menu::keys) {
                chosen = chosen.or(now.chain.key(key));
            }
            // (A key may have closed this very menu: then nothing is drawn.)
            if let (None, Some(depth)) = (&chosen, now.chain.level_of(serial)) {
                let seconds = ui.input(|input| input.time);
                let (pointed, pressed) = menu(ui, &palette, &now.chain.levels[depth]);
                now.chain.pointed(depth, pointed, seconds);
                if let Some(line) = pressed {
                    chosen = now.chain.pressed(depth, line);
                }
                now.chain.rested(seconds);
                if let Some(wait) = now.chain.rests_in(seconds) {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.01)));
                }
            }
            chosen
        };
        if let Some(does) = chosen {
            // A line that does something closes every menu.
            close_menus(&open, control);
            carry_out(&zones, does);
            return Flow::Close;
        }
        fit(&open, &zones, &themed, control);
        if open.borrow().chain.level_of(serial).is_some() {
            Flow::Continue
        } else {
            Flow::Close
        }
    }
}

/// The keys a display's surface heard while a menu is open are the
/// menu's. Says whether a menu is still open afterwards.
fn menu_keys(zones: &Shared, open: &Rc<RefCell<OpenMenus>>, themed: &Themed, keys: Vec<menu::Key>, control: &mut Control) -> bool {
    let mut chosen = None;
    {
        let mut now = open.borrow_mut();
        // (The host may have closed some since: Escape, where it hears it.)
        if let Some(gone) = now.surfaces.iter().position(|(_, surface)| !control.is_open(*surface)) {
            now.chain.close_from(gone);
        }
        for key in keys {
            chosen = chosen.or(now.chain.key(key));
        }
    }
    if let Some(does) = chosen {
        close_menus(open, control);
        carry_out(zones, does);
        return false;
    }
    fit(open, zones, themed, control);
    !open.borrow().chain.levels.is_empty()
}

/// Open what the pointer asked for: a menu beside where it was pressed,
/// or a box to type into.
#[allow(clippy::too_many_arguments)]
fn answer(
    zones: &Shared,
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    palette: &Palette,
    ask: Ask,
    display: &DisplayId,
    parent: SurfaceId,
    control: &mut Control,
) {
    let origin = {
        let mut zones = zones.borrow_mut();
        zones.layout().desks.iter().find(|desk| &desk.display == display).map(|desk| desk.rect.min)
    };
    let Some(origin) = origin else {
        return;
    };
    let (lines, at) = match ask {
        Ask::DeskMenu { ref display, at } => (desk_menu(&zones.borrow(), display, at, palette), at),
        Ask::ZoneMenu { ref zone, at } => (zone_menu(&zones.borrow(), zone, palette), at),
        Ask::ViewMenu { ref zone, at } => (view_menu(&zones.borrow(), zone), at),
        Ask::Rules { scope } => {
            rules(zones, surfaces, themed, scope, display, control);
            return;
        }
        Ask::AutoZones { display: ref on } => {
            auto_zones(zones, surfaces, themed, on, control);
            return;
        }
        Ask::NewFolder { zone } => {
            typing(zones, surfaces, themed, Typed::FolderName(zone), display, control);
            return;
        }
        Ask::NewPortal { at } => {
            choose_folder(zones, surfaces, themed, at, display, control);
            return;
        }
        Ask::NewFolderIn { parent } => {
            typing(zones, surfaces, themed, Typed::FolderIn(parent), display, control);
            return;
        }
        Ask::Sheet { heading, lines } => {
            properties(surfaces, themed, heading, lines, display, &zones.borrow_mut().spots(), control);
            return;
        }
        Ask::SaveLayout => {
            typing(zones, surfaces, themed, Typed::LayoutName, display, control);
            return;
        }
        Ask::RenameLayout { name } => {
            typing(zones, surfaces, themed, Typed::LayoutRename(name), display, control);
            return;
        }
        Ask::ExportLayout { name } => {
            typing(zones, surfaces, themed, Typed::ExportPath(name), display, control);
            return;
        }
        Ask::ImportLayout => {
            typing(zones, surfaces, themed, Typed::ImportPath, display, control);
            return;
        }
        Ask::ItemMenu { ref items, at, ref lines } => {
            // Every program they can be opened with stands beside the
            // line that leads to them (asked of the system only where
            // there is such a line).
            let programs = if lines.iter().any(|line| line.id == super::CHOOSE) {
                zones.borrow_mut().programs_for(items)
            } else {
                Vec::new()
            };
            (item_menu(items, lines, &programs), at)
        }
        Ask::Rename { zone } => {
            typing(zones, surfaces, themed, Typed::ZoneName(zone), display, control);
            return;
        }
        Ask::RenameItem { item } => {
            typing(zones, surfaces, themed, Typed::ItemName(item), display, control);
            return;
        }
        Ask::Filter => {
            typing(zones, surfaces, themed, Typed::Find, display, control);
            return;
        }
    };
    // Asked for by a key: the keys are on its first line already.
    let by_keys = zones.borrow_mut().take_menu_by_keys();
    if lines.is_empty() {
        return;
    }
    // One menu at a time: the one that is open goes as this one comes,
    // with every menu that opened from it.
    if let Some(open) = surfaces.borrow_mut().menu.take() {
        close_menus(&open, control);
    }
    let anchor = Rect::from_min_size(at - origin.to_vec2(), vec2(1.0, 1.0));
    let size = menu_size(&lines);
    let open = Rc::new(RefCell::new(OpenMenus { chain: Chain::of(lines, by_keys), surfaces: Vec::new(), drawn: Vec::new() }));
    let serial = open.borrow().chain.levels[0].serial;
    let opened = control.popup(parent, anchor, size, menu_surface(&open, zones, themed, serial));
    open.borrow_mut().surfaces.push((serial, opened));
    surfaces.borrow_mut().menu = Some(open);
}

/// How large the sheet of the zones by type is, in points.
const AUTO_SHEET: (u32, u32) = (640, 520);

/// What was chosen on the sheet of the zones by type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoChoice {
    /// Look at the loose items only, or at what lies in zones filled by
    /// hand as well.
    Scope(Scope),
    /// Add the rules for what comes later, or not.
    Rules(bool),
    /// Make what is shown.
    Make,
    /// Take the zones made last back.
    Undo,
    Close,
}

/// Open the sheet that shows which zones would be made for the types of
/// what lies on the desktop, before anything is made: each zone with
/// what would go into it, and a small picture of the display with where
/// they would stand. From it they are made, and what was made is taken
/// back. One at a time; put away as a menu is, by a press beside it or
/// Escape.
fn auto_zones(
    zones: &Shared,
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    display: &DisplayId,
    control: &mut Control,
) {
    if let Some(open) = surfaces.borrow_mut().sheet.take() {
        if control.is_open(open) {
            control.close(open);
        }
    }
    let spot = zones.borrow_mut().spots().into_iter().find(|(known, _)| known == display).map(|(_, spot)| spot);
    let place = Place {
        name: "gevrek-zone-auto".to_string(),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: AUTO_SHEET.0,
        height: AUTO_SHEET.1,
        reserve: false,
        // (It is read and pressed; the keyboard comes with a press on it.)
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: spot.map(|spot| spot.output),
    };
    let shared = Rc::clone(zones);
    let all = Rc::clone(surfaces);
    let look = Rc::clone(themed);
    let on = display.clone();
    let mut fonts = false;
    let mut rules = zones.borrow().core().config().auto.add_rules;
    let mut preview = zones.borrow_mut().auto_preview(Scope::Loose, &on);
    // What was done from this sheet, to say so on it.
    let mut done: Option<&'static str> = None;
    let surface = control.open(place, move |ui, _| {
        let (palette, _) = look(ui, &mut fonts);
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let choice = auto_sheet(ui, &palette, &preview, rules, done);
        let mut zones = shared.borrow_mut();
        match choice {
            _ if escaped => {}
            Some(AutoChoice::Scope(scope)) => {
                preview = zones.auto_preview(scope, &on);
                done = None;
            }
            Some(AutoChoice::Rules(add)) => rules = add,
            Some(AutoChoice::Make) => {
                zones.command(Command::AutoCreate {
                    scope: preview.scope,
                    min_items: None,
                    display: Some(on.clone()),
                    add_rules: Some(rules),
                    dry_run: false,
                });
                preview = zones.auto_preview(preview.scope, &on);
                done = Some("Made. It can be taken back.");
            }
            Some(AutoChoice::Undo) => {
                zones.command(Command::UndoAutoCreate);
                preview = zones.auto_preview(preview.scope, &on);
                done = Some("Taken back.");
            }
            Some(AutoChoice::Close) | None => {}
        }
        if choice.is_some() {
            (zones.wake_later)(std::time::Duration::ZERO);
        }
        if escaped || choice == Some(AutoChoice::Close) {
            all.borrow_mut().sheet = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    surfaces.borrow_mut().sheet = Some(surface);
}

/// Draw the sheet of the zones that would be made by type; what was
/// chosen on it. `rules`: whether the rules for what comes later would
/// be added. `done`: what was just done from it, said above the buttons.
pub fn auto_sheet(
    ui: &mut egui::Ui,
    palette: &Palette,
    preview: &super::AutoPreview,
    rules: bool,
    done: Option<&str>,
) -> Option<AutoChoice> {
    let mut choice = None;
    let plan = &preview.plan;
    let new = plan.zones.iter().filter(|zone| zone.zone.is_none()).count();
    let moved: usize = plan.zones.iter().map(|zone| zone.items.len()).sum();
    egui::Frame::new().fill(palette.surface).corner_radius(12).inner_margin(18).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.label(egui::RichText::new("Zones by type").size(16.0).color(palette.text));
        ui.label(egui::RichText::new("A zone for each type of what lies on the desktop. Nothing is changed until you say so.").size(12.0).color(palette.muted));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (label, scope) in [("Loose items only", Scope::Loose), ("Also what lies in zones filled by hand", Scope::All)] {
                if ui.selectable_label(preview.scope == scope, label).clicked() && preview.scope != scope {
                    choice = Some(AutoChoice::Scope(scope));
                }
            }
        });
        ui.add_space(6.0);
        // In a few words, before the picture and the list.
        let count = |number: usize, one: &str, many: &str| match number {
            1 => format!("1 {one}"),
            number => format!("{number} {many}"),
        };
        let mut sum = Vec::new();
        if new > 0 {
            sum.push(count(new, "zone would be made", "zones would be made"));
        }
        if moved > 0 {
            sum.push(count(moved, "item would go into its zone", "items would go into their zones"));
        }
        match plan.left.len() {
            0 => {}
            1 => sum.push("1 stays where it is".to_string()),
            left => sum.push(format!("{left} stay where they are")),
        }
        let sum = if plan.zones.is_empty() { "Nothing to do: every type that has enough items has its zone.".to_string() } else { sum.join(" \u{00b7} ") };
        ui.label(egui::RichText::new(sum).size(12.5).color(palette.text));
        ui.add_space(8.0);
        let below = 112.0;
        let high = (ui.available_height() - below).max(80.0);
        ui.horizontal_top(|ui| {
            // The display, small, with the zones there are and where the
            // new ones would stand.
            let wide = 250.0;
            let (map, _) = ui.allocate_exact_size(vec2(wide, high), egui::Sense::hover());
            let whole = preview.display;
            let scale = (wide / whole.width().max(1.0)).min(high / whole.height().max(1.0));
            let small = Rect::from_min_size(map.min, whole.size() * scale);
            let to_map = |rect: Rect| Rect::from_min_size(small.min + (rect.min - whole.min) * scale, rect.size() * scale).intersect(small);
            ui.painter().rect_filled(small, 6.0, palette.control);
            ui.painter().rect_stroke(small, 6.0, egui::Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
            for there in &preview.there {
                ui.painter().rect_filled(to_map(*there).shrink(0.5), 2.0, palette.text.gamma_multiply(0.14));
            }
            for (number, zone) in plan.zones.iter().enumerate() {
                let rect = zone.rect;
                let place = to_map(Rect::from_min_size(pos2(rect.x as f32, rect.y as f32), vec2(rect.width as f32, rect.height as f32))).shrink(0.5);
                let ink = if zone.zone.is_none() { palette.accent } else { palette.muted };
                ui.painter().rect_filled(place, 2.0, ink.gamma_multiply(0.22));
                ui.painter().rect_stroke(place, 2.0, egui::Stroke::new(1.0, ink), egui::StrokeKind::Inside);
                if place.width() >= 12.0 && place.height() >= 12.0 {
                    ui.painter().text(place.center(), egui::Align2::CENTER_CENTER, (number + 1).to_string(), egui::FontId::proportional(10.5), palette.text);
                }
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().max_height(high).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (number, zone) in plan.zones.iter().enumerate() {
                        let what = if zone.zone.is_none() { "new" } else { "is there, takes more" };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(egui::RichText::new(format!("{}", number + 1)).size(11.5).color(palette.faint));
                            ui.label(egui::RichText::new(&zone.title).size(13.0).color(palette.text));
                            ui.label(egui::RichText::new(format!("{} \u{00b7} {what}", count(zone.items.len(), "item", "items"))).size(11.5).color(palette.muted));
                        });
                        // A few of what would go into it, by name.
                        let names: Vec<&str> = zone.items.iter().take(4).filter_map(|item| preview.names.get(item).map(String::as_str)).collect();
                        let more = zone.items.len().saturating_sub(names.len());
                        let mut said = names.join(", ");
                        if more > 0 {
                            said.push_str(&format!(" and {more} more"));
                        }
                        ui.add(egui::Label::new(egui::RichText::new(said).size(11.5).color(palette.faint)).truncate());
                        ui.add_space(4.0);
                    }
                });
            });
        });
        ui.add_space(6.0);
        let mut add = rules;
        if ui.checkbox(&mut add, "Sort new items of these types too (a rule for each zone)").changed() {
            choice = Some(AutoChoice::Rules(add));
        }
        if let Some(done) = done {
            ui.label(egui::RichText::new(done).size(12.0).color(palette.ok));
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(!plan.zones.is_empty(), egui::Button::new("Make them")).clicked() {
                    choice = Some(AutoChoice::Make);
                }
                if ui.add_enabled(preview.can_undo, egui::Button::new("Take the automatic zones back")).clicked() {
                    choice = Some(AutoChoice::Undo);
                }
                if ui.button("Close").clicked() {
                    choice = Some(AutoChoice::Close);
                }
            });
        });
    });
    choice
}

/// How large the box to type into is, in points.
const BOX: (u32, u32) = (360, 96);

/// What a box is typed into for.
#[derive(Debug, Clone, PartialEq)]
enum Typed {
    ZoneName(ZoneId),
    ItemName(gevrek_zones::ItemId),
    Find,
    /// The name of a folder to be made in the folder a portal stands in.
    FolderName(ZoneId),
    /// The name of a folder to be made in a folder that is named.
    FolderIn(String),
    /// The name this arrangement is kept under.
    LayoutName,
    /// A kept arrangement's new name.
    LayoutRename(String),
    /// The file an arrangement is written to.
    ExportPath(Option<String>),
    /// The file an arrangement is taken from.
    ImportPath,
}

/// The file an arrangement is offered to be written to: beside the
/// desktop's own folder (not in it, where it would lie among the icons),
/// called after the arrangement.
pub fn export_path(desktop: Option<&str>, name: Option<&str>) -> String {
    let called: String = name
        .unwrap_or("now")
        .chars()
        .map(|letter| if letter.is_alphanumeric() || letter == '-' { letter.to_ascii_lowercase() } else { '-' })
        .collect();
    let file = format!("gevrek-zones-{}.json", called.trim_matches('-'));
    match desktop {
        Some(folder) => {
            let folder = folder.trim_end_matches(['/', '\\']);
            let cut = folder.rfind(['/', '\\']).map_or(0, |at| at + 1);
            format!("{}{file}", &folder[..cut])
        }
        None => file,
    }
}

/// The name a zone gets that shows a folder: the folder's own.
pub fn portal_title(directory: &str) -> String {
    let name = directory.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default();
    if name.is_empty() { directory.to_string() } else { name.to_string() }
}

/// Open the box a zone's or a file's name is typed into, or the one for
/// what is looked for. It has the keyboard until Enter, Escape, or a
/// press elsewhere: it closes itself when the keyboard goes to another
/// window, and a press on a display's surface closes it ([`dismiss`]),
/// which is the only one of the two that happens where the box holds the
/// keyboard for itself alone.
fn typing(
    zones: &Shared,
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    what: Typed,
    display: &DisplayId,
    control: &mut Control,
) {
    if let Some(open) = surfaces.borrow_mut().typing.take() {
        if control.is_open(open) {
            control.close(open);
        }
    }
    let (mut text, heading, title_at) = {
        let mut zones = zones.borrow_mut();
        zones.renaming = match &what {
            Typed::ZoneName(zone) => Some(zone.clone()),
            _ => None,
        };
        match &what {
            Typed::ZoneName(zone) => {
                let title = zones.core().config().zone(zone).map(|zone| zone.title.clone()).unwrap_or_default();
                let head = zones.layout().zone(zone).map(|shown| shown.head);
                (title.clone(), format!("Rename the zone \u{201c}{title}\u{201d}"), head)
            }
            Typed::ItemName(item) => {
                let name = zones.core().item(item).map(|item| item.name.clone()).unwrap_or_default();
                let at = zones.layout().item(item).map(|shown| shown.rect);
                (name.clone(), format!("Rename \u{201c}{name}\u{201d}"), at)
            }
            Typed::Find => (zones.filter.clone(), "Find an item by its name".to_string(), None),
            Typed::FolderName(zone) => {
                let head = zones.layout().zone(zone).map(|shown| shown.head);
                ("New folder".to_string(), "The name of the new folder".to_string(), head)
            }
            Typed::FolderIn(_) => ("New folder".to_string(), "The name of the new folder".to_string(), None),
            Typed::LayoutName => (String::new(), "Keep this arrangement under the name".to_string(), None),
            Typed::LayoutRename(name) => (name.clone(), format!("Rename the arrangement \u{201c}{name}\u{201d}"), None),
            Typed::ExportPath(name) => (
                export_path(zones.desktop_known().as_deref(), name.as_deref()),
                "Write the arrangement to the file".to_string(),
                None,
            ),
            Typed::ImportPath => (
                export_path(zones.desktop_known().as_deref(), None),
                "Take an arrangement from the file".to_string(),
                None,
            ),
        }
    };
    let spot = zones.borrow_mut().spots().into_iter().find(|(known, _)| known == display).map(|(_, spot)| spot);
    let place = Place {
        // (Not a name that begins as the displays' surfaces' does: those
        // are the desktop to the window manager, and this is not.)
        name: "gevrek-zone-box".to_string(),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: BOX.0,
        height: BOX.1,
        reserve: false,
        keyboard: Keyboard::Exclusive,
        margin: 0,
        output: spot.as_ref().map(|spot| spot.output.clone()),
    };
    let shared = Rc::clone(zones);
    let all = Rc::clone(surfaces);
    let look = Rc::clone(themed);
    let mut fonts = false;
    let mut had_keyboard = false;
    let draw = move |ui: &mut egui::Ui, _: &mut Control| {
        let (palette, _) = look(ui, &mut fonts);
        let context = ui.ctx().clone();
        let mut close = context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let before = text.clone();
        let entered = type_box(ui, &palette, &heading, &mut text);
        let mut zones = shared.borrow_mut();
        match &what {
            Typed::ZoneName(zone) => {
                if entered {
                    if !text.trim().is_empty() {
                        zones.command(Command::RenameZone { zone: zone.clone(), title: text.trim().to_string() });
                    }
                    close = true;
                }
            }
            Typed::ItemName(item) => {
                if entered {
                    if !text.trim().is_empty() {
                        zones.command(Command::RenameItem { item: item.clone(), name: text.trim().to_string() });
                    }
                    close = true;
                }
            }
            Typed::FolderName(zone) => {
                if entered {
                    if !text.trim().is_empty() {
                        zones.command(Command::NewFolder { zone: zone.clone(), name: text.trim().to_string() });
                    }
                    close = true;
                }
            }
            Typed::LayoutName => {
                if entered {
                    if !text.trim().is_empty() {
                        zones.command(Command::SaveSnapshot { name: text.trim().to_string() });
                    }
                    close = true;
                }
            }
            Typed::LayoutRename(name) => {
                if entered {
                    if !text.trim().is_empty() && text.trim() != name {
                        zones.command(Command::RenameSnapshot { name: name.clone(), to: text.trim().to_string() });
                    }
                    close = true;
                }
            }
            Typed::ExportPath(name) => {
                if entered {
                    // What came of it is said: a file is not seen to be written.
                    match zones.export_to(name.clone(), text.trim()) {
                        Ok(()) => zones.say("The arrangement was written", vec![("File".to_string(), text.trim().to_string())]),
                        Err(why) => zones.say("The arrangement was not written", vec![("Why".to_string(), why)]),
                    }
                    close = true;
                }
            }
            Typed::ImportPath => {
                if entered {
                    match zones.import_from(text.trim()) {
                        Ok(name) => zones.say(
                            "The arrangement was taken in",
                            vec![
                                ("Kept as".to_string(), name),
                                ("Before".to_string(), "The arrangement there was is kept too, under Arrangements".to_string()),
                            ],
                        ),
                        Err(why) => zones.say("The arrangement was not taken in", vec![("Why".to_string(), why)]),
                    }
                    close = true;
                }
            }
            Typed::FolderIn(parent) => {
                if entered {
                    if !text.trim().is_empty() {
                        zones.new_folder_in(parent, text.trim());
                    }
                    close = true;
                }
            }
            Typed::Find => {
                // What is looked for is seen as it is typed.
                if text != before {
                    zones.filter = text.clone();
                    zones.redraw = true;
                    (zones.wake_later)(std::time::Duration::ZERO);
                }
                if close {
                    zones.filter.clear();
                    zones.redraw = true;
                    (zones.wake_later)(std::time::Duration::ZERO);
                }
                close |= entered;
            }
        }
        let has = context.input(|input| input.focused);
        had_keyboard |= has;
        if close || (had_keyboard && !has) {
            zones.renaming = None;
            all.borrow_mut().typing = None;
            (zones.wake_later)(std::time::Duration::ZERO);
            return Flow::Close;
        }
        Flow::Continue
    };
    // On Windows the box can be put where the title is; a Wayland
    // compositor places a surface itself, and it is in the middle.
    #[cfg(windows)]
    let surface = match (title_at, &spot) {
        (Some(head), Some(spot)) => {
            let origin = shared_origin(zones, display);
            let scale = {
                let mut zones = zones.borrow_mut();
                let wide = zones.layout().desks.iter().find(|desk| &desk.display == display).map_or(1.0, |desk| desk.rect.width());
                spot.pixels.2 as f32 / wide.max(1.0)
            };
            let at = (head.min - origin.to_vec2()) * scale;
            control.open_at(
                place,
                gevrek_shellkit::win::place::Area::new(
                    spot.pixels.0 + at.x as i32,
                    spot.pixels.1 + at.y as i32,
                    (BOX.0 as f32 * scale) as i32,
                    (BOX.1 as f32 * scale) as i32,
                ),
                draw,
            )
        }
        _ => control.open(place, draw),
    };
    #[cfg(not(windows))]
    let surface = {
        let _ = title_at;
        control.open(place, draw)
    };
    surfaces.borrow_mut().typing = Some(surface);
}

/// Open a sheet with what there is to say of some items: lines of a
/// name and a value. Put away as the sheet of the rules is.
fn properties(
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    heading: String,
    lines: Vec<(String, String)>,
    display: &DisplayId,
    spots: &[(DisplayId, Spot)],
    control: &mut Control,
) {
    if let Some(open) = surfaces.borrow_mut().sheet.take() {
        if control.is_open(open) {
            control.close(open);
        }
    }
    let place = Place {
        name: "gevrek-zone-properties".to_string(),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: 460,
        height: (96 + 24 * lines.len() as u32).min(560),
        reserve: false,
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: spots.iter().find(|(known, _)| known == display).map(|(_, spot)| spot.output.clone()),
    };
    let all = Rc::clone(surfaces);
    let look = Rc::clone(themed);
    let mut fonts = false;
    let surface = control.open(place, move |ui, _| {
        let (palette, _) = look(ui, &mut fonts);
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if properties_sheet(ui, &palette, &heading, &lines) || escaped {
            all.borrow_mut().sheet = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    surfaces.borrow_mut().sheet = Some(surface);
}

/// Draw what there is to say of some items; whether it was closed.
pub fn properties_sheet(ui: &mut egui::Ui, palette: &Palette, heading: &str, lines: &[(String, String)]) -> bool {
    let mut close = false;
    egui::Frame::new().fill(palette.surface).corner_radius(12).inner_margin(18).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.label(egui::RichText::new(heading).size(16.0).color(palette.text));
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(40.0)).auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("zone-item-properties").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
                for (name, value) in lines {
                    ui.label(egui::RichText::new(name).size(12.5).color(palette.muted));
                    ui.add(egui::Label::new(egui::RichText::new(value).size(12.5).color(palette.text)).wrap());
                    ui.end_row();
                }
            });
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            close = ui.button("Close").clicked();
        });
    });
    close
}

/// How large the folder chooser is, in points.
const CHOOSER: (u32, u32) = (820, 560);

/// Open the chooser for the folder a new zone is to show (the design
/// system's own: the same one the settings use), beginning beside the
/// desktop's own folder. In it a folder is gone to by pressing, or by
/// typing its path in place of the steps. Chosen, a zone that shows the
/// folder is made where the menu was asked for.
fn choose_folder(
    zones: &Shared,
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    at: Pos2,
    display: &DisplayId,
    control: &mut Control,
) {
    if let Some(open) = surfaces.borrow_mut().sheet.take() {
        if control.is_open(open) {
            control.close(open);
        }
    }
    let spot = zones.borrow_mut().spots().into_iter().find(|(known, _)| known == display).map(|(_, spot)| spot);
    let place = Place {
        name: "gevrek-zone-folder".to_string(),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: CHOOSER.0,
        height: CHOOSER.1,
        reserve: false,
        // (It is pressed, and typed into where the person presses to type.)
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: spot.map(|spot| spot.output),
    };
    let begin = zones.borrow().desktop_known().unwrap_or_default();
    let home = gevrek_ui::browser::home();
    let mut browser =
        gevrek_ui::browser::Browser::new(gevrek_ui::widgets::PathKind::Folder, &begin, home.as_deref(), "", &[]);
    let shared = Rc::clone(zones);
    let all = Rc::clone(surfaces);
    let look = Rc::clone(themed);
    let mut fonts = false;
    let surface = control.open(place, move |ui, _| {
        let _ = look(ui, &mut fonts);
        let Some(done) = browser.show(ui.ctx()) else {
            return Flow::Continue;
        };
        if let gevrek_ui::browser::Done::Chosen(folder) = done {
            let mut zones = shared.borrow_mut();
            let (directory, rect) = portal_at(&folder.to_string_lossy(), at);
            zones.make_zone(&portal_title(&directory), ZoneKind::Portal { directory }, Some(rect), None);
            (zones.wake_later)(std::time::Duration::ZERO);
        }
        all.borrow_mut().sheet = None;
        Flow::Close
    });
    surfaces.borrow_mut().sheet = Some(surface);
}

/// The folder a new zone shows, as it is kept, and where the zone is
/// made: at the point the menu was asked for.
pub fn portal_at(folder: &str, at: Pos2) -> (String, gevrek_zones::Rect) {
    (folder.to_string(), gevrek_zones::Rect::new(at.x.round() as i32, at.y.round() as i32, 360, 260))
}

/// How large the sheet of the rules is, in points.
const SHEET: (u32, u32) = (600, 460);

/// What was chosen on the sheet of the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulesChoice {
    /// Look at the loose items only, or at every item.
    Scope(Scope),
    /// Do what is shown.
    Apply,
    /// Take the last sorting back.
    Undo,
    Close,
}

/// Open the sheet that shows what the rules would do, before anything is
/// done: every item with where it would go and by which rule, and what
/// is held back and why. From it the rules are applied, and what was
/// applied is taken back. One at a time; put away as a menu is, by a
/// press beside it or Escape.
fn rules(
    zones: &Shared,
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
    scope: Scope,
    display: &DisplayId,
    control: &mut Control,
) {
    if let Some(open) = surfaces.borrow_mut().sheet.take() {
        if control.is_open(open) {
            control.close(open);
        }
    }
    let spot = zones.borrow_mut().spots().into_iter().find(|(known, _)| known == display).map(|(_, spot)| spot);
    let place = Place {
        name: "gevrek-zone-rules".to_string(),
        edge: Edge::Centre,
        level: Level::Overlay,
        width: SHEET.0,
        height: SHEET.1,
        reserve: false,
        // (It is read and pressed; the keyboard comes with a press on it.)
        keyboard: Keyboard::OnDemand,
        margin: 0,
        output: spot.map(|spot| spot.output),
    };
    let shared = Rc::clone(zones);
    let all = Rc::clone(surfaces);
    let look = Rc::clone(themed);
    let mut fonts = false;
    let mut preview = zones.borrow().rules_preview(scope);
    // What was done from this sheet, to say so on it.
    let mut done: Option<&'static str> = None;
    let surface = control.open(place, move |ui, _| {
        let (palette, _) = look(ui, &mut fonts);
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let choice = rules_sheet(ui, &palette, &preview, done);
        let mut zones = shared.borrow_mut();
        match choice {
            _ if escaped => {}
            Some(RulesChoice::Scope(scope)) => {
                preview = zones.rules_preview(scope);
                done = None;
            }
            Some(RulesChoice::Apply) => {
                zones.command(Command::ApplyRules { scope: preview.scope });
                preview = zones.rules_preview(preview.scope);
                done = Some("Sorted. It can be taken back.");
            }
            Some(RulesChoice::Undo) => {
                zones.command(Command::UndoRules);
                preview = zones.rules_preview(preview.scope);
                done = Some("Taken back.");
            }
            Some(RulesChoice::Close) | None => {}
        }
        if choice.is_some() {
            (zones.wake_later)(std::time::Duration::ZERO);
        }
        if escaped || choice == Some(RulesChoice::Close) {
            all.borrow_mut().sheet = None;
            return Flow::Close;
        }
        Flow::Continue
    });
    surfaces.borrow_mut().sheet = Some(surface);
}

/// Draw the sheet of what the rules would do; what was chosen on it.
/// `done`: what was just done from it, said above the buttons.
pub fn rules_sheet(
    ui: &mut egui::Ui,
    palette: &Palette,
    preview: &super::RulesPreview,
    done: Option<&str>,
) -> Option<RulesChoice> {
    use gevrek_zones::rules::{Decided, Destination, Reason};
    let mut choice = None;
    let moved = preview.lines.iter().filter(|line| line.held.is_none());
    let (into_zones, files): (Vec<_>, Vec<_>) = moved.partition(|line| matches!(line.destination, Destination::Zone { .. }));
    let held = preview.lines.iter().filter(|line| line.held.is_some()).count();
    egui::Frame::new().fill(palette.surface).corner_radius(12).inner_margin(18).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.label(egui::RichText::new("What the rules would do").size(16.0).color(palette.text));
        ui.label(egui::RichText::new("Nothing is changed until you say so.").size(12.0).color(palette.muted));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (label, scope) in [("Loose items only", Scope::Loose), ("Every item on the desktop", Scope::All)] {
                if ui.selectable_label(preview.scope == scope, label).clicked() && preview.scope != scope {
                    choice = Some(RulesChoice::Scope(scope));
                }
            }
        });
        ui.add_space(6.0);
        // In a few words, before the list.
        let count = |number: usize, one: &str, many: &str| match number {
            1 => format!("1 {one}"),
            number => format!("{number} {many}"),
        };
        let mut sum = Vec::new();
        if !into_zones.is_empty() {
            sum.push(format!("{} into zones", count(into_zones.len(), "item would go", "items would go")));
        }
        if !files.is_empty() {
            sum.push(format!("{} to another folder", count(files.len(), "file would be moved", "files would be moved")));
        }
        match held {
            0 => {}
            1 => sum.push("1 stays where it is".to_string()),
            held => sum.push(format!("{held} stay where they are")),
        }
        let sum = if sum.is_empty() { "No rule fits anything here.".to_string() } else { sum.join(" \u{00b7} ") };
        ui.label(egui::RichText::new(sum).size(12.5).color(palette.text));
        ui.add_space(6.0);
        let below = 86.0 + if preview.timed_every.is_some() { 34.0 } else { 0.0 };
        egui::ScrollArea::vertical().max_height((ui.available_height() - below).max(60.0)).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for line in &preview.lines {
                let stays = line.held.is_some();
                let ink = if stays { palette.muted } else { palette.text };
                let rule = match (&line.decided, line.rule.is_empty()) {
                    (Decided::CatchAll, _) => "what no rule takes".to_string(),
                    (Decided::Rule(index), true) => format!("rule {}", index + 1),
                    (Decided::Rule(_), false) => format!("\u{201c}{}\u{201d}", line.rule),
                };
                let goes = match (&line.held, &line.destination) {
                    (Some(Reason::NoSuchZone), _) => "stays: its rule names a zone that is not there".to_string(),
                    (Some(Reason::ZoneFillsItself), _) => "stays: that zone fills itself".to_string(),
                    (Some(Reason::Locked), _) => "stays: a zone lets nothing in or out".to_string(),
                    (Some(Reason::NotAFile), _) => "stays: it is no file that could be moved".to_string(),
                    (None, Destination::Zone { zone, title }) => {
                        format!("into {}", if title.is_empty() { zone.as_str() } else { title.as_str() })
                    }
                    (None, Destination::Folder { folder }) => format!("the file is moved to {folder}"),
                };
                let moves_file = !stays && matches!(line.destination, Destination::Folder { .. });
                // Three columns, each begun at its left: what, where to, by which rule.
                let cell = |ui: &mut egui::Ui, wide: f32, text: String, colour: egui::Color32, size: f32| {
                    ui.allocate_ui_with_layout(egui::vec2(wide, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(wide);
                        ui.add(egui::Label::new(egui::RichText::new(text).size(size).color(colour)).truncate());
                    });
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 0.0);
                    cell(ui, 168.0, line.name.clone(), ink, 12.5);
                    cell(ui, 236.0, goes, if moves_file { palette.warn } else { ink }, 12.5);
                    cell(ui, 120.0, format!("by {rule}"), palette.faint, 11.5);
                });
            }
        });
        ui.add_space(6.0);
        if let Some(every) = preview.timed_every {
            let minutes = (every / 60).max(1);
            let files = if preview.timed_move_files { "also the ones that move files" } else { "never the ones that move files" };
            ui.label(
                egui::RichText::new(format!(
                    "Rules that ask how old a file is come true by themselves: they are tried every {minutes} minutes ({files})."
                ))
                .size(11.5)
                .color(palette.muted),
            );
            ui.add_space(4.0);
        }
        if let Some(done) = done {
            ui.label(egui::RichText::new(done).size(12.0).color(palette.ok));
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                let anything = !into_zones.is_empty() || !files.is_empty();
                if ui.add_enabled(anything, egui::Button::new("Sort now")).clicked() {
                    choice = Some(RulesChoice::Apply);
                }
                if ui.add_enabled(preview.can_undo, egui::Button::new("Take the last sorting back")).clicked() {
                    choice = Some(RulesChoice::Undo);
                }
                if ui.button("Close").clicked() {
                    choice = Some(RulesChoice::Close);
                }
            });
        });
    });
    choice
}

/// Where a display begins on the desktop.
#[cfg(windows)]
fn shared_origin(zones: &Shared, display: &DisplayId) -> Pos2 {
    let mut zones = zones.borrow_mut();
    zones
        .layout()
        .desks
        .iter()
        .find(|desk| &desk.display == display)
        .map_or(Pos2::ZERO, |desk| desk.rect.min)
}

/// A heading and one line to type into. Says whether Enter was pressed.
fn type_box(ui: &mut egui::Ui, palette: &Palette, heading: &str, text: &mut String) -> bool {
    let mut entered = false;
    egui::Frame::new().fill(palette.surface).corner_radius(12).inner_margin(16).show(ui, |ui| {
        ui.label(egui::RichText::new(heading).size(14.0).color(palette.text));
        ui.add_space(8.0);
        let field = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    });
    entered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zones::draw::tests::a_working_desktop;

    #[test]
    fn a_zones_menu_marks_what_is_in_force_and_every_line_is_a_command() {
        let zones = a_working_desktop("menu");
        let downloads = zones.core().config().zones[1].id.clone();
        let lines = zone_menu(&zones, &downloads, &palette());
        // (In the menu itself and in the one of what is held, beside it.)
        let marked: Vec<String> = every_line(&lines)
            .into_iter()
            .filter(|(way, line)| matches!(line, Line::Choice { marked: true, .. }) && !way.starts_with("Shown") && !way.starts_with("Look"))
            .map(|(way, _)| way)
            .collect();
        assert_eq!(marked, Vec::<String>::new(), "nothing of it is held, and it is not rolled up");
        // What has several values to pick from is a menu beside its line.
        let leads_on: Vec<&str> = lines
            .iter()
            .filter_map(|line| match line {
                Line::Sub { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(leads_on, ["Shown and ordered", "Look", "Held"]);
        let Some(Line::Sub { lines: shown_beside, .. }) = lines.iter().find(|line| matches!(line, Line::Sub { label, .. } if label == "Shown and ordered")) else {
            panic!("the line leads to a menu");
        };
        assert_eq!(shown_beside, &view_menu(&zones, &downloads));
        // How it shows what it holds is a menu of its own.
        let shown: Vec<String> = view_menu(&zones, &downloads)
            .into_iter()
            .filter_map(|line| match line {
                Line::Choice { label, marked: true, does: Does::Command(Command::SetView { .. }) } => Some(label),
                _ => None,
            })
            .collect();
        assert_eq!(shown, ["As a list", "By name", "Medium icons"]);
        assert!(every_line(&lines).iter().all(|(_, line)| match line {
            Line::Choice { does, .. } => matches!(does, Does::Command(_) | Does::Ask(Ask::Rename { .. } | Ask::NewFolder { .. })),
            Line::Sub { .. } | Line::Gap => true,
        }));
        // It shows a folder: a folder can be made in it, and it can be
        // asked again what is there; a zone filled by hand has neither.
        let labels = |zone: &ZoneId| -> Vec<String> {
            every_line(&zone_menu(&zones, zone, &palette())).into_iter().map(|(way, _)| way).collect()
        };
        let papers = zones.core().config().zones[0].id.clone();
        assert!(labels(&downloads).contains(&"New folder in it\u{2026}".to_string()));
        assert!(!labels(&papers).iter().any(|label| label.starts_with("New folder") || label.starts_with("Look again")));
        // A zone that is a tab can be taken out of its tabs; one that is not, not.
        let has = |zone: &ZoneId, label: &str| {
            zone_menu(&zones, zone, &palette()).iter().any(|line| matches!(line, Line::Choice { label: has, .. } if has == label))
        };
        let work = zones.core().config().zones[5].id.clone();
        assert!(has(&work, "Out of its tabs") && !has(&downloads, "Out of its tabs"));
        assert_eq!(zone_menu(&zones, &ZoneId::new("no-such"), &palette()), []);
        let size = menu_size(&lines);
        assert!(size.x == menu::MENU_WIDTH && size.y > 200.0 && size.y < 440.0, "{size:?}");
    }

    #[test]
    fn the_keyboard_comes_with_a_press_and_is_never_asked_for() {
        let spot = Spot { output: "DP-1".to_string(), pixels: (0, 0, 1920, 1080) };
        let asked = place(&spot);
        // Not `Exclusive` (which takes the keyboard when the surface is
        // shown) and below the windows: shown, it takes nothing from them.
        assert_eq!(asked.keyboard, Keyboard::OnDemand);
        assert_eq!((asked.level, asked.edge), (Level::Below, Edge::Fill));
        // Nothing that draws the zones or reads the pointer on them asks
        // for the keyboard: no part of it names a call that would.
        let asks_for_focus = ["request_focus", "Keyboard::Exclusive", "memory_mut", "focus("];
        for (file, text) in [
            ("draw.rs", include_str!("draw.rs")),
            ("view.rs", include_str!("view.rs")),
            ("gesture.rs", include_str!("gesture.rs")),
            ("mod.rs", include_str!("mod.rs")),
            ("motion.rs", include_str!("motion.rs")),
        ] {
            for call in asks_for_focus {
                assert!(!text.contains(call), "{file} has {call}");
            }
        }
        // What a screen reader is told (`access.rs`) names the node that
        // has the keyboard among the nodes of a surface that has it
        // already: once, behind that condition. It asks the system for
        // nothing: no surface's keyboard and no window's place is named.
        let told = include_str!("access.rs");
        assert_eq!(told.matches("request_focus(").count(), 1);
        let after_the_condition = told.split("if let (true, Some(id)) = (has_keyboard, with_keyboard)").nth(1).expect("the condition is there");
        assert!(after_the_condition.contains("request_focus(id)"));
        for call in ["Keyboard::", "ViewportCommand", "set_keyboard", "set_level"] {
            assert!(!told.contains(call), "access.rs has {call}");
        }
        // Here, only the box that is typed into does: a surface of its own.
        let here = include_str!("surfaces.rs");
        let before_the_tests = here.split("#[cfg(test)]").next().unwrap();
        assert_eq!(before_the_tests.matches(".request_focus()").count(), 1);
        assert_eq!(before_the_tests.matches("keyboard: Keyboard::Exclusive").count(), 1);
        let typed_into = before_the_tests.split("fn type_box").nth(1).unwrap();
        assert!(typed_into.contains(".request_focus()"));
        // The one exception is a command, not the drawing: while the
        // zones are looked at above the windows (`Command::Peek`), their
        // surfaces have the keyboard for themselves, so that what is
        // typed is looked for; and they give it back when that ends.
        assert_eq!(stands(false), (Level::Below, Keyboard::OnDemand));
        assert_eq!(stands(true), (Level::Overlay, Keyboard::Exclusive));
        // Nothing else names the keyboard for a surface that is open:
        // `stands` is the only place that says `Exclusive` beside the
        // box, and what it says is told to the host in `sync` alone,
        // which goes by what the core says of peeking and by nothing
        // else (not the pointer, not a frame).
        assert_eq!(before_the_tests.matches("Keyboard::Exclusive").count(), 2);
        assert_eq!(before_the_tests.matches("set_keyboard(").count(), 2);
        let sync_alone = before_the_tests.split("pub fn sync").nth(1).unwrap().split("\nfn pressed_elsewhere").next().unwrap();
        assert_eq!(sync_alone.matches("set_keyboard(").count(), 2);
        assert_eq!(sync_alone.matches("stands(").count(), 2);
        assert!(sync_alone.contains("zones.core().peeking()"));
    }

    #[test]
    fn looked_at_the_surfaces_go_above_the_windows_with_the_keyboard_and_back() {
        let (zones, surfaces, look, mut control, ..) = beside_a_display("above");
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!surfaces.borrow().above);
        // A key (or another program) asks to look at the zones.
        zones.borrow_mut().command(Command::Peek { on: true });
        sync(&zones, &surfaces, &mut control, &look);
        assert!(surfaces.borrow().above);
        // What is typed then is looked for everywhere...
        zones.borrow_mut().typed("bud");
        let found = zones.borrow().search.clone().expect("something is looked for");
        assert_eq!(found.typed, "bud");
        assert_eq!(found.hits[0].name, "Budget.xlsx");
        // ...and Escape first empties what was typed, then ends the look.
        zones.borrow_mut().key(crate::zones::gesture::Key::Escape);
        assert!(zones.borrow().core().peeking() && zones.borrow().search.as_ref().unwrap().typed.is_empty());
        zones.borrow_mut().key(crate::zones::gesture::Key::Escape);
        assert!(!zones.borrow().core().peeking() && zones.borrow().search.is_none());
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!surfaces.borrow().above);
        // Opening what was found ends it as well: the core says so.
        zones.borrow_mut().command(Command::Peek { on: true });
        zones.borrow_mut().typed("budget");
        zones.borrow_mut().key(crate::zones::gesture::Key::Enter);
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!surfaces.borrow().above && !zones.borrow().core().peeking());
        // Switched off while looked at: not above any more.
        zones.borrow_mut().command(Command::Peek { on: true });
        sync(&zones, &surfaces, &mut control, &look);
        assert!(surfaces.borrow().above);
        let mut off = zones.borrow().core().config().clone();
        off.enabled = false;
        zones.borrow_mut().configure(&off);
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!surfaces.borrow().above);
    }

    #[test]
    fn the_room_kept_for_zones_is_one_strip_a_side_as_deep_as_the_deepest() {
        use gevrek_zones::geometry::Edge as At;
        use gevrek_zones::reserve::ReservedRegion;
        let region = |display: &str, edge: At, thickness: i32| ReservedRegion {
            display: DisplayId::new(display),
            rect: gevrek_zones::Rect::new(0, 0, 100, thickness.max(1)),
            edge,
            thickness,
            zones: vec![ZoneId::new("z")],
        };
        let rooms = rooms_for(&[
            region("a", At::Top, 120),
            region("a", At::Top, 80),
            region("a", At::Left, 300),
            region("b", At::Bottom, 64),
            region("b", At::Right, 0),
        ]);
        let a = DisplayId::new("a");
        let b = DisplayId::new("b");
        assert_eq!(
            rooms,
            BTreeMap::from([((a.clone(), Side::Top), 120), ((a, Side::Left), 300), ((b, Side::Bottom), 64)])
        );
        assert!(rooms_for(&[]).is_empty());
    }

    #[test]
    fn a_zone_that_keeps_windows_off_has_room_kept_and_what_is_private_is_kept_from_pictures() {
        let (zones, surfaces, look, mut control, ..) = beside_a_display("rooms");
        sync(&zones, &surfaces, &mut control, &look);
        assert!(surfaces.borrow().rooms.is_empty() && !surfaces.borrow().unpictured);
        // A zone at the display's top asks windows to keep off it.
        let papers = zones.borrow().core().config().zones[0].id.clone();
        let asked = Command::ModifyZone { zone: papers.clone(), changes: serde_json::json!({ "reserves_space": true }) };
        assert!(gevrek_zones::ipc::Answer::of(&zones.borrow_mut().command(asked)).ok);
        let wanted = rooms_for(&zones.borrow().core().reserved_regions());
        assert_eq!(wanted.len(), 1, "{wanted:?}");
        sync(&zones, &surfaces, &mut control, &look);
        // A strip is opened for it where a surface can keep room (a
        // layer surface); on Windows the room goes to the window manager
        // with the snapshot, and no surface is opened.
        let kept: Vec<((DisplayId, Side), u32)> =
            surfaces.borrow().rooms.iter().map(|(at, (_, depth))| (at.clone(), *depth)).collect();
        if cfg!(windows) {
            assert!(kept.is_empty());
            let snapshot = gevrek_zones::ipc::ZonesSnapshot::of(zones.borrow().core(), 0);
            assert_eq!(snapshot.reserved.len(), 1);
        } else {
            assert_eq!(kept, wanted.into_iter().collect::<Vec<_>>());
            let strip = surfaces.borrow().rooms.values().next().unwrap().0;
            assert!(control.is_open(strip));
            // No longer asked for: the strip goes.
            let asked = Command::ModifyZone { zone: papers.clone(), changes: serde_json::json!({ "reserves_space": false }) };
            zones.borrow_mut().command(asked);
            sync(&zones, &surfaces, &mut control, &look);
            assert!(surfaces.borrow().rooms.is_empty() && !control.is_open(strip));
        }
        // A private zone: the surfaces are left out of pictures of the
        // screen for as long as there is one, also while it is shown.
        let private = Command::ModifyZone { zone: papers.clone(), changes: serde_json::json!({ "private": true }) };
        zones.borrow_mut().command(private);
        sync(&zones, &surfaces, &mut control, &look);
        assert!(surfaces.borrow().unpictured);
        zones.borrow_mut().command(Command::Reveal { zone: Some(papers.clone()), revealed: Some(true) });
        sync(&zones, &surfaces, &mut control, &look);
        assert!(surfaces.borrow().unpictured, "seen by the person, and still in no picture");
        // The configuration says nothing is to be done about sharing: let in again.
        zones.borrow_mut().command(Command::ModifySettings { changes: serde_json::json!({"privacy": {"when_shared": "nothing"}}) });
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!surfaces.borrow().unpictured);
    }

    #[test]
    fn a_shared_screen_covers_what_is_private_until_the_person_asks_again() {
        let (zones, ..) = beside_a_display("shared");
        let papers = zones.borrow().core().config().zones[0].id.clone();
        let private = Command::ModifyZone { zone: papers.clone(), changes: serde_json::json!({ "private": true }) };
        zones.borrow_mut().command(private);
        let covered = |zones: &Shared| zones.borrow_mut().layout().zone(&papers).unwrap().masked;
        let items = |zones: &Shared| zones.borrow_mut().layout().zone(&papers).unwrap().items.len();
        // Covered until the person asks: nothing of what it holds is
        // there to be drawn, only how many there are.
        assert!(covered(&zones) && items(&zones) == 0);
        assert_eq!(zones.borrow_mut().layout().zone(&papers).unwrap().item_count, 8);
        // A click on the cover asks to see it.
        use crate::zones::gesture::Pointer;
        let at = pos2(400.0, 200.0);
        zones.borrow_mut().pointer(&Pointer { at: Some(at), pressed: true, down: true, ..Pointer::default() });
        zones.borrow_mut().pointer(&Pointer { at: Some(at), released: true, ..Pointer::default() });
        assert!(!covered(&zones) && items(&zones) == 8);
        // The compositor says a display is shown to others: covered again.
        let said = |shared: &str| vec![("state".to_string(), "started".to_string()), ("shared".to_string(), shared.to_string())];
        assert!(shared_now(&said("DP-1")) && shared_now(&said("DP-1,HDMI-A-1")) && !shared_now(&said("")) && !shared_now(&[]));
        let running = Running { zones: Rc::clone(&zones), surfaces: Rc::default(), themed: Rc::new(|_: &egui::Ui, _: &mut bool| unreachable!()) };
        running.screen_shared(&said("DP-1"));
        assert!(covered(&zones) && items(&zones) == 0);
        // No longer shared: it stays covered until the person asks.
        running.screen_shared(&said(""));
        assert!(covered(&zones));
        // Everything covered by hand, and shown again.
        zones.borrow_mut().command(Command::Mask { masked: Some(true) });
        assert!(zones.borrow_mut().layout().desks[0].masked);
        assert!(zones.borrow_mut().layout().zones.iter().all(|zone| zone.masked));
        zones.borrow_mut().command(Command::Mask { masked: Some(false) });
        assert!(!zones.borrow_mut().layout().desks[0].masked);
    }

    #[test]
    fn the_look_of_a_zone_is_changed_by_laying_changes_over_its_configuration() {
        let mut zones = a_working_desktop("look-menu");
        let papers = zones.core().config().zones[0].id.clone();
        let choose = |zones: &mut Zones, zone: Option<&ZoneId>, label: &str| {
            // (Its title, its ground and its icons are menus beside their lines.)
            let lines = look_menu(zones, zone, &palette());
            let all = every_line(&lines);
            let line = all.iter().map(|(_, line)| *line).find(|line| matches!(line, Line::Choice { label: has, .. } if has == label));
            let Some(Line::Choice { does: Does::Command(command), .. }) = line else {
                panic!("no line {label:?}");
            };
            let effects = zones.command(command.clone());
            assert!(gevrek_zones::ipc::Answer::of(&effects).ok, "{label}: {effects:?}");
        };
        let is_marked = |zones: &Zones, zone: Option<&ZoneId>, label: &str| {
            every_line(&look_menu(zones, zone, &palette()))
                .iter()
                .any(|(_, line)| matches!(line, Line::Choice { label: has, marked: true, .. } if has == label))
        };
        // Nothing of its own at first: it looks as all zones do.
        assert!(!is_marked(&zones, Some(&papers), "Title alone"));
        assert!(is_marked(&zones, None, "Title bar") && is_marked(&zones, Some(&papers), "Names under the icons"));
        for label in ["Title alone", "Names under the icons", "The desktop soft behind it", "Solid ground", "Round corners", "Blue ground", "Grey icons"] {
            choose(&mut zones, Some(&papers), label);
        }
        let look = zones.core().config().zone(&papers).unwrap().look.clone();
        assert_eq!(look.title_bar, Some(gevrek_zones::config::TitleBar::Plain));
        assert_eq!((look.labels, look.blur, look.opacity, look.corner_radius), (Some(false), Some(true), Some(95), Some(18)));
        assert_eq!(look.icon_saturation, Some(0));
        assert!(look.background.as_deref().is_some_and(|colour| colour.len() == 7 && colour.starts_with('#')));
        assert!(is_marked(&zones, Some(&papers), "Title alone") && is_marked(&zones, Some(&papers), "Blue ground"));
        // The configuration that comes of it is one that may be written.
        assert_eq!(zones.core().config().problems(), []);
        // Back to how the others look: nothing of its own is left.
        choose(&mut zones, Some(&papers), "Everything as the others");
        assert_eq!(zones.core().config().zone(&papers).unwrap().look, gevrek_zones::config::ZoneLook::default());
        // For all zones at once.
        choose(&mut zones, None, "No title");
        assert_eq!(zones.core().config().look.title_bar, Some(gevrek_zones::config::TitleBar::Hidden));
        assert_eq!(zones.layout().zones[0].title_bar, gevrek_zones::config::TitleBar::Hidden);
        assert_eq!(look_menu(&zones, Some(&ZoneId::new("no-such")), &palette()), []);
    }

    /// The colours a menu of the look is made with.
    fn palette() -> Palette {
        gevrek_ui::Settings::default().resolve(egui::Theme::Dark, egui::Color32::from_rgb(64, 224, 208))
    }

    /// What `answer` and `sync` need, without a display: the zones of a
    /// working day, no surface yet, a look that is never asked for (no
    /// frame is drawn), and a host that only writes down what is asked
    /// of it. With the surface of the one display, as if it were open.
    fn beside_a_display(name: &str) -> (Shared, Rc<RefCell<Surfaces>>, Themed, Control, DisplayId, SurfaceId) {
        let zones: Shared = Rc::new(RefCell::new(a_working_desktop(name)));
        let look: Themed = Rc::new(|_: &egui::Ui, _: &mut bool| -> (Palette, gevrek_ui::Settings) {
            unreachable!("no frame is drawn")
        });
        let mut control = Control::default();
        let spot = Spot { output: "picture-1".to_string(), pixels: (0, 0, 1280, 800) };
        let parent = control.open(place(&spot), |_, _| Flow::Continue);
        (zones, Rc::default(), look, control, DisplayId::new("picture-1"), parent)
    }

    #[test]
    fn there_is_one_menu_at_a_time_and_a_press_beside_it_puts_it_away() {
        let (zones, surfaces, look, mut control, display, parent) = beside_a_display("one-menu");
        let papers = zones.borrow().core().config().zones[0].id.clone();
        let menu_now = |surfaces: &Rc<RefCell<Surfaces>>| surfaces.borrow().menu();
        let bare = Ask::DeskMenu { display: display.clone(), at: pos2(1100.0, 700.0) };
        // The other button on the bare desktop, and again without choosing:
        // the first menu goes as the second comes.
        answer(&zones, &surfaces, &look, &palette(), bare.clone(), &display, parent, &mut control);
        let first = menu_now(&surfaces).expect("a menu was opened");
        assert!(control.is_open(first));
        answer(&zones, &surfaces, &look, &palette(), bare.clone(), &display, parent, &mut control);
        let second = menu_now(&surfaces).expect("a menu was opened");
        assert!(first != second && !control.is_open(first) && control.is_open(second));
        // A zone's menu is the same one menu.
        answer(&zones, &surfaces, &look, &palette(), Ask::ZoneMenu { zone: papers.clone(), at: pos2(300.0, 150.0) }, &display, parent, &mut control);
        let third = menu_now(&surfaces).expect("a menu was opened");
        assert!(!control.is_open(second) && control.is_open(third));
        // A press on a display's surface puts it away; the next finds none.
        dismiss(&zones, &surfaces, &mut control);
        assert!(!control.is_open(third) && menu_now(&surfaces).is_none());
        dismiss(&zones, &surfaces, &mut control);
        // The displays' surface itself is never what is closed.
        assert!(control.is_open(parent));
        // A menu that closed itself (a line was chosen) is not closed again,
        // and is no obstacle to the next.
        answer(&zones, &surfaces, &look, &palette(), bare.clone(), &display, parent, &mut control);
        let chosen = menu_now(&surfaces).unwrap();
        control.close(chosen);
        answer(&zones, &surfaces, &look, &palette(), bare, &display, parent, &mut control);
        assert!(control.is_open(menu_now(&surfaces).unwrap()));
    }

    #[test]
    fn a_press_beside_the_box_that_is_typed_into_puts_it_away_too() {
        let (zones, surfaces, look, mut control, display, parent) = beside_a_display("one-box");
        let papers = zones.borrow().core().config().zones[0].id.clone();
        answer(&zones, &surfaces, &look, &palette(), Ask::Rename { zone: papers.clone() }, &display, parent, &mut control);
        let typed_into = surfaces.borrow().typing.expect("a box was opened");
        assert!(control.is_open(typed_into));
        assert_eq!(zones.borrow().renaming, Some(papers));
        // With a menu open beside it: one press, and both are gone.
        answer(&zones, &surfaces, &look, &palette(), Ask::DeskMenu { display: display.clone(), at: pos2(1100.0, 700.0) }, &display, parent, &mut control);
        let menu = surfaces.borrow().menu().unwrap();
        dismiss(&zones, &surfaces, &mut control);
        assert!(!control.is_open(typed_into) && !control.is_open(menu));
        assert!(surfaces.borrow().typing.is_none() && zones.borrow().renaming.is_none());
        // What was looked for stays looked for when its box is put away so.
        zones.borrow_mut().filter = "report".to_string();
        answer(&zones, &surfaces, &look, &palette(), Ask::Filter, &display, parent, &mut control);
        dismiss(&zones, &surfaces, &mut control);
        assert_eq!(zones.borrow().filter, "report");
    }

    #[test]
    fn hiding_everything_puts_the_open_menu_away() {
        let (zones, surfaces, look, mut control, display, parent) = beside_a_display("hidden-menu");
        let bare = Ask::DeskMenu { display: display.clone(), at: pos2(1100.0, 700.0) };
        sync(&zones, &surfaces, &mut control, &look);
        answer(&zones, &surfaces, &look, &palette(), bare.clone(), &display, parent, &mut control);
        let menu = surfaces.borrow().menu().unwrap();
        // Nothing changed: the menu stays.
        sync(&zones, &surfaces, &mut control, &look);
        assert!(control.is_open(menu));
        // Hidden from elsewhere (a key, another program) while it is open.
        zones.borrow_mut().command(Command::QuickHide { hidden: Some(true) });
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!control.is_open(menu) && surfaces.borrow().menu().is_none());
        // The menu asked for while hidden stays (it is the way back), and
        // showing everything again closes nothing.
        answer(&zones, &surfaces, &look, &palette(), bare.clone(), &display, parent, &mut control);
        let back = surfaces.borrow().menu().unwrap();
        sync(&zones, &surfaces, &mut control, &look);
        assert!(control.is_open(back));
        zones.borrow_mut().command(Command::QuickHide { hidden: Some(false) });
        sync(&zones, &surfaces, &mut control, &look);
        assert!(control.is_open(back));
        // Switched off in the configuration: away as well.
        let mut off = zones.borrow().core().config().clone();
        off.enabled = false;
        zones.borrow_mut().configure(&off);
        sync(&zones, &surfaces, &mut control, &look);
        assert!(!control.is_open(back));
    }

    #[test]
    fn a_button_going_down_or_escape_with_the_keyboard_is_a_press_elsewhere() {
        use crate::zones::draw::tests::button;
        let at = pos2(400.0, 300.0);
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let heard = |events: Vec<egui::Event>, focused: bool| {
            let context = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))),
                events,
                focused,
                ..Default::default()
            };
            let mut heard = false;
            let mut output = context.run_ui(input, |ui| heard = ui.input(pressed_elsewhere));
            // (Nothing is drawn with: the textures go nowhere.)
            output.textures_delta.clear();
            heard
        };
        assert!(heard(vec![button(at, egui::PointerButton::Primary, true)], false));
        assert!(heard(vec![button(at, egui::PointerButton::Secondary, true)], false));
        // (Down and up within one frame is a press all the same.)
        assert!(heard(
            vec![button(at, egui::PointerButton::Primary, true), button(at, egui::PointerButton::Primary, false)],
            false
        ));
        assert!(!heard(vec![egui::Event::PointerMoved(at)], true));
        assert!(!heard(vec![button(at, egui::PointerButton::Primary, false)], true));
        assert!(heard(vec![key(true)], true));
        // The key of a window that has the keyboard is not ours.
        assert!(!heard(vec![key(true)], false));
        assert!(!heard(Vec::new(), true));
    }

    #[test]
    fn hidden_the_desktops_menu_is_the_way_back_wherever_it_is_asked_for() {
        use crate::zones::draw::tests::{button, frames};
        let labels = |lines: &[Line]| -> Vec<String> {
            lines
                .iter()
                .filter_map(|line| match line {
                    Line::Choice { label, .. } | Line::Sub { label, .. } => Some(label.clone()),
                    Line::Gap => None,
                })
                .collect()
        };
        let display = DisplayId::new("picture-1");
        // On the bare desktop, and where a zone lies that is hidden.
        for (name, at) in [("menu-bare", pos2(1100.0, 700.0)), ("menu-zone", pos2(300.0, 150.0))] {
            let mut zones = a_working_desktop(name);
            let context = egui::Context::default();
            gevrek_ui::theme::install_fonts(&context, false);
            zones.command(Command::QuickHide { hidden: Some(true) });
            let other = egui::PointerButton::Secondary;
            frames(
                &mut zones,
                &context,
                10.0,
                vec![vec![egui::Event::PointerMoved(at)], vec![button(at, other, true)], vec![button(at, other, false)]],
            );
            let asks = zones.take_asks();
            let [Ask::DeskMenu { at: asked_at, .. }] = asks.as_slice() else {
                panic!("{name}: the other button asked for {asks:?}");
            };
            let lines = desk_menu(&zones, &display, *asked_at, &palette());
            assert_eq!(labels(&lines), ["Show the zones"], "{name}");
            let Line::Choice { does: Does::Command(command), .. } = &lines[0] else {
                panic!("the line is a command");
            };
            zones.command(command.clone());
            assert!(!zones.hidden() && !zones.layout().zones.is_empty());
            assert!(labels(&desk_menu(&zones, &display, *asked_at, &palette())).contains(&"Hide the zones".to_string()));
        }
    }

    #[test]
    fn the_workspace_the_zones_go_by_is_the_one_shown_where_the_keyboard_is() {
        assert_eq!(shown_workspace(&serde_json::json!({ "monitors": [] })), None);
        assert_eq!(shown_workspace(&serde_json::json!({})), None);
    }

    #[test]
    fn the_desktops_menu_offers_sorting_only_where_there_are_rules() {
        let mut zones = a_working_desktop("desk-menu");
        let display = DisplayId::new("picture-1");
        // The lines of the menu itself: those that lead to a menu beside
        // them are said with an arrow here.
        let own = |zones: &Zones| -> Vec<String> {
            desk_menu(zones, &display, pos2(600.0, 400.0), &palette())
                .into_iter()
                .filter_map(|line| match line {
                    Line::Choice { label, .. } => Some(label),
                    Line::Sub { label, .. } => Some(format!("{label} >")),
                    Line::Gap => None,
                })
                .collect()
        };
        // Every line that does something, with the way to it.
        let labels = |zones: &Zones| -> Vec<String> {
            every_line(&desk_menu(zones, &display, pos2(600.0, 400.0), &palette())).into_iter().map(|(way, _)| way).collect()
        };
        assert_eq!(
            own(&zones),
            [
                "New zone >",
                "Find an item\u{2026}",
                "New folder on the desktop\u{2026}",
                "Take back the last change to a file",
                "Hide the zones",
                "Every icon grey",
                "Everything covered from view",
                "Held >",
                "Look of all zones >",
                "Arrangements >"
            ]
        );
        // Every way to a new zone is beside the one line, and the zones
        // that fill themselves beside theirs.
        let new: Vec<String> = labels(&zones).into_iter().filter(|way| way.starts_with("New zone \u{203a}")).collect();
        assert_eq!(
            new,
            [
                "New zone \u{203a} Empty, here",
                "New zone \u{203a} Of a folder\u{2026}",
                "New zone \u{203a} That fills itself \u{203a} The files used last",
                "New zone \u{203a} That fills itself \u{203a} What was downloaded today",
                "New zone \u{203a} That fills itself \u{203a} The programs that run",
                "New zone \u{203a} That fills itself \u{203a} The pinned programs",
                "New zone \u{203a} That fills itself \u{203a} The pictures on the desktop",
                "New zone \u{203a} Make zones by type\u{2026}"
            ]
        );
        assert!(labels(&zones).contains(&"Held \u{203a} Every zone held where it is".to_string()));
        assert!(labels(&zones).contains(&"Look of all zones \u{203a} Ground \u{203a} Solid ground".to_string()));
        assert!(labels(&zones).contains(&"Arrangements \u{203a} Keep this arrangement as\u{2026}".to_string()));
        assert!(!own(&zones).contains(&"Rules >".to_string()), "there are no rules");
        // Zones made by type can be taken back from the same place, once
        // some were made (and no longer when they were).
        assert!(!labels(&zones).iter().any(|way| way.ends_with("Take the automatic zones back")));
        zones.command(Command::AutoCreate { scope: Scope::Loose, min_items: Some(1), display: None, add_rules: Some(false), dry_run: false });
        assert!(labels(&zones).contains(&"New zone \u{203a} Take the automatic zones back".to_string()));
        zones.command(Command::UndoAutoCreate);
        assert!(!labels(&zones).iter().any(|way| way.ends_with("Take the automatic zones back")));
        let mut config = zones.core().config().clone();
        config.catch_all = Some(gevrek_zones::config::RuleAction::AssignToZone { zone: config.zones[0].id.clone() });
        zones.configure(&config);
        assert!(labels(&zones).contains(&"Rules \u{203a} Sort the desktop by the rules".to_string()));
        assert!(labels(&zones).contains(&"Rules \u{203a} What the rules would do\u{2026}".to_string()));
        zones.command(Command::ApplyRules { scope: Scope::Loose });
        assert!(labels(&zones).contains(&"Rules \u{203a} Take the sorting back".to_string()));
        // The new zone is made where the menu was opened.
        let menu = desk_menu(&zones, &display, pos2(600.4, 400.6), &palette());
        let all = every_line(&menu);
        let Some((_, Line::Choice { does: Does::Command(Command::CreateZone { rect: Some(rect), .. }), .. })) = all.first() else {
            panic!("the first line makes a zone");
        };
        assert_eq!((rect.x, rect.y), (600, 401));
        // Every icon grey is marked while it is so.
        zones.command(Command::GreyIcons { on: Some(true) });
        let menu = desk_menu(&zones, &display, pos2(600.0, 400.0), &palette());
        assert!(menu.iter().any(|line| matches!(line, Line::Choice { label, marked: true, .. } if label == "Every icon grey")));
    }

    /// The line of a menu that is open, by what it says.
    fn line_called(open: &Rc<RefCell<OpenMenus>>, level: usize, label: &str) -> usize {
        let open = open.borrow();
        open.chain.levels[level]
            .lines
            .iter()
            .position(|line| matches!(line, Line::Choice { label: has, .. } | Line::Sub { label: has, .. } if has == label))
            .unwrap_or_else(|| panic!("no line {label:?} in menu {level}"))
    }

    #[test]
    fn a_menu_opens_beside_the_line_that_leads_to_it_and_the_one_it_came_from_stays() {
        let (zones, surfaces, look, mut control, display, parent) = beside_a_display("menus-beside");
        answer(&zones, &surfaces, &look, &palette(), Ask::DeskMenu { display: display.clone(), at: pos2(600.0, 400.0) }, &display, parent, &mut control);
        let open = surfaces.borrow().menu.clone().expect("a menu was opened");
        let menus = |surfaces: &Rc<RefCell<Surfaces>>| surfaces.borrow().menus();
        assert_eq!(menus(&surfaces).len(), 1);
        let first = menus(&surfaces)[0];
        // The line is pressed: its menu opens, and the first stays.
        let new_zone = line_called(&open, 0, "New zone");
        assert_eq!(open.borrow_mut().chain.pressed(0, new_zone), None);
        fit(&open, &zones, &look, &mut control);
        let now = menus(&surfaces);
        assert_eq!(now.len(), 2);
        assert!(now[0] == first && control.is_open(now[0]) && control.is_open(now[1]));
        // And from that one the next.
        let fills_itself = line_called(&open, 1, "That fills itself");
        open.borrow_mut().chain.pressed(1, fills_itself);
        fit(&open, &zones, &look, &mut control);
        let three = menus(&surfaces);
        assert_eq!(three.len(), 3);
        assert!(three.iter().all(|menu| control.is_open(*menu)));
        // Fitting again with nothing changed opens and closes nothing.
        fit(&open, &zones, &look, &mut control);
        assert_eq!(menus(&surfaces), three);
        // The left arrow goes one menu back; the others stay.
        assert!(menu_keys(&zones, &open, &look, vec![menu::Key::Back], &mut control));
        assert_eq!(menus(&surfaces), three[..2]);
        assert!(!control.is_open(three[2]) && control.is_open(three[1]));
        // Another line of the first menu that leads elsewhere: one way
        // through the menus is open at a time.
        let held = line_called(&open, 0, "Held");
        open.borrow_mut().chain.pressed(0, held);
        fit(&open, &zones, &look, &mut control);
        let other = menus(&surfaces);
        assert_eq!(other.len(), 2);
        assert!(other[0] == first && other[1] != three[1] && !control.is_open(three[1]) && control.is_open(other[1]));
        // A menu the host closed itself (Escape, where the host hears
        // it) is no longer open here either, nor what opened from it.
        control.close(other[1]);
        assert!(menu_keys(&zones, &open, &look, Vec::new(), &mut control));
        assert_eq!(menus(&surfaces), [first]);
        assert_eq!(open.borrow().chain.levels.len(), 1);
        assert_eq!(open.borrow().chain.levels[0].open, None);
        // Escape in the first menu closes it: no menu is open.
        assert!(!menu_keys(&zones, &open, &look, vec![menu::Key::Escape], &mut control));
        assert!(!control.is_open(first) && menus(&surfaces).is_empty());
    }

    #[test]
    fn a_line_that_does_something_closes_every_menu_and_the_keys_choose_as_the_pointer_does() {
        let (zones, surfaces, look, mut control, display, parent) = beside_a_display("menus-chosen");
        let papers = zones.borrow().core().config().zones[0].id.clone();
        // Asked for by the key for menus: the keys are on its first line.
        zones.borrow_mut().command(Command::FocusZone { zone: Some(papers.clone()) });
        zones.borrow_mut().key(crate::zones::gesture::Key::Menu);
        let asks = zones.borrow_mut().take_asks();
        let [Ask::ZoneMenu { zone, .. }] = asks.as_slice() else {
            panic!("the key asked for {asks:?}");
        };
        assert_eq!(zone, &papers);
        answer(&zones, &surfaces, &look, &palette(), asks[0].clone(), &display, parent, &mut control);
        let open = surfaces.borrow().menu.clone().unwrap();
        assert_eq!(open.borrow().chain.levels[0].at, Some(0));
        // Down to "Shown and ordered", into it, down to "As a list", Enter.
        use menu::Key::{Choose, Down, Into};
        assert!(menu_keys(&zones, &open, &look, vec![Down, Down, Into], &mut control));
        let all = surfaces.borrow().menus();
        assert_eq!(all.len(), 2);
        assert_eq!(open.borrow().chain.levels[1].at, Some(0), "the keys are on its first line");
        assert!(!menu_keys(&zones, &open, &look, vec![Down, Choose], &mut control), "no menu is left open");
        assert!(all.iter().all(|menu| !control.is_open(*menu)));
        assert_eq!(zones.borrow().core().config().zone(&papers).unwrap().view, View::List);
        // A menu asked for by the pointer has the keys on no line yet.
        answer(&zones, &surfaces, &look, &palette(), Ask::ZoneMenu { zone: papers.clone(), at: pos2(300.0, 150.0) }, &display, parent, &mut control);
        let open = surfaces.borrow().menu.clone().unwrap();
        assert_eq!(open.borrow().chain.levels[0].at, None);
        // A press beside the menus puts all of them away.
        let shown = line_called(&open, 0, "Shown and ordered");
        open.borrow_mut().chain.pressed(0, shown);
        fit(&open, &zones, &look, &mut control);
        let all = surfaces.borrow().menus();
        assert_eq!(all.len(), 2);
        dismiss(&zones, &surfaces, &mut control);
        assert!(all.iter().all(|menu| !control.is_open(*menu)) && surfaces.borrow().menu().is_none());
        // The button of a title asks for the one menu of how a zone is
        // shown and ordered, by itself.
        answer(&zones, &surfaces, &look, &palette(), Ask::ViewMenu { zone: papers.clone(), at: pos2(300.0, 150.0) }, &display, parent, &mut control);
        let open = surfaces.borrow().menu.clone().unwrap();
        assert_eq!(open.borrow().chain.levels[0].lines, view_menu(&zones.borrow(), &papers));
    }

    #[test]
    fn the_programs_some_items_open_with_stand_beside_the_line_that_leads_to_them() {
        let action = |id: &str, label: &str| DeskAction { id: id.to_string(), label: label.to_string() };
        let items = vec![gevrek_zones::ItemId::new("/desk/a.txt")];
        let lines = [action("open", "Open"), action(crate::zones::CHOOSE, "Open with\u{2026}"), action(crate::zones::RENAME, "Rename\u{2026}")];
        let programs = [action("with:editor", "Editor"), action("with:viewer", "Viewer")];
        let menu = item_menu(&items, &lines, &programs);
        let ways: Vec<String> = every_line(&menu).into_iter().map(|(way, _)| way).collect();
        assert_eq!(ways, ["Open", "Open with \u{203a} Editor", "Open with \u{203a} Viewer", "Rename\u{2026}"]);
        let Line::Sub { lines: beside, .. } = &menu[1] else {
            panic!("the line leads to a menu");
        };
        assert_eq!(beside[1], Line::Choice { label: "Viewer".to_string(), marked: false, does: Does::Act { items: items.clone(), action: "with:viewer".to_string() } });
        // Where the system names none, the line stays one that asks.
        let menu = item_menu(&items, &lines, &[]);
        assert!(matches!(&menu[1], Line::Choice { does: Does::Act { action, .. }, .. } if action == crate::zones::CHOOSE));
    }

    #[test]
    fn the_zones_by_type_are_looked_over_made_and_taken_back_from_their_sheet() {
        use crate::zones::draw::tests::a_desk_to_sort;
        let mut zones = a_desk_to_sort("auto-sheet");
        let display = DisplayId::new("picture-1");
        let before = zones.core().config().zones.len();
        let preview = zones.auto_preview(Scope::Loose, &display);
        let titles: Vec<&str> = preview.plan.zones.iter().map(|zone| zone.title.as_str()).collect();
        assert_eq!(titles, ["Folders", "Programs", "Documents", "Pictures", "Video", "Audio", "Archives", "Code"]);
        assert!(!preview.can_undo);
        // Every item of the plan has its name, and the picture knows the
        // display and the zone that stands on it.
        assert!(preview.plan.zones.iter().flat_map(|zone| &zone.items).all(|item| preview.names.contains_key(item)));
        assert_eq!(preview.display, Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0)));
        assert_eq!(preview.there, [Rect::from_min_size(pos2(900.0, 520.0), vec2(340.0, 220.0))]);
        // Nothing was changed by looking.
        assert_eq!(zones.core().config().zones.len(), before);
        // No new zone lies over the one that is there, nor over another.
        let planned: Vec<Rect> = preview
            .plan
            .zones
            .iter()
            .map(|zone| Rect::from_min_size(pos2(zone.rect.x as f32, zone.rect.y as f32), vec2(zone.rect.width as f32, zone.rect.height as f32)))
            .collect();
        for (number, rect) in planned.iter().enumerate() {
            assert!(!rect.intersects(preview.there[0]), "{rect:?}");
            assert!(planned[number + 1..].iter().all(|other| !rect.intersects(*other)), "{rect:?}");
            assert!(preview.display.contains_rect(*rect));
        }
        // Made, as the sheet's button does it.
        zones.command(Command::AutoCreate { scope: Scope::Loose, min_items: None, display: Some(display.clone()), add_rules: Some(true), dry_run: false });
        assert_eq!(zones.core().config().zones.len(), before + 8);
        let after = zones.auto_preview(Scope::Loose, &display);
        assert!(after.plan.zones.is_empty() && after.can_undo, "{:?}", after.plan);
        assert_eq!(zones.layout().zones.len(), before + 8);
        // And taken back.
        zones.command(Command::UndoAutoCreate);
        assert_eq!(zones.core().config().zones.len(), before);
        assert!(!zones.auto_preview(Scope::Loose, &display).can_undo);
    }

    /// One frame of a menu's surface: what it heard, and what came of it
    /// (the line under the pointer, the line pressed, and what the frame
    /// tells a screen reader).
    fn menu_frame(
        context: &egui::Context,
        shown: &menu::Level,
        time: f64,
        events: Vec<egui::Event>,
    ) -> ((Option<usize>, Option<usize>), Option<egui::accesskit::TreeUpdate>) {
        let size = menu_size(&shown.lines);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut came = (None, None);
        let mut output = context.run_ui(input, |ui| {
            came = menu(ui, &palette(), shown);
        });
        output.textures_delta.clear();
        (came, output.platform_output.accesskit_update.take())
    }

    #[test]
    fn a_menu_says_where_the_pointer_is_and_what_was_pressed_and_tells_a_screen_reader_its_lines() {
        use egui::accesskit::{Action, HasPopup, Role, Toggled};
        let zones = a_working_desktop("menu-drawn");
        let papers = zones.core().config().zones[0].id.clone();
        let mut chain = Chain::of(zone_menu(&zones, &papers, &palette()), false);
        let context = egui::Context::default();
        gevrek_ui::theme::install_fonts(&context, false);
        let rows = menu::rows(&chain.levels[0].lines);
        let main = egui::PointerButton::Primary;
        let click = |at: Pos2, pressed: bool| egui::Event::PointerButton { pos: at, button: main, pressed, modifiers: egui::Modifiers::NONE };
        // The pointer over the third line ("Shown and ordered").
        let third = rows[2].center();
        menu_frame(&context, &chain.levels[0], 1.0, vec![egui::Event::PointerMoved(third)]);
        let (came, _) = menu_frame(&context, &chain.levels[0], 1.1, Vec::new());
        assert_eq!(came, (Some(2), None));
        // Over a gap: on no line.
        let gap = chain.levels[0].lines.iter().position(|line| *line == Line::Gap).unwrap();
        menu_frame(&context, &chain.levels[0], 1.2, vec![egui::Event::PointerMoved(rows[gap].center())]);
        assert_eq!(menu_frame(&context, &chain.levels[0], 1.3, Vec::new()).0, (None, None));
        // Pressed and let go on the first line ("Rename...").
        let first = rows[0].center();
        menu_frame(&context, &chain.levels[0], 1.4, vec![egui::Event::PointerMoved(first)]);
        menu_frame(&context, &chain.levels[0], 1.5, vec![click(first, true)]);
        let (came, _) = menu_frame(&context, &chain.levels[0], 1.6, vec![click(first, false)]);
        assert_eq!(came, (Some(0), Some(0)));
        assert!(matches!(chain.pressed(0, 0), Some(Does::Ask(Ask::Rename { .. }))));
        // What a screen reader is told: a menu of items, the marked ones
        // as marked, and the ones that lead on as having a menu.
        chain.pressed(0, 2);
        context.enable_accesskit();
        menu_frame(&context, &chain.levels[0], 2.0, Vec::new());
        let (_, told) = menu_frame(&context, &chain.levels[0], 2.1, Vec::new());
        let update = told.expect("a tree for the screen reader");
        assert!(update.nodes.iter().any(|(_, node)| node.role() == Role::Menu));
        let named = |name: &str| update.nodes.iter().find(|(_, node)| node.label() == Some(name)).unwrap_or_else(|| panic!("no node is called {name}"));
        assert_eq!(named("Rename").1.role(), Role::MenuItem);
        let (_, shown_and_ordered) = named("Shown and ordered");
        assert_eq!(shown_and_ordered.has_popup(), Some(HasPopup::Menu));
        assert_eq!(shown_and_ordered.is_expanded(), Some(true), "its menu is open beside it");
        assert_eq!(named("Look").1.is_expanded(), Some(false));
        // A marked line of the menu beside it.
        let (_, as_a_grid) = {
            let (_, told) = {
                menu_frame(&context, &chain.levels[1], 2.2, Vec::new());
                menu_frame(&context, &chain.levels[1], 2.3, Vec::new())
            };
            let update = told.unwrap();
            let node = update.nodes.iter().find(|(_, node)| node.label() == Some("As a grid")).unwrap().1.clone();
            ((), node)
        };
        assert_eq!((as_a_grid.role(), as_a_grid.toggled()), (Role::MenuItemCheckBox, Some(Toggled::True)));
        // Pressed for the person by the screen reader: a press.
        let (rename_id, _) = named("Rename");
        let request = egui::accesskit::ActionRequest {
            action: Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: *rename_id,
            data: None,
        };
        let (came, _) = menu_frame(&context, &chain.levels[0], 2.4, vec![egui::Event::AccessKitActionRequest(request)]);
        assert_eq!(came.1, Some(0));
    }

    #[test]
    fn the_keys_of_a_menu_are_taken_out_of_what_a_surface_heard() {
        let context = egui::Context::default();
        let key = |key: egui::Key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0))),
            events: vec![key(egui::Key::ArrowDown), key(egui::Key::ArrowDown), key(egui::Key::ArrowRight), key(egui::Key::Escape), key(egui::Key::Delete)],
            focused: true,
            ..Default::default()
        };
        let mut heard = Vec::new();
        let mut left = (false, false, false);
        let mut output = context.run_ui(input, |ui| {
            heard = ui.input_mut(menu::keys);
            left = ui.input(|input| (input.key_pressed(egui::Key::ArrowDown), input.key_pressed(egui::Key::Escape), input.key_pressed(egui::Key::Delete)));
            // (So a press of Escape that was a menu's puts nothing else away.)
            assert!(!ui.input(pressed_elsewhere));
        });
        output.textures_delta.clear();
        use menu::Key::{Down, Escape, Into};
        assert_eq!(heard, [Down, Down, Into, Escape]);
        assert_eq!(left, (false, false, true), "the menu's keys are gone, the others are there");
    }
}
