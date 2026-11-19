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
    let wanted = zones.borrow_mut().spots();
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
        if let Some((surface, _)) = open_now.displays.remove(&display) {
            if control.is_open(surface) {
                control.close(surface);
            }
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
            // What the system shows on the zones' behalf while this
            // surface is worked on (its menu, its progress) belongs to it.
            #[cfg(windows)]
            {
                let own = all.borrow().displays.get(&here).map(|(surface, _)| *surface);
                if let Some(window) = own.and_then(|own| control.native_window(own)) {
                    shared.borrow_mut().desk.set_owner(window);
                }
            }
            let (asks, others) = {
                let mut zones = shared.borrow_mut();
                draw::draw(&mut zones, ui, &here, &Look { palette: &palette, settings: &settings });
                (zones.take_asks(), std::mem::take(&mut zones.redraw))
            };
            let own = all.borrow().displays.get(&here).map(|(surface, _)| *surface);
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
                    answer(&shared, &all, &look, ask, &here, own, control);
                }
            }
            Flow::Continue
        });
        // Until a drawing says more: our own items, moved. (The answer a
        // drawing gives is seen a frame later; most of what is dragged
        // over the desktop is the desktop's own.)
        control.accept_drops(surface, Some(STANDING));
        open_now.displays.insert(display, (surface, spot));
    }
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
                OVER.with(|over| *over.borrow_mut() = Some(payload));
                control.answer_drag(own, answer_of(would));
            }
            DragEvent::Moved { at } => {
                let payload = OVER.with(|over| over.borrow().clone()).unwrap_or_default();
                let would = zones.drag_over(Some(at + origin.to_vec2()), &payload);
                control.answer_drag(own, answer_of(would));
            }
            DragEvent::Left => {
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
                log::debug!("a drop on the zones: {came:?}");
            }
            DragEvent::Ended { outcome } => {
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
        for event in heard {
            link.publish_zones_event(event);
        }
        let mut snapshot = gevrek_zones::ipc::ZonesSnapshot::of(zones.core(), super::now());
        snapshot.not_shown = zones.not_shown();
        link.publish_zones(snapshot);
    }
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
        link.answer_zones(request, answer);
    }
}

/// One line of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// Something to choose, perhaps marked as what is in force.
    Choice { label: String, marked: bool, does: Does },
    /// A little room between two groups of lines.
    Gap,
}

/// What choosing a line of a menu does.
#[derive(Debug, Clone, PartialEq)]
pub enum Does {
    Command(Command),
    /// Ask for a box to type into.
    Ask(Ask),
    /// A line the backend gave for some items.
    Act { items: Vec<gevrek_zones::ItemId>, action: String },
}

fn choice(label: &str, does: Does) -> Line {
    Line::Choice { label: label.to_string(), marked: false, does }
}

fn marked(label: &str, marked: bool, command: Command) -> Line {
    Line::Choice { label: label.to_string(), marked, does: Does::Command(command) }
}

/// The menu of the bare desktop, pressed at a point of it.
pub fn desk_menu(zones: &Zones, at: Pos2) -> Vec<Line> {
    let here = gevrek_zones::Rect::new(at.x.round() as i32, at.y.round() as i32, 320, 240);
    let mut lines = vec![
        choice(
            "New zone here",
            Does::Command(Command::CreateZone { title: "New zone".to_string(), kind: ZoneKind::Manual, rect: Some(here) }),
        ),
        choice("Find an item\u{2026}", Does::Ask(Ask::Filter)),
        Line::Gap,
    ];
    if !zones.core().config().rules.is_empty() || zones.core().config().catch_all.is_some() {
        lines.push(choice("Sort the desktop by the rules", Does::Command(Command::ApplyRules { scope: Scope::Loose })));
    }
    if zones.core().state().undo.is_some() {
        lines.push(choice("Take the sorting back", Does::Command(Command::UndoRules)));
    }
    lines.push(choice("Hide the zones", Does::Command(Command::QuickHide { hidden: Some(true) })));
    lines
}

/// The menu of a zone: its name, how it shows and orders what it holds,
/// what of it is held fast, and taking it away.
pub fn zone_menu(zones: &Zones, zone: &ZoneId) -> Vec<Line> {
    let Some(definition) = zones.core().config().zone(zone) else {
        return Vec::new();
    };
    let id = || zone.clone();
    let view = |view: View| Command::SetView { zone: id(), view: Some(view), sort: None, descending: None, icon_size: None };
    let sort = |sort: Sort| Command::SetView { zone: id(), view: None, sort: Some(sort), descending: None, icon_size: None };
    let size = |size: u32| Command::SetView { zone: id(), view: None, sort: None, descending: None, icon_size: Some(size) };
    let mut lines = vec![
        choice("Rename\u{2026}", Does::Ask(Ask::Rename { zone: id() })),
        marked("Rolled up", definition.rolled_up, Command::RollUp { zone: id(), rolled_up: None }),
        Line::Gap,
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
    lines.push(Line::Gap);
    lines.push(marked(
        "Held where it is",
        definition.lock.geometry,
        Command::Lock { zone: id(), geometry: Some(!definition.lock.geometry), members: None },
    ));
    lines.push(marked(
        "Nothing in or out",
        definition.lock.members,
        Command::Lock { zone: id(), geometry: None, members: Some(!definition.lock.members) },
    ));
    if definition.tab_group.is_some() {
        lines.push(choice("Out of its tabs", Does::Command(Command::Untab { zone: id() })));
    }
    lines.push(Line::Gap);
    lines.push(choice("Take this zone away", Does::Command(Command::DeleteZone { zone: id() })));
    lines
}

/// The lines the backend gave for some items, as a menu.
fn item_menu(items: &[gevrek_zones::ItemId], lines: &[DeskAction]) -> Vec<Line> {
    lines
        .iter()
        .map(|line| choice(&line.label, Does::Act { items: items.to_vec(), action: line.id.clone() }))
        .collect()
}

const LINE_HEIGHT: f32 = 28.0;
const GAP_HEIGHT: f32 = 9.0;
const MENU_WIDTH: f32 = 236.0;

/// How large a menu of these lines is, in points.
pub fn menu_size(lines: &[Line]) -> egui::Vec2 {
    let high: f32 = lines
        .iter()
        .map(|line| if *line == Line::Gap { GAP_HEIGHT } else { LINE_HEIGHT + 2.0 })
        .sum();
    vec2(MENU_WIDTH, 12.0 + high)
}

/// Draw a menu; the line that was pressed, if one was.
pub fn menu(ui: &mut egui::Ui, palette: &Palette, lines: &[Line]) -> Option<usize> {
    let mut chosen = None;
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(palette.surface).inner_margin(egui::Margin::same(6)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (index, line) in lines.iter().enumerate() {
                let Line::Choice { label, marked, .. } = line else {
                    let (gap, _) = ui.allocate_exact_size(vec2(ui.available_width(), GAP_HEIGHT - 2.0), egui::Sense::hover());
                    ui.painter().hline(
                        (gap.left() + 6.0)..=(gap.right() - 6.0),
                        gap.center().y,
                        egui::Stroke::new(1.0, palette.border.gamma_multiply(0.6)),
                    );
                    continue;
                };
                let (row, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), LINE_HEIGHT), egui::Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(row, 6.0, palette.control_hover());
                }
                if *marked {
                    // The same short mark a zone's title has.
                    let mark = Rect::from_center_size(pos2(row.left() + 9.0, row.center().y), vec2(3.0, 12.0));
                    ui.painter().rect_filled(mark, 1.5, palette.accent);
                }
                ui.painter().text(
                    pos2(row.left() + 20.0, row.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(13.0),
                    palette.text,
                );
                if response.clicked() {
                    chosen = Some(index);
                }
            }
        });
    chosen
}

/// Open what the pointer asked for: a menu beside where it was pressed,
/// or a box to type into.
fn answer(
    zones: &Shared,
    surfaces: &Rc<RefCell<Surfaces>>,
    themed: &Themed,
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
        Ask::DeskMenu { at, .. } => (desk_menu(&zones.borrow(), at), at),
        Ask::ZoneMenu { ref zone, at } => (zone_menu(&zones.borrow(), zone), at),
        Ask::ItemMenu { ref items, at, ref lines } => (item_menu(items, lines), at),
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
    if lines.is_empty() {
        return;
    }
    let anchor = Rect::from_min_size(at - origin.to_vec2(), vec2(1.0, 1.0));
    let shared = Rc::clone(zones);
    let look = Rc::clone(themed);
    let mut fonts = false;
    control.popup(parent, anchor, menu_size(&lines), move |ui, _| {
        let (palette, _) = look(ui, &mut fonts);
        let Some(index) = menu(ui, &palette, &lines) else {
            return Flow::Continue;
        };
        if let Line::Choice { does, .. } = &lines[index] {
            let mut zones = shared.borrow_mut();
            match does.clone() {
                Does::Command(command) => {
                    zones.command(command);
                }
                Does::Ask(ask) => zones.ask(ask),
                Does::Act { items, action } => zones.act(&items, &action),
            }
            // (What follows is seen when the shell next looks at the zones.)
            (zones.wake_later)(std::time::Duration::ZERO);
        }
        Flow::Close
    });
}

/// How large the box to type into is, in points.
const BOX: (u32, u32) = (360, 96);

/// What a box is typed into for.
#[derive(Debug, Clone, PartialEq)]
enum Typed {
    ZoneName(ZoneId),
    ItemName(gevrek_zones::ItemId),
    Find,
}

/// Open the box a zone's or a file's name is typed into, or the one for
/// what is looked for. It has the keyboard until Enter, Escape, or a
/// press elsewhere.
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
        let lines = zone_menu(&zones, &downloads);
        let marked: Vec<&str> = lines
            .iter()
            .filter_map(|line| match line {
                Line::Choice { label, marked: true, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(marked, ["As a list", "By name", "Medium icons"]);
        assert!(lines.iter().all(|line| match line {
            Line::Choice { does, .. } => matches!(does, Does::Command(_) | Does::Ask(Ask::Rename { .. })),
            Line::Gap => true,
        }));
        // A zone that is a tab can be taken out of its tabs; one that is not, not.
        let has = |zone: &ZoneId, label: &str| {
            zone_menu(&zones, zone).iter().any(|line| matches!(line, Line::Choice { label: has, .. } if has == label))
        };
        let work = zones.core().config().zones[5].id.clone();
        assert!(has(&work, "Out of its tabs") && !has(&downloads, "Out of its tabs"));
        assert_eq!(zone_menu(&zones, &ZoneId::new("no-such")), []);
        let size = menu_size(&lines);
        assert!(size.x == MENU_WIDTH && size.y > 300.0 && size.y < 600.0, "{size:?}");
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
        ] {
            for call in asks_for_focus {
                assert!(!text.contains(call), "{file} has {call}");
            }
        }
        // Here, only the box that is typed into does: a surface of its own.
        let here = include_str!("surfaces.rs");
        let before_the_tests = here.split("#[cfg(test)]").next().unwrap();
        assert_eq!(before_the_tests.matches(".request_focus()").count(), 1);
        assert_eq!(before_the_tests.matches("keyboard: Keyboard::Exclusive").count(), 1);
        let typed_into = before_the_tests.split("fn type_box").nth(1).unwrap();
        assert!(typed_into.contains(".request_focus()"));
    }

    #[test]
    fn the_workspace_the_zones_go_by_is_the_one_shown_where_the_keyboard_is() {
        assert_eq!(shown_workspace(&serde_json::json!({ "monitors": [] })), None);
        assert_eq!(shown_workspace(&serde_json::json!({})), None);
    }

    #[test]
    fn the_desktops_menu_offers_sorting_only_where_there_are_rules() {
        let mut zones = a_working_desktop("desk-menu");
        let labels = |zones: &Zones| -> Vec<String> {
            desk_menu(zones, pos2(600.0, 400.0))
                .into_iter()
                .filter_map(|line| match line {
                    Line::Choice { label, .. } => Some(label),
                    Line::Gap => None,
                })
                .collect()
        };
        assert_eq!(labels(&zones), ["New zone here", "Find an item\u{2026}", "Hide the zones"]);
        let mut config = zones.core().config().clone();
        config.catch_all = Some(gevrek_zones::config::RuleAction::AssignToZone { zone: config.zones[0].id.clone() });
        zones.configure(&config);
        assert!(labels(&zones).contains(&"Sort the desktop by the rules".to_string()));
        zones.command(Command::ApplyRules { scope: Scope::Loose });
        assert!(labels(&zones).contains(&"Take the sorting back".to_string()));
        // The new zone is made where the menu was opened.
        let Line::Choice { does: Does::Command(Command::CreateZone { rect: Some(rect), .. }), .. } =
            &desk_menu(&zones, pos2(600.4, 400.6))[0]
        else {
            panic!("the first line makes a zone");
        };
        assert_eq!((rect.x, rect.y), (600, 401));
    }
}
