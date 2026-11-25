//! Drag and drop on Wayland: `wl_data_device`.
//!
//! The compositor tells the seat's data device when something dragged
//! comes over one of the shell's surfaces, and offers it in kinds of data
//! named as on the Internet. A surface that accepts drops says which kind
//! it would take and what it would do (copy or move: the compositor knows
//! no link); the compositor chooses the action with the source, and at the
//! drop the data is asked for and comes through a pipe. That pipe is read
//! on a thread of its own: the source writes when it likes, and the
//! connection to the compositor is never kept waiting for it.
//!
//! A drag out of a surface is a data source that offers the files in the
//! kinds file managers read, begun with the number of the press that
//! started it (the compositor lets nothing else begin one), with a
//! surface of its own for the picture that follows the pointer.
//!
//! Nothing here exists for a shell none of whose surfaces asked: the data
//! device is made at the first asking.

use super::{Host, SurfaceId};
use crate::dnd::{
    self, mime, DragAction, DragActions, DragEvent, DragImage, DragOutcome, DragPayload, DragStart,
    DropAnswer, Held, Offered, PathStyle,
};
use smithay_client_toolkit::data_device_manager::{
    data_device::{DataDevice, DataDeviceHandler},
    data_offer::{DataOfferHandler, DragOffer},
    data_source::{DataSourceHandler, DragSource},
    DataDeviceManagerState, ReadPipe, WritePipe,
};
use smithay_client_toolkit::reexports::calloop::channel;
use smithay_client_toolkit::shm::{
    slot::{Buffer, SlotPool},
    Shm, ShmHandler,
};
use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{
    wl_data_device::WlDataDevice, wl_data_device_manager::DndAction, wl_data_source::WlDataSource,
    wl_shm, wl_surface::WlSurface,
};
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

/// How long a source is waited for after a drop. One that has not sent
/// its data by then (it hangs, or is gone) is given up on, and the
/// surface is told the drag left.
const PATIENCE: Duration = Duration::from_secs(10);

/// The most that is read of what a source sends: a list of files is a
/// few lines, and a source that never ends is not listened to for ever.
const MOST: u64 = 16 * 1024 * 1024;

/// What was read from a source's pipe, for the loop: the read's number
/// and the bytes.
pub(super) type WasRead = (u64, Vec<u8>);

/// What the shell knows of drags.
pub(super) struct Dnd {
    manager: Option<DataDeviceManagerState>,
    /// The seat's data device, from the first surface that asks.
    device: Option<DataDevice>,
    /// Memory shared with the compositor, for a drag's picture.
    shm: Option<Shm>,
    /// The surfaces that take drops, each with what it takes of any drag.
    accepting: Vec<(SurfaceId, DropAnswer)>,
    /// The drag that is over one of them now.
    over: Option<Over>,
    /// Drops whose data is on its way.
    waiting: Vec<Waiting>,
    /// The drag one of the surfaces began.
    out: Option<Out>,
    /// Where the threads that read pipes leave what they read.
    read: channel::Sender<WasRead>,
    reads: u64,
    /// The mouse buttons held on a surface: told as let go when a drag
    /// takes the pointer away (their releases never come).
    pub(super) held: Vec<egui::PointerButton>,
    /// The machine's name, to know its own files by; read when first
    /// needed.
    host_name: Option<Option<String>>,
}

/// A drag over a surface that takes drops.
struct Over {
    on: SurfaceId,
    kinds: Vec<String>,
    allowed: DragActions,
    at: egui::Pos2,
    /// What the surface said of this drag.
    now: Option<DropAnswer>,
    /// The action the compositor chose, of what both sides allow.
    chosen: Option<DragAction>,
    /// What the compositor was last told: the kind, the actions taken,
    /// the one preferred.
    said: Option<(Option<&'static str>, u32, u32)>,
}

/// A drop whose data has been asked for.
struct Waiting {
    read: u64,
    on: SurfaceId,
    at: egui::Pos2,
    kind: &'static str,
    allowed: DragActions,
    action: DragAction,
    offer: DragOffer,
    until: Instant,
}

/// A drag out of a surface.
struct Out {
    from: SurfaceId,
    source: DragSource,
    payload: DragPayload,
    /// What the compositor last said would be done.
    action: Option<DragAction>,
    /// Kept while the drag lasts: the compositor shows it.
    _picture: Option<Picture>,
}

/// The surface that follows the pointer, and what its picture is in.
struct Picture {
    surface: WlSurface,
    viewport: Option<WpViewport>,
    _buffer: Buffer,
    _pool: SlotPool,
}

impl Drop for Picture {
    fn drop(&mut self) {
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        self.surface.destroy();
    }
}

impl Dnd {
    pub(super) fn new(
        globals: &GlobalList,
        handle: &QueueHandle<Host>,
        read: channel::Sender<WasRead>,
    ) -> Self {
        Self {
            manager: DataDeviceManagerState::bind(globals, handle).ok(),
            device: None,
            shm: Shm::bind(globals, handle).ok(),
            accepting: Vec::new(),
            over: None,
            waiting: Vec::new(),
            out: None,
            read,
            reads: 0,
            held: Vec::new(),
            host_name: None,
        }
    }

    /// When the loop must look again, at the latest: a source that is
    /// waited for is given up on.
    pub(super) fn wake_at(&self) -> Option<Instant> {
        self.waiting.iter().map(|waiting| waiting.until).min()
    }

    fn standing(&self, id: SurfaceId) -> Option<DropAnswer> {
        self.accepting
            .iter()
            .find(|(on, _)| *on == id)
            .map(|(_, answer)| *answer)
    }

    fn offer(&self) -> Option<DragOffer> {
        self.device.as_ref()?.data().drag_offer()
    }
}

fn action_of(action: DndAction) -> Option<DragAction> {
    if action == DndAction::Copy {
        Some(DragAction::Copy)
    } else if action == DndAction::Move {
        Some(DragAction::Move)
    } else {
        None
    }
}

fn actions_of(actions: DragActions) -> DndAction {
    DndAction::from_bits_truncate(actions.wayland())
}

/// Read a pipe to its end on a thread of its own and leave what was read
/// for the loop.
fn read_pipe(pipe: ReadPipe, read: u64, to: channel::Sender<WasRead>) {
    let work = move || {
        let mut bytes = Vec::new();
        if let Err(error) = std::fs::File::from(OwnedFd::from(pipe))
            .take(MOST)
            .read_to_end(&mut bytes)
        {
            log::debug!("what was dropped was read only in part: {error}");
        }
        if to.send((read, bytes)).is_err() {
            log::debug!("what was dropped was read after the shell ended");
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("gevrek-drop-read".to_string())
        .spawn(work)
    {
        log::warn!("what was dropped cannot be read: {error}");
    }
}

/// Write to a pipe on a thread of its own: a target reads when it likes.
fn write_pipe(pipe: WritePipe, bytes: Vec<u8>) {
    let work = move || {
        // (A target that stops reading early is its own business.)
        if let Err(error) = std::fs::File::from(OwnedFd::from(pipe)).write_all(&bytes) {
            log::debug!("what is dragged was not all taken: {error}");
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("gevrek-drag-write".to_string())
        .spawn(work)
    {
        log::warn!("what is dragged cannot be sent: {error}");
    }
}

impl Host {
    /// The seat's data device, made when a surface first has a use for
    /// it and the seat is known. `false`: not yet, or the compositor has
    /// none.
    pub(super) fn drag_device(&mut self) -> bool {
        if self.dnd.device.is_none() {
            if let (Some(manager), Some(seat)) = (&self.dnd.manager, &self.seat) {
                self.dnd.device = Some(manager.get_data_device(&self.handle, seat));
            }
        }
        self.dnd.device.is_some()
    }

    /// The seat is known: surfaces that asked before it was are served.
    pub(super) fn drag_seat_known(&mut self) {
        if !self.dnd.accepting.is_empty() {
            self.drag_device();
        }
    }

    fn drag_news(&mut self, id: SurfaceId, event: DragEvent) {
        if let Some(surface) = self.by_id(Some(id)) {
            surface.redraw = true;
            self.control.drag_news.push((id, event));
        }
    }

    /// A surface is closed: nothing more is told to it.
    pub(super) fn forget_drags(&mut self, id: SurfaceId) {
        self.dnd.accepting.retain(|(on, _)| *on != id);
        if self.dnd.over.as_ref().is_some_and(|over| over.on == id) {
            self.dnd.over = None;
        }
        self.dnd.waiting.retain(|waiting| {
            if waiting.on == id {
                waiting.offer.destroy();
            }
            waiting.on != id
        });
        self.control.drag_news.retain(|(on, _)| *on != id);
    }

    /// What handlers asked for of drags, carried out.
    pub(super) fn carry_out_drags(&mut self) {
        for (id, standing) in std::mem::take(&mut self.control.drops) {
            self.dnd.accepting.retain(|(on, _)| *on != id);
            match standing {
                Some(standing) if self.by_id(Some(id)).is_some() => {
                    self.dnd.accepting.push((id, standing));
                    if !self.drag_device() && self.dnd.manager.is_none() {
                        log::warn!("no drag and drop: the compositor has no data devices");
                    }
                }
                _ => {
                    if self.dnd.over.as_ref().is_some_and(|over| over.on == id) {
                        self.dnd.over = None;
                        self.refuse_offer();
                    }
                }
            }
        }
        let answers = std::mem::take(&mut self.control.answers);
        if !answers.is_empty() {
            for (id, answer) in answers {
                // (Of a drag that left already: not kept for the next.)
                if let Some(over) = self.dnd.over.as_mut().filter(|over| over.on == id) {
                    over.now = Some(answer);
                }
            }
            self.tell_offer();
        }
        for (from, start) in std::mem::take(&mut self.control.drags) {
            if let Err(why) = self.begin_drag(from, start) {
                log::warn!("a drag could not begin: {why}");
                self.drag_news(
                    from,
                    DragEvent::Ended {
                        outcome: DragOutcome::Cancelled,
                    },
                );
            }
        }
        // Sources that never sent what they promised.
        let now = Instant::now();
        let mut late = Vec::new();
        self.dnd.waiting.retain(|waiting| {
            if waiting.until <= now {
                waiting.offer.destroy();
                late.push(waiting.on);
            }
            waiting.until > now
        });
        for on in late {
            log::warn!("what was dropped never came: its source sent nothing");
            self.drag_news(on, DragEvent::Left);
        }
    }

    /// Say to the compositor what the surface under the drag would do.
    fn tell_offer(&mut self) {
        if let Some(offer) = self.dnd.offer() {
            self.tell_offer_of(&offer);
        }
    }

    /// As [`Host::tell_offer`], for an offer that is at hand. (Inside the
    /// offer's own events it must be the one that is handed in: the
    /// toolkit holds the offer's lock there, and asking the data device
    /// for it would wait for that lock for ever.)
    fn tell_offer_of(&mut self, offer: &DragOffer) {
        let Some(over) = &self.dnd.over else {
            return;
        };
        let answer = over
            .now
            .or_else(|| self.dnd.standing(over.on))
            .unwrap_or(DropAnswer::REFUSE);
        // (Which keys are held is for the source to say: it narrows what
        // it allows. A link is nothing a compositor can be told of.)
        let taken = DragActions {
            link: false,
            ..over.allowed.both(answer.accept)
        };
        let preferred = dnd::negotiate(
            over.allowed,
            Held::default(),
            DropAnswer {
                accept: taken,
                prefer: answer.prefer,
            },
        );
        let kind = mime::pick(&over.kinds).filter(|_| preferred.is_some());
        let saying = (
            kind,
            if kind.is_some() { taken.wayland() } else { 0 },
            preferred
                .filter(|_| kind.is_some())
                .map_or(0, |action| DragActions::only(action).wayland()),
        );
        if over.said == Some(saying) {
            return;
        }
        offer.accept_mime_type(offer.serial, kind.map(str::to_string));
        offer.set_actions(
            DndAction::from_bits_truncate(saying.1),
            DndAction::from_bits_truncate(saying.2),
        );
        if let Some(over) = self.dnd.over.as_mut() {
            over.said = Some(saying);
        }
    }

    /// Say that the drag that is over a surface is not taken there.
    fn refuse_offer(&mut self) {
        if let Some(offer) = self.dnd.offer() {
            offer.accept_mime_type(offer.serial, None);
            offer.set_actions(DndAction::empty(), DndAction::empty());
        }
    }

    /// What a source sent after a drop has arrived.
    pub(super) fn drop_read(&mut self, read: u64, bytes: Vec<u8>) {
        let Some(at) = self
            .dnd
            .waiting
            .iter()
            .position(|waiting| waiting.read == read)
        else {
            return;
        };
        let waiting = self.dnd.waiting.remove(at);
        let name = self
            .dnd
            .host_name
            .get_or_insert_with(|| {
                std::fs::read_to_string("/proc/sys/kernel/hostname")
                    .ok()
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty())
            })
            .clone();
        let mut payload = DragPayload {
            allowed: waiting.allowed,
            ..Default::default()
        };
        mime::read(
            &mut payload,
            waiting.kind,
            &bytes,
            PathStyle::Unix,
            name.as_deref(),
        );
        // The source is told that its data was taken, and may let go of
        // it.
        waiting.offer.finish();
        waiting.offer.destroy();
        self.drag_news(
            waiting.on,
            DragEvent::Dropped {
                at: waiting.at,
                payload,
                action: waiting.action,
            },
        );
    }

    /// Begin a drag out of a surface.
    fn begin_drag(&mut self, from: SurfaceId, start: DragStart) -> Result<(), &'static str> {
        if self.dnd.out.is_some() {
            return Err("another drag of the shell's is under way");
        }
        let Some(origin) = self
            .surfaces
            .iter()
            .find(|surface| surface.id == from)
            .map(|surface| surface.role.wl_surface().clone())
        else {
            return Err("the surface it begins on is gone");
        };
        let kinds = mime::kinds(&start.payload, PathStyle::Unix);
        if kinds.is_empty() {
            return Err("there is nothing to drag");
        }
        let actions = actions_of(start.payload.allowed);
        if actions.is_empty() {
            return Err("a Wayland compositor carries copies and moves only");
        }
        let Some(serial) = self.last_press else {
            return Err("no button was pressed on the shell's surfaces");
        };
        if !self.drag_device() {
            return Err("the compositor has no data devices");
        }
        let (Some(manager), Some(device)) = (&self.dnd.manager, &self.dnd.device) else {
            return Err("the compositor has no data devices");
        };
        let source = manager.create_drag_and_drop_source(&self.handle, kinds, actions);
        let picture = start
            .image
            .as_ref()
            .filter(|image| image.is_sound())
            .and_then(|image| self.picture(image));
        source.start_drag(
            device,
            &origin,
            picture.as_ref().map(|picture| &picture.surface),
            serial,
        );
        // (A surface is a drag's picture from `start_drag` on; what it
        // shows is taken up then.)
        if let Some(picture) = &picture {
            picture.surface.commit();
        }
        self.dnd.out = Some(Out {
            from,
            source,
            payload: start.payload,
            action: None,
            _picture: picture,
        });
        // The pointer is the drag's from here on: the compositor tells
        // the surface that it left, and never that the button was let go.
        let held = std::mem::take(&mut self.dnd.held);
        let touched = self.fingers.first().is_some_and(|(_, on, _)| *on == from);
        self.fingers.retain(|(_, on, _)| *on != from);
        if self.pointer_on == Some(from) {
            self.pointer_on = None;
        }
        let modifiers = self.modifiers;
        if let Some(surface) = self.by_id(Some(from)) {
            let at = surface
                .context
                .input(|input| input.pointer.latest_pos())
                .unwrap_or_default();
            let mut let_go = held;
            if touched && !let_go.contains(&egui::PointerButton::Primary) {
                let_go.push(egui::PointerButton::Primary);
            }
            for button in let_go {
                surface.events.push(egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed: false,
                    modifiers,
                });
            }
            surface.events.push(egui::Event::PointerGone);
            surface.redraw = true;
        }
        Ok(())
    }

    /// A surface with a drag's picture on it, ready to be shown once it
    /// is the drag's. Nothing where the compositor shares no memory.
    fn picture(&self, image: &DragImage) -> Option<Picture> {
        let shm = self.dnd.shm.as_ref()?;
        let (width, height) = (image.width as i32, image.height as i32);
        let stride = width.checked_mul(4)?;
        let length = (stride as usize).checked_mul(image.height as usize)?;
        let mut pool = SlotPool::new(length, shm).ok()?;
        let (buffer, canvas) = pool
            .create_buffer(width, height, stride, wl_shm::Format::Argb8888)
            .ok()?;
        let pixels = dnd::premultiplied_bgra(&image.rgba);
        canvas.get_mut(..pixels.len())?.copy_from_slice(&pixels);
        let surface = self.compositor.create_surface(&self.handle);
        buffer.attach_to(&surface).ok()?;
        surface.damage_buffer(0, 0, width, height);
        // Made for a display's pixels, shown at its logical size: exactly
        // where the compositor stretches pictures, to the nearest whole
        // scale where it does not.
        let viewport = self.viewporter.as_ref().map(|viewporter| {
            let viewport = viewporter.get_viewport(&surface, &self.handle, ());
            let (wide, high) = image.logical_size();
            viewport.set_destination(wide, high);
            viewport
        });
        if viewport.is_none() {
            surface.set_buffer_scale((image.scale.round() as i32).max(1));
        }
        // The pointer's tip is on the picture's hotspot. (An older
        // compositor cannot be told: there it is on its corner.)
        if surface.version() >= 5 {
            let (x, y) = image.logical_hotspot();
            surface.offset(-x, -y);
        }
        Some(Picture {
            surface,
            viewport,
            _buffer: buffer,
            _pool: pool,
        })
    }

    /// The drag a surface began is over.
    fn end_drag(&mut self, source: &WlDataSource, outcome: impl FnOnce(&Out) -> DragOutcome) {
        if self
            .dnd
            .out
            .as_ref()
            .is_none_or(|out| out.source.inner() != source)
        {
            return;
        }
        let Some(out) = self.dnd.out.take() else {
            return;
        };
        let outcome = outcome(&out);
        self.drag_news(out.from, DragEvent::Ended { outcome });
    }
}

impl DataDeviceHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        x: f64,
        y: f64,
        wl_surface: &WlSurface,
    ) {
        self.dnd.over = None;
        let Some(id) = self
            .index_of(wl_surface)
            .map(|index| self.surfaces[index].id)
        else {
            return;
        };
        // (A drag with nothing to offer is one the program keeps to
        // itself; there is nothing to take.)
        let Some(offer) = self.dnd.offer() else {
            return;
        };
        if self.dnd.standing(id).is_none() {
            // Not a surface to drop on: the source shows that.
            self.refuse_offer();
            return;
        }
        let kinds = offer.with_mime_types(<[String]>::to_vec);
        let allowed = DragActions::from_wayland(offer.source_actions.bits());
        // (Places come in the surface's own logical pixels, whatever the
        // display's scale: they are egui's points as they are.)
        let at = egui::pos2(x as f32, y as f32);
        self.drag_news(
            id,
            DragEvent::Entered {
                at,
                offered: Offered {
                    files: mime::has_files(&kinds),
                    kinds: kinds.clone(),
                    allowed,
                    payload: None,
                },
            },
        );
        self.dnd.over = Some(Over {
            on: id,
            kinds,
            allowed,
            at,
            now: None,
            chosen: action_of(offer.selected_action),
            said: None,
        });
        self.tell_offer();
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        // (After a drop there is nothing over the surface any more, and
        // the drop is what it is told.)
        if let Some(over) = self.dnd.over.take() {
            self.drag_news(over.on, DragEvent::Left);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, x: f64, y: f64) {
        let at = egui::pos2(x as f32, y as f32);
        let Some(over) = self.dnd.over.as_mut() else {
            return;
        };
        over.at = at;
        let on = over.on;
        self.drag_news(on, DragEvent::Moved { at });
    }

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        // What was copied is not the shell's surfaces' business.
    }

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let Some(over) = self.dnd.over.take() else {
            return;
        };
        let Some(offer) = self.dnd.offer() else {
            return;
        };
        // What the compositor chose is what is done. (One that chooses
        // nothing because it is too old to choose lets everything be
        // copied.)
        let action = over.chosen.or_else(|| {
            (offer.inner().version() < 3 && over.said.is_some_and(|said| said.0.is_some()))
                .then_some(DragAction::Copy)
        });
        let kind = over.said.and_then(|said| said.0);
        let asked = match (action, kind) {
            (Some(action), Some(kind)) => offer
                .receive(kind.to_string())
                .ok()
                .map(|pipe| (action, kind, pipe)),
            _ => None,
        };
        let Some((action, kind, pipe)) = asked else {
            // Let go where it is not taken: over, as if it had left.
            offer.destroy();
            self.drag_news(over.on, DragEvent::Left);
            return;
        };
        self.dnd.reads += 1;
        let read = self.dnd.reads;
        read_pipe(pipe, read, self.dnd.read.clone());
        self.dnd.waiting.push(Waiting {
            read,
            on: over.on,
            at: over.at,
            kind,
            allowed: over.allowed,
            action,
            offer,
            until: Instant::now() + PATIENCE,
        });
    }
}

impl DataOfferHandler for Host {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        offer: &mut DragOffer,
        actions: DndAction,
    ) {
        // The source allows something else now (a key was pressed in it).
        if let Some(over) = self.dnd.over.as_mut() {
            over.allowed = DragActions::from_wayland(actions.bits());
            self.tell_offer_of(offer);
        }
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        action: DndAction,
    ) {
        if let Some(over) = self.dnd.over.as_mut() {
            over.chosen = action_of(action);
        }
    }
}

impl DataSourceHandler for Host {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        kind: String,
        pipe: WritePipe,
    ) {
        let Some(out) = self
            .dnd
            .out
            .as_ref()
            .filter(|out| out.source.inner() == source)
        else {
            return;
        };
        // (Asked for a kind that was not offered: the pipe is closed
        // with nothing in it.)
        if let Some(bytes) = mime::bytes(&out.payload, &kind, PathStyle::Unix) {
            write_pipe(pipe, bytes);
        }
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.end_drag(source, |_| DragOutcome::Cancelled);
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        // Let go over a target that takes it: over when the target says
        // it has what it wanted (`dnd_finished`), or gives up.
    }

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        // A target that was given addresses of files does with the files
        // what was agreed, a move too: nothing is left to the source.
        self.end_drag(source, |out| DragOutcome::Taken {
            action: out.action.unwrap_or(DragAction::Copy),
            remove: false,
        });
    }

    fn action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        action: DndAction,
    ) {
        if let Some(out) = self
            .dnd
            .out
            .as_mut()
            .filter(|out| out.source.inner() == source)
        {
            out.action = action_of(action);
        }
    }
}

impl ShmHandler for Host {
    fn shm_state(&mut self) -> &mut Shm {
        self.dnd
            .shm
            .as_mut()
            .expect("asked only by what the shared memory itself sends")
    }
}

smithay_client_toolkit::delegate_data_device!(Host);
smithay_client_toolkit::delegate_shm!(Host);
