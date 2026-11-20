//! Borderless tool windows with egui on them.
//!
//! The Windows half of the host, in the shape of the Linux half
//! ([`crate::layer`]): a [`Shell`] is one thread's message loop and one
//! graphics device, with any number of surfaces on it, each a window of
//! its own drawn by its own function. Surfaces are opened and closed
//! while it runs: by the drawing of another surface, or by a message
//! from outside (another thread that listens to the window manager),
//! through [`Control`].
//!
//! What a [`Place`] asks for, in Windows' terms:
//!
//! - a window without a frame that is in neither the taskbar nor the
//!   window switcher, of a class of its own whose name begins with
//!   `Gevrek_Shell` and ends with the surface's name ([`place::class_of`]):
//!   the window manager leaves such windows alone;
//! - `reserve`: the window is an application bar, so maximised windows
//!   and the window manager's own keep clear of it;
//! - `level`: above every window, or below every window. Windows has no
//!   place behind the desktop's icons for a program's window, so
//!   `Background` and `Below` are the same here;
//! - `keyboard`: `Never` is a window that a click does not make the
//!   active one; `Exclusive` takes the keyboard when the surface opens,
//!   and gives it back to the window that had it when the surface closes
//!   with the keyboard still its own. Windows lets no window keep the
//!   keyboard from the others: a click elsewhere takes it away, the
//!   drawing hears of that (egui's `focused`), and what then becomes of
//!   the surface is its own business (a launcher closes).
//!
//! A surface can also be put at a rectangle that someone else decided on
//! ([`Control::open_at`], [`Control::put_at`]): the panel, whose room the
//! window manager keeps and names. Such a surface is no application bar.
//!
//! Sizes are in logical pixels and follow the scale of the display the
//! surface is on, fractions included. A frame is drawn when something
//! happened on a surface, when egui asks for another, and never
//! otherwise: the loop sleeps in between, and a still panel costs nothing.
//!
//! A surface that asks for it takes what other programs drag onto it and
//! can drag files out ([`dnd`], and [`crate::dnd`] for what both systems
//! share).
//!
//! Not here: input methods (composed text arrives only once it is
//! committed), the clipboard, touch, and the session lock, which is the
//! system's own on Windows.

pub mod dnd;
pub mod input;
pub mod look;
pub mod place;

use crate::dnd::{DragActions, DragEvent, DragImage, DragOutcome, DragStart, DropAnswer};
use crate::look::BlurRegion;
use crate::{Edge, Flow, Keyboard, Level, Place, Timings};
use anyhow::Context as _;
use egui_wgpu::wgpu;
use place::{Area, Side};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    BOOL, COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, ValidateRect, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SendInput, SetCapture, SetFocus, TrackMouseEvent, INPUT,
    INPUT_MOUSE, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_ESCAPE, VK_F4, VK_LBUTTON, VK_MBUTTON,
    VK_MENU, VK_RBUTTON, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABM_ACTIVATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    ABM_WINDOWPOSCHANGED, ABN_FULLSCREENAPP, ABN_POSCHANGED, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageTime, GetSystemMetrics, GetWindowLongPtrW, IsWindow, LoadCursorW, MsgWaitForMultipleObjectsEx,
    PeekMessageW, PostThreadMessageW, RegisterClassExW, SetCursor, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow,
    TranslateMessage, UnhookWindowsHookEx, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, HCURSOR, HHOOK,
    HTCLIENT, HWND_BOTTOM, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
    HWND_TOPMOST, KBDLLHOOKSTRUCT, LWA_ALPHA, MA_NOACTIVATE, MSG, MSLLHOOKSTRUCT,
    MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SW_SHOWNOACTIVATE, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOWPOS, WINEVENT_OUTOFCONTEXT,
    WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR,
    WM_SETFOCUS, WM_SETTINGCHANGE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_WINDOWPOSCHANGED,
    WM_WINDOWPOSCHANGING, WM_XBUTTONDBLCLK, WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

/// What draws a surface. It is given the surface's root `Ui` and a way to
/// ask for other things to happen once the frame is done.
pub type Draw = Box<dyn FnMut(&mut egui::Ui, &mut Control) -> Flow>;

/// The name a surface is known by while the shell runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceId(u32);

/// What a drawing, or a message's handler, asks of the shell. It is
/// carried out when the handler returns.
/// What a popup is for: a menu takes the pointer (a press outside closes
/// it); a hint is only read, under (or over) what it is about, or
/// beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Popped {
    Menu,
    /// A menu that opens from a line of another: beside that line.
    Sub,
    Hint { beside: bool },
}

#[derive(Default)]
pub struct Control {
    open: Vec<(SurfaceId, Place, Draw)>,
    close: Vec<SurfaceId>,
    redraw: Vec<SurfaceId>,
    resize: Vec<(SurfaceId, u32, u32)>,
    popups: Vec<(SurfaceId, SurfaceId, egui::Rect, egui::Vec2, Popped, Draw)>,
    put: Vec<(SurfaceId, Area)>,
    quit: bool,
    next_id: u32,
    known: Vec<SurfaceId>,
    /// The surfaces that take the keyboard only when pressed, beside
    /// which a mouse button was pressed elsewhere on the desktop since
    /// they last asked (`pressed_outside`).
    outside: Vec<SurfaceId>,
    /// The buttons that went down on surfaces, with the system's time of
    /// each, until they are asked for (`presses`); only the last few are
    /// kept, for the surfaces that never ask.
    pressed: Vec<(SurfaceId, crate::Press)>,
    /// Surfaces that begin or cease to take drops, with what they take
    /// of any drag.
    drops: Vec<(SurfaceId, Option<DropAnswer>)>,
    /// What a surface would do with the drag that is over it now.
    answers: Vec<(SurfaceId, DropAnswer)>,
    /// Drags to begin.
    drags: Vec<(SurfaceId, Dragged)>,
    /// What happened to the surfaces' drags and is not asked for yet.
    drag_news: Vec<(SurfaceId, DragEvent)>,
    /// What was last dropped on a surface, as Windows handed it over.
    dropped: Vec<(SurfaceId, IDataObject)>,
    /// The window of each surface that is open, as Windows names it.
    windows: Vec<(SurfaceId, isize)>,
    /// Surfaces to be given another level among the windows.
    levels: Vec<(SurfaceId, Level)>,
    /// Surfaces that are to take the keyboard another way.
    keyboards: Vec<(SurfaceId, Keyboard)>,
    /// The parts of surfaces behind which the desktop is shown soft.
    blurs: Vec<(SurfaceId, Vec<BlurRegion>)>,
    /// Surfaces to be left out of pictures of the screen, or no longer.
    captures: Vec<(SurfaceId, bool)>,
    /// This Windows shows the desktop soft behind a window.
    blur: bool,
}

/// What a drag out of a surface carries.
enum Dragged {
    /// Paths and text: an object is made for them.
    Plain(DragStart),
    /// An object someone else made.
    Object {
        object: IDataObject,
        allowed: DragActions,
        image: Option<DragImage>,
    },
}

impl Control {
    /// Open a surface; it is there after this handler returns.
    pub fn open(
        &mut self,
        place: Place,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.open.push((id, place, Box::new(draw)));
        id
    }

    /// Open a surface at a rectangle of the desktop, in the pixels of the
    /// screen, that someone else decided on: the window manager, which
    /// keeps the room of the panel free itself and says where it is. Of
    /// `place` the name, the level and the keyboard count; the surface
    /// keeps no room free (who gave the rectangle does), and it is on the
    /// display that holds most of the rectangle. Windows only: a Wayland
    /// compositor places a layer surface itself.
    pub fn open_at(
        &mut self,
        place: Place,
        area: Area,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        let id = self.open(place, draw);
        self.put.push((id, area));
        id
    }

    /// Give a surface another rectangle of the desktop (see
    /// [`Control::open_at`]); from then on it stays where it is put.
    pub fn put_at(&mut self, id: SurfaceId, area: Area) {
        self.put.push((id, area));
    }

    /// Open a surface that belongs to another and may reach beyond it (a
    /// menu under a button of a panel): `size` large, next to `anchor`
    /// (a rectangle on the parent, in its own points), where the display
    /// has room. A press outside the shell's surfaces closes it, and so
    /// does Escape.
    pub fn popup(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.popups
            .push((id, parent, anchor, size, Popped::Menu, Box::new(draw)));
        id
    }

    /// Open a menu that belongs to a line of another menu (or of any
    /// surface): as [`Control::popup`], but beside `anchor`, their tops in
    /// line; on the other side of it where the display ends, and slid
    /// along where it would leave the display below. The surface it
    /// belongs to stays open, and may itself be a popup.
    pub fn submenu(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.popups
            .push((id, parent, anchor, size, Popped::Sub, Box::new(draw)));
        id
    }

    /// The buttons that went down on a surface since this was last asked
    /// for it, each with the system's own time of the press (see
    /// [`crate::Press`]). The same presses reach the drawing through egui;
    /// these say when they were made.
    pub fn presses(&mut self, id: SurfaceId) -> Vec<crate::Press> {
        let mut mine = Vec::new();
        self.pressed.retain(|(on, press)| {
            if *on == id {
                mine.push(*press);
            }
            *on != id
        });
        mine
    }

    /// Open a hint: as [`Control::popup`], but only to be read. It takes
    /// neither the pointer nor a click (what is under it gets them), and
    /// stays until it is closed. `beside`: next to the anchor (for a
    /// panel that goes down a side) and not under it.
    pub fn hint(
        &mut self,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        beside: bool,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        self.next_id += 1;
        let id = SurfaceId(self.next_id);
        self.popups.push((
            id,
            parent,
            anchor,
            size,
            Popped::Hint { beside },
            Box::new(draw),
        ));
        id
    }

    pub fn close(&mut self, id: SurfaceId) {
        self.close.push(id);
    }

    /// Draw a surface again (what it shows changed from outside).
    pub fn redraw(&mut self, id: SurfaceId) {
        self.redraw.push(id);
    }

    /// Give a surface another size (logical pixels).
    pub fn resize(&mut self, id: SurfaceId, width: u32, height: u32) {
        self.resize.push((id, width, height));
    }

    /// Draw every surface again.
    pub fn redraw_all(&mut self) {
        let all = self.known.clone();
        self.redraw.extend(all);
    }

    /// Whether a mouse button was pressed outside the shell's surfaces
    /// since this was last asked for the surface: told to surfaces that
    /// take the keyboard only when they are pressed (`Keyboard::OnDemand`),
    /// which stand beside the windows without holding the keyboard and
    /// are put away by such a press.
    pub fn pressed_outside(&mut self, id: SurfaceId) -> bool {
        let before = self.outside.len();
        self.outside.retain(|pressed| *pressed != id);
        self.outside.len() != before
    }

    /// Let other programs' drags be dropped on a surface, or no longer
    /// (`None`). `standing` is what the surface takes of any drag until
    /// it says more of the one that is over it ([`Control::answer_drag`]):
    /// the program a drag comes from shows its pointer by it from the
    /// first moment. What happens is asked for with
    /// [`Control::drag_events`]. A surface that never calls this is told
    /// nothing and is no place to drop on.
    pub fn accept_drops(&mut self, id: SurfaceId, standing: Option<DropAnswer>) {
        self.drops.push((id, standing));
    }

    /// What the surface would do with the drag that is over it, where it
    /// is now: said by the drawing when it was told of a move, and held
    /// until it says otherwise or the drag is gone. (The source's pointer
    /// follows a frame later.)
    pub fn answer_drag(&mut self, id: SurfaceId, answer: DropAnswer) {
        self.answers.push((id, answer));
    }

    /// What happened to drags over the surface, and to the drag it began,
    /// since this was last asked; the surface is drawn again whenever
    /// there is something.
    pub fn drag_events(&mut self, id: SurfaceId) -> Vec<DragEvent> {
        let mut mine = Vec::new();
        let mut index = 0;
        while index < self.drag_news.len() {
            if self.drag_news[index].0 == id {
                mine.push(self.drag_news.remove(index).1);
            } else {
                index += 1;
            }
        }
        mine
    }

    /// Begin a drag of files (or text) out of a surface, while the mouse
    /// button that will drop it is held on it. The surface is told that
    /// the button was let go and the pointer is gone (it is the drag's
    /// from here on), and, when the drag is over, how it ended
    /// ([`DragEvent::Ended`]).
    pub fn start_drag(&mut self, from: SurfaceId, drag: DragStart) {
        self.drags.push((from, Dragged::Plain(drag)));
    }

    /// As [`Control::start_drag`], with an object made elsewhere: things
    /// of Windows' shell that are no files (the zones' backend makes
    /// those). Windows only.
    pub fn start_drag_of(
        &mut self,
        from: SurfaceId,
        object: IDataObject,
        allowed: DragActions,
        image: Option<DragImage>,
    ) {
        self.drags.push((
            from,
            Dragged::Object {
                object,
                allowed,
                image,
            },
        ));
    }

    /// What was last dropped on a surface, as Windows handed it over: for
    /// what the payload of [`DragEvent::Dropped`] cannot say (things
    /// without a path). There from the drop until it is taken or the next
    /// drag comes. Windows only.
    pub fn dropped_object(&mut self, id: SurfaceId) -> Option<IDataObject> {
        let at = self.dropped.iter().position(|(on, _)| *on == id)?;
        Some(self.dropped.remove(at).1)
    }

    /// The window a surface is drawn on, as Windows names it (an `HWND`,
    /// as a number). For what Windows itself shows on a surface's behalf
    /// and wants an owner for: the shell's own menu for a file, and the
    /// windows that tell how far copying or moving has come. Nothing for a
    /// surface that has no window yet (asked for in this pass, it has one
    /// from the next) or is closed. The number names the window only
    /// while the surface is open: it is asked for when needed and not
    /// kept, since Windows gives a closed window's number to another.
    /// Windows only: a Wayland surface has nothing of the kind.
    pub fn native_window(&self, id: SurfaceId) -> Option<isize> {
        if self.close.contains(&id) {
            return None;
        }
        self.windows
            .iter()
            .find(|(open, _)| *open == id)
            .map(|(_, window)| *window)
    }

    /// Put an open surface on another level among the windows, without
    /// closing it: below them, above them, over everything, and back
    /// (what lies below the windows and is raised over them to be looked
    /// at for a moment). Its place, its size and what is drawn on it stay.
    /// Not to or from [`Level::Marks`], which a surface is from the start
    /// or never: such a change is left undone and said in the log.
    pub fn set_level(&mut self, id: SurfaceId, level: Level) {
        self.levels.push((id, level));
    }

    /// Change how an open surface takes the keyboard. To
    /// [`Keyboard::Exclusive`]: it has the keyboard from now (as when it
    /// opens so), and the window that had it is remembered; away from it:
    /// that window has the keyboard again, if the surface still had it.
    /// To [`Keyboard::Never`]: a press on it no longer takes the keyboard
    /// (if it has it now, it keeps it until something else is pressed).
    pub fn set_keyboard(&mut self, id: SurfaceId, keyboard: Keyboard) {
        self.keyboards.push((id, keyboard));
    }

    /// Show the desktop soft (blurred) behind these parts of a surface
    /// and nowhere else behind it; an empty list ends it. The parts are
    /// in the surface's own points and are said again whenever they
    /// change (a ground moved, sized, added, taken away); what was said
    /// last holds. The surface must draw something see-through there, or
    /// nothing of it shows. See [`crate::look`], and
    /// [`Control::blur_available`] for when nothing will be shown.
    ///
    /// On Windows each part is the system's material for a passing window
    /// with the system's own round corners (eight pixels, four, or none:
    /// the nearest to `radius`), on a window of its own under the surface
    /// ([`look`]).
    pub fn set_blur_regions(&mut self, id: SurfaceId, regions: &[BlurRegion]) {
        self.blurs.retain(|(known, _)| *known != id);
        self.blurs.push((id, regions.to_vec()));
    }

    /// Whether [`Control::set_blur_regions`] shows anything here. Where it
    /// does not (a Windows before 11 22H2), a drawing makes its grounds
    /// solid enough to be read on by themselves: a plain see-through
    /// ground is what is left.
    pub fn blur_available(&self) -> bool {
        self.blur
    }

    /// Leave a surface out of pictures taken of the screen (screenshots,
    /// recordings, a screen that is shared), or no longer: it is seen on
    /// the display and missing in the picture, with what is behind it in
    /// its place. Windows 10 of 2020 and later; before that the surface
    /// is black in such pictures.
    pub fn exclude_from_capture(&mut self, id: SurfaceId, exclude: bool) {
        self.captures.retain(|(known, _)| *known != id);
        self.captures.push((id, exclude));
    }

    /// Whether a surface is open now.
    pub fn is_open(&self, id: SurfaceId) -> bool {
        // (Also one asked for earlier in this pass, which is made when
        // the pass is over: asked for a second time it would be made
        // twice, and the first never closed.)
        let asked_for = self.open.iter().any(|(asked, ..)| *asked == id)
            || self.popups.iter().any(|(asked, ..)| *asked == id);
        (self.known.contains(&id) || asked_for) && !self.close.contains(&id)
    }

    /// End the shell: every surface is closed and `run` returns.
    pub fn quit(&mut self) {
        self.quit = true;
    }
}

/// A way to send the running shell a message from another thread.
pub struct Sender<M> {
    channel: std::sync::mpsc::Sender<M>,
    /// The thread whose loop is woken to read the message.
    thread: u32,
}

impl<M> Clone for Sender<M> {
    fn clone(&self) -> Self {
        Self {
            channel: self.channel.clone(),
            thread: self.thread,
        }
    }
}

impl<M> Sender<M> {
    /// `false` when the shell is gone.
    pub fn send(&self, message: M) -> bool {
        if self.channel.send(message).is_err() {
            return false;
        }
        // The loop sleeps until Windows has something for it: this is
        // something. (Lost when the loop is gone, as the message is.)
        let _ = unsafe { PostThreadMessageW(self.thread, WAKE, WPARAM(0), LPARAM(0)) };
        true
    }
}

/// One thread's windows with surfaces on them.
pub struct Shell<M: 'static> {
    host: Host,
    messages: std::sync::mpsc::Receiver<M>,
    sender: Sender<M>,
    /// What the thread is busy with, said where it is for long.
    watch: crate::stall::Watch,
}

impl<M: 'static> Shell<M> {
    pub fn new() -> anyhow::Result<Self> {
        let started = Instant::now();
        // Sizes are this program's business on every display: without
        // this, Windows stretches the picture on a scaled one. (For the
        // whole program where nothing has decided yet, and for this
        // thread's windows in any case.)
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        // Direct3D, with the picture given to the desktop's compositor
        // as a layer of its own: only so can a surface be seen through
        // where nothing is drawn (rounded corners, a notification's
        // shadow). The environment can ask for something else
        // (`WGPU_BACKEND`, `WGPU_DX12_PRESENTATION_SYSTEM`), and then
        // surfaces are opaque.
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        descriptor.backend_options.dx12.presentation_system =
            wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let descriptor = descriptor.with_env();
        let see_through = descriptor.backends == wgpu::Backends::DX12
            && descriptor.backend_options.dx12.presentation_system
                == wgpu::Dx12SwapchainKind::DxgiFromVisual;
        let module = unsafe { GetModuleHandleW(None) }.context("this program's module")?;
        let mut host = Host {
            surfaces: Vec::new(),
            gpu: None,
            instance: wgpu::Instance::new(descriptor),
            see_through,
            module: module.into(),
            classes: Vec::new(),
            watcher: None,
            hooks: None,
            pointer_on: None,
            displays_changed: false,
            covered_looked: None,
            started,
            timings: Timings::default(),
            total_ms: 0.0,
            control: Control::default(),
            ole: None,
            double_click: double_click_setting(),
        };
        // A window nobody sees, to be told when displays come, go or
        // change (such news is for top-level windows only), and so that
        // this thread can be sent messages from the start.
        let class = host.class(place::CLASS_PREFIX)?;
        let watcher = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(class.as_ptr()),
                PCWSTR(class.as_ptr()),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                host.module,
                None,
            )
        }
        .context("the window that watches the displays")?;
        host.control.blur = look::supported(watcher);
        host.watcher = Some(Window {
            handle: watcher,
            reserved: false,
            accepts: false,
        });
        let (channel, messages) = std::sync::mpsc::channel();
        Ok(Self {
            host,
            messages,
            sender: Sender {
                channel,
                thread: unsafe { GetCurrentThreadId() },
            },
            watch: crate::stall::Watch::start(),
        })
    }

    /// For another thread to talk to the shell with.
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    /// Open a surface before the shell runs.
    pub fn open(
        &mut self,
        place: Place,
        draw: impl FnMut(&mut egui::Ui, &mut Control) -> Flow + 'static,
    ) -> SurfaceId {
        let id = self.host.control.open(place, draw);
        self.host.carry_out();
        id
    }

    /// Run until a handler asks to quit. `on_message` gets every message
    /// sent through a [`Sender`]. With `until_empty`, the shell also ends
    /// when its last surface is closed.
    pub fn run(
        self,
        until_empty: bool,
        on_message: impl FnMut(M, &mut Control),
    ) -> anyhow::Result<Timings> {
        self.run_with_outputs(until_empty, on_message, |_, _| {})
    }

    /// As [`Shell::run`]; `on_outputs` is told the names of the displays
    /// when the shell starts and each time one comes or goes.
    pub fn run_with_outputs(
        mut self,
        until_empty: bool,
        mut on_message: impl FnMut(M, &mut Control),
        mut on_outputs: impl FnMut(&[String], &mut Control),
    ) -> anyhow::Result<Timings> {
        let mut known: Option<Vec<String>> = None;
        self.host.displays_changed = true;
        loop {
            if std::mem::take(&mut self.host.displays_changed) {
                // Whatever changed, every surface finds its place again;
                // those of a display that is gone are closed.
                self.host.settle_all();
                let mut names: Vec<String> = displays()
                    .iter()
                    .map(|display| place::display_name(&display.device).to_string())
                    .collect();
                names.sort();
                if known.as_ref() != Some(&names) {
                    on_outputs(&names, &mut self.host.control);
                    known = Some(names);
                }
                self.host.carry_out();
            }
            if self.host.control.quit || (until_empty && self.host.surfaces.is_empty()) {
                break;
            }
            // Sleep until Windows has a message, another thread has one,
            // or a surface is due to be drawn again.
            let now = Instant::now();
            // What a window was told while the loop itself worked on it
            // (shown, it is told at once that it has the keyboard) waits
            // for no message: it is there already.
            let heard = HEARD.with(|heard| !heard.borrow().is_empty());
            let wait = if heard || self.host.surfaces.iter().any(|surface| surface.redraw) {
                0
            } else {
                let drawn_again = self
                    .host
                    .surfaces
                    .iter()
                    .filter_map(|surface| surface.again_at)
                    .min()
                    .map_or(u32::MAX, |at| {
                        // Rounded up: woken a moment early, the loop
                        // would go round with nothing to do.
                        let left = at.saturating_duration_since(now);
                        (left.as_micros().div_ceil(1000)).min(u128::from(u32::MAX - 1)) as u32
                    });
                // (A bar that stepped down is looked at again without
                // any message: `look_at_the_covered`.)
                if self.host.surfaces.iter().any(|surface| surface.covered) {
                    drawn_again.min(COVERED_LOOK.as_millis() as u32)
                } else {
                    drawn_again
                }
            };
            unsafe {
                MsgWaitForMultipleObjectsEx(None, wait, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                self.watch.enter(crate::stall::Part::Events);
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    // Makes the characters of the keys that write some.
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                self.watch.leave();
            }
            turn(&mut self.host, &self.messages, &mut on_message, &self.watch);
            // A drag out of a surface: Windows runs it in a loop of its
            // own and returns when it is over. The shell's own pass is
            // made from inside that loop meanwhile.
            while let Some((from, dragged)) = self.host.next_drag() {
                let Some(ready) = self.host.begin_drag(from, dragged) else {
                    continue;
                };
                let (host, messages, watch) = (&mut self.host, &self.messages, &self.watch);
                let outcome =
                    dnd::run(&ready.object, ready.allowed, ready.image.as_ref(), &mut || {
                        turn(host, messages, &mut on_message, watch)
                    });
                self.host.end_drag(from, outcome);
            }
        }
        // Surfaces first: they draw with the device.
        self.host.hooks = None;
        self.host.surfaces.clear();
        if self.host.timings.frames > 0 {
            self.host.timings.mean_ms = self.host.total_ms / self.host.timings.frames as f64;
        }
        Ok(self.host.timings)
    }
}

/// One pass of the shell: what the windows were told becomes input, the
/// messages of other threads are handled, and every surface that is due
/// is drawn. Made by the loop after Windows' messages, and from inside a
/// drag (which has a loop of its own, see [`dnd::run`]).
fn turn<M>(
    host: &mut Host,
    messages: &std::sync::mpsc::Receiver<M>,
    on_message: &mut impl FnMut(M, &mut Control),
    watch: &crate::stall::Watch,
) {
    use crate::stall::Part;
    // What the windows were told, in the order it came.
    let heard = HEARD.with(|heard| {
        heard
            .try_borrow_mut()
            .map(|mut heard| std::mem::take(&mut *heard))
            .unwrap_or_default()
    });
    watch.during(Part::Events, || {
        for news in heard {
            host.take_in(news);
        }
        host.take_in_drags();
    });
    watch.during(Part::Messages, || {
        for message in messages.try_iter() {
            on_message(message, &mut host.control);
        }
    });
    watch.during(Part::CarryOut, || host.carry_out());
    watch.during(Part::Access, || host.access_news());
    watch.during(Part::Other("looking at the covered surfaces"), || host.look_at_the_covered());
    let now = Instant::now();
    for index in 0..host.surfaces.len() {
        let surface = &mut host.surfaces[index];
        if surface.again_at.is_some_and(|at| now >= at) {
            surface.again_at = None;
            surface.redraw = true;
        }
        if surface.redraw {
            let (width, height) = surface.configured;
            let size = (width.max(0) as u32, height.max(0) as u32);
            let part = Part::Draw { surface: surface.id.0, size };
            watch.during(part, || host.draw(index));
        }
    }
    watch.during(Part::CarryOut, || host.carry_out());
}

/// Show one surface at `place` and draw `ui` on it until it says to close
/// (or Windows closes it).
pub fn run(
    place: Place,
    mut ui: impl FnMut(&mut egui::Ui) -> Flow + 'static,
) -> anyhow::Result<Timings> {
    let mut shell: Shell<()> = Shell::new()?;
    shell.open(place, move |root, _| ui(root));
    shell.run(true, |(), _| {})
}

/// The message that wakes the loop for another thread's message.
const WAKE: u32 = WM_APP + 1;
/// What Windows tells an application bar with: the message is ours to
/// choose, the news is in its first number.
const BAR_NEWS: u32 = WM_APP + 2;
/// News that is no window's message: a press of a mouse button anywhere
/// (at these pixels of the screen), Escape, and another window coming to
/// the front, heard while a popup is open.
const PRESSED_AT: u32 = WM_APP + 3;
const ESCAPE: u32 = WM_APP + 4;
const IN_FRONT: u32 = WM_APP + 5;

/// The room between a popup and the rectangle it belongs to, in logical
/// pixels; as on Linux.
const POPUP_GAP: i32 = 4;

/// How far a menu that opens from a line of another lies from that line's
/// end: over the rim of the menu it came from by a little, so that the
/// two read as one thing and the pointer crosses no gap between them.
const SUB_GAP: i32 = -3;

/// How many presses are kept for the surfaces to ask for.
const PRESSES_KEPT: usize = 32;

/// How often a bar that stepped down for a window that covers its display
/// is looked at again (`Host::look_at_the_covered`).
const COVERED_LOOK: Duration = Duration::from_secs(1);

/// The room between a hint beside a panel and what it is about.
const HINT_GAP: i32 = 6;

/// Something a window was told, kept until the loop looks at it: a
/// window's procedure is called in the middle of other things (while a
/// window is made, moved or destroyed), where the host cannot be touched.
#[derive(Debug, Clone, Copy)]
struct News {
    window: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    /// The keys held when it happened.
    modifiers: egui::Modifiers,
    /// When Windows saw what the message tells of, in thousandths of a
    /// second (not when the loop came to it).
    time: u32,
}

/// What a window's procedure must know to answer on the spot.
#[derive(Debug, Clone, Copy, Default)]
struct Facts {
    /// A click makes it the active window.
    activates: bool,
    /// It stays below every other window.
    bottom: bool,
    /// It is an application bar.
    reserved: bool,
    /// The pointer shown over it; 0 is none.
    pointer: isize,
}

thread_local! {
    static HEARD: RefCell<Vec<News>> = const { RefCell::new(Vec::new()) };
    static FACTS: RefCell<HashMap<isize, Facts>> = RefCell::new(HashMap::new());
}

fn hear(window: isize, message: u32, wparam: usize, lparam: isize) {
    let held = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetKeyState(i32::from(key.0)) < 0
    };
    let news = News {
        window,
        message,
        wparam,
        lparam,
        modifiers: input::modifiers(held(VK_SHIFT), held(VK_CONTROL), held(VK_MENU)),
        // SAFETY: a question without arguments. (Asked by a hook and not
        // by a window's procedure, it says the time of this thread's last
        // message: near enough for news that is no press.)
        time: unsafe { GetMessageTime() } as u32,
    };
    HEARD.with(|heard| {
        if let Ok(mut heard) = heard.try_borrow_mut() {
            heard.push(news);
        }
    });
}

fn facts_of(window: HWND) -> Facts {
    FACTS.with(|facts| {
        facts
            .try_borrow()
            .ok()
            .and_then(|facts| facts.get(&(window.0 as isize)).copied())
            .unwrap_or_default()
    })
}

/// The message Windows' own shell sends every top-level window when it
/// has started (again): it has then forgotten every application bar.
fn shell_started() -> u32 {
    static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    // SAFETY: a constant name; the number is the same for every caller.
    *MESSAGE.get_or_init(|| unsafe {
        windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows::core::w!(
            "TaskbarCreated"
        ))
    })
}

fn change_facts(window: HWND, change: impl FnOnce(&mut Facts)) {
    FACTS.with(|facts| {
        if let Ok(mut facts) = facts.try_borrow_mut() {
            change(facts.entry(window.0 as isize).or_default());
        }
    });
}

fn bar_data(window: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: window,
        uCallbackMessage: BAR_NEWS,
        ..Default::default()
    }
}

/// The procedure of every window of the shell: it answers what must be
/// answered at once and keeps the rest for the loop.
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let keep = || hear(window.0 as isize, message, wparam.0, lparam.0);
    match message {
        WM_MOUSEMOVE => {
            // Asked for each time: Windows tells once that the pointer left.
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: window,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut track);
            keep();
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_LBUTTONDBLCLK
        | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK | WM_XBUTTONDBLCLK => {
            // A drag that leaves the surface is still this surface's.
            SetCapture(window);
            keep();
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            // The lower half of the first number: the buttons still down.
            if wparam.0 & 0x73 == 0 {
                let _ = ReleaseCapture();
            }
            keep();
            LRESULT(0)
        }
        WM_MOUSELEAVE | WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_KEYDOWN | WM_KEYUP | WM_CHAR
        | WM_SETFOCUS | WM_KILLFOCUS | WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE
        | BAR_NEWS => {
            keep();
            LRESULT(0)
        }
        told if told != 0 && told == shell_started() => {
            keep();
            LRESULT(0)
        }
        WM_SYSKEYDOWN | WM_SYSKEYUP => {
            keep();
            // Alt and F4 closes, as everywhere; Alt alone would open a
            // menu the window does not have.
            if wparam.0 == usize::from(VK_F4.0) {
                return DefWindowProcW(window, message, wparam, lparam);
            }
            LRESULT(0)
        }
        // A letter typed with Alt: without an answer Windows beeps.
        WM_SYSCHAR => LRESULT(0),
        WM_CLOSE => {
            // Closed by the loop, which owns the window.
            keep();
            LRESULT(0)
        }
        WM_PAINT => {
            let _ = ValidateRect(window, None);
            keep();
            LRESULT(0)
        }
        // Nothing to clear: the picture covers the window.
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEACTIVATE => {
            if facts_of(window).activates {
                DefWindowProcW(window, message, wparam, lparam)
            } else {
                LRESULT(MA_NOACTIVATE as isize)
            }
        }
        WM_SETCURSOR => {
            // The lower half of the second number: the part of the window
            // the pointer is over.
            if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                SetCursor(HCURSOR(facts_of(window).pointer as *mut _));
                LRESULT(1)
            } else {
                DefWindowProcW(window, message, wparam, lparam)
            }
        }
        WM_WINDOWPOSCHANGING => {
            if facts_of(window).bottom {
                // Whatever raises it (a click, another program): it stays
                // below.
                let position = &mut *(lparam.0 as *mut WINDOWPOS);
                position.hwndInsertAfter = HWND_BOTTOM;
                position.flags &= !SWP_NOZORDER;
            }
            LRESULT(0)
        }
        WM_ACTIVATE | WM_WINDOWPOSCHANGED => {
            // Where it stands among the windows may be another: what
            // lies under it to show the desktop soft goes under it again.
            if message == WM_WINDOWPOSCHANGED {
                let position = &*(lparam.0 as *const WINDOWPOS);
                if !position.flags.contains(SWP_NOZORDER) {
                    look::restack_under(window);
                }
            }
            // Windows asks to be told both of an application bar.
            if facts_of(window).reserved {
                let mut data = bar_data(window);
                SHAppBarMessage(
                    if message == WM_ACTIVATE {
                        ABM_ACTIVATE
                    } else {
                        ABM_WINDOWPOSCHANGED
                    },
                    &mut data,
                );
            }
            DefWindowProcW(window, message, wparam, lparam)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

// Not among the messages of the windows crate's module for windows.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// While a popup is open: a press of a mouse button, wherever it is.
unsafe extern "system" fn mouse_heard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0
        && matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        let event = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        hear(0, PRESSED_AT, event.pt.x as usize, event.pt.y as isize);
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// A popup is open: the Escape key closes it, whichever window has the
/// keyboard. (Not so while only a surface beside the windows is open:
/// the key is the window's then.)
static ESCAPE_IS_A_POPUPS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// While a popup is open: Escape, whichever window has the keyboard (a
/// popup never has it). The key goes no further, as with a menu.
unsafe extern "system" fn keys_heard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if event.vkCode == u32::from(VK_ESCAPE.0)
            && ESCAPE_IS_A_POPUPS.load(std::sync::atomic::Ordering::Relaxed)
        {
            if matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
                hear(0, ESCAPE, 0, 0);
            }
            return LRESULT(1);
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// While a popup is open: another window comes to the front.
unsafe extern "system" fn front_heard(
    _: HWINEVENTHOOK,
    _: u32,
    window: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    hear(0, IN_FRONT, window.0 as usize, 0);
}

impl Host {
    /// A bar that stepped down for a window that covers its display comes
    /// up again when no window covers it any more, whether or not Windows
    /// told of it.
    ///
    /// Windows tells a bar when a window covers its display and when that
    /// is over (`ABN_FULLSCREENAPP`), and does not always tell the second:
    /// a window that covered the display and was hidden or closed leaves
    /// the bar told nothing. The bar then stayed below every window, and
    /// under the surface the zones of the desktop are drawn on: a press
    /// on it was a press on the desktop to the window manager, which gave
    /// that display the keyboard, and the bar did nothing until it was
    /// pressed a second time.
    fn look_at_the_covered(&mut self) {
        if !self.surfaces.iter().any(|surface| surface.covered) {
            return;
        }
        let now = Instant::now();
        if self
            .covered_looked
            .is_some_and(|at| now.duration_since(at) < COVERED_LOOK)
        {
            return;
        }
        self.covered_looked = Some(now);
        let all = displays();
        for surface in &mut self.surfaces {
            if !surface.covered {
                continue;
            }
            let Some(display) = all.iter().find(|display| display.device == surface.display)
            else {
                continue;
            };
            if !display_is_covered(display.whole) {
                log::debug!("no window covers the display of {:?} any more", surface.id);
                surface.covered = false;
                surface.restack();
            }
        }
    }

    /// What screen readers asked since the last pass: a surface that is
    /// listened to is described from its next frame on, and what was
    /// asked to be done is input for it.
    fn access_news(&mut self) {
        for surface in &mut self.surfaces {
            let Some(access) = surface.access.as_mut() else {
                continue;
            };
            if access
                .shared
                .wanted
                .load(std::sync::atomic::Ordering::Relaxed)
                && !access.described
            {
                surface.context.enable_accesskit();
                access.described = true;
                surface.redraw = true;
            }
            let asked = std::mem::take(
                &mut *access
                    .shared
                    .asked
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
            for request in asked {
                surface
                    .events
                    .push(egui::Event::AccessKitActionRequest(request));
                surface.redraw = true;
            }
        }
    }
}

/// What is listened to while a popup is open, and no longer.
struct Hooks {
    mouse: HHOOK,
    keys: HHOOK,
    front: HWINEVENTHOOK,
}

impl Hooks {
    fn new(module: HINSTANCE) -> Option<Self> {
        unsafe {
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_heard), module, 0).ok()?;
            let keys = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(keys_heard), module, 0) {
                Ok(keys) => keys,
                Err(_) => {
                    let _ = UnhookWindowsHookEx(mouse);
                    return None;
                }
            };
            let front = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(front_heard),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            Some(Self { mouse, keys, front })
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.mouse);
            let _ = UnhookWindowsHookEx(self.keys);
            if !self.front.is_invalid() {
                let _ = UnhookWinEvent(self.front);
            }
        }
    }
}

/// A display as Windows describes it.
#[derive(Debug, Clone, PartialEq)]
struct Display {
    /// Windows' name for it (`\\.\DISPLAY1`).
    device: String,
    /// All of it, and what application bars leave of it.
    whole: Area,
    work: Area,
    scale: f32,
    primary: bool,
    handle: isize,
}

/// Whether a window of a program fills this display now: shown, not
/// minimized, not kept out of sight by the system (`covers_display`).
fn display_is_covered(display: Area) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, IsIconic, IsWindowVisible,
    };
    struct Search {
        display: Area,
        covered: bool,
    }
    unsafe extern "system" fn each(window: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
            return true.into();
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return true.into();
        }
        let mut name = [0u16; 128];
        let length = GetClassNameW(window, &mut name) as usize;
        let class = String::from_utf16_lossy(&name[..length.min(name.len())]);
        if !place::covers_display(&class, area_of(rect), search.display) {
            return true.into();
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        if cloaked != 0 {
            return true.into();
        }
        search.covered = true;
        false.into()
    }
    let mut search = Search {
        display,
        covered: false,
    };
    // SAFETY: the callback runs before the call returns and gets a pointer
    // to `search`, which outlives it; the rest are plain questions about
    // windows that may be gone.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut search as *mut Search as isize));
    }
    search.covered
}

/// Whether the window that has the keyboard is the desktop or one of
/// gevrek's own, which a bar never steps down for.
fn front_is_no_programs() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    // SAFETY: plain questions about a window that may be gone.
    let class = unsafe {
        let front = GetForegroundWindow();
        if front.is_invalid() {
            return false;
        }
        let mut name = [0u16; 128];
        let length = GetClassNameW(front, &mut name) as usize;
        String::from_utf16_lossy(&name[..length.min(name.len())])
    };
    let anywhere = Area::default();
    !place::covers_display(&class, anywhere, anywhere)
}

fn area_of(rect: RECT) -> Area {
    Area::new(
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )
}

fn rect_of(area: Area) -> RECT {
    RECT {
        left: area.x,
        top: area.y,
        right: area.right(),
        bottom: area.bottom(),
    }
}

unsafe extern "system" fn each_display(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    list: LPARAM,
) -> BOOL {
    let list = &mut *(list.0 as *mut Vec<Display>);
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    if GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
        let (mut across, mut down) = (96u32, 96u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut across, &mut down);
        let length = info
            .szDevice
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(info.szDevice.len());
        list.push(Display {
            device: String::from_utf16_lossy(&info.szDevice[..length]),
            whole: area_of(info.monitorInfo.rcMonitor),
            work: area_of(info.monitorInfo.rcWork),
            // 96 dots to the inch is a scale of one.
            scale: across.max(1) as f32 / 96.0,
            // The first of the flags: the main display.
            primary: info.monitorInfo.dwFlags & 1 != 0,
            handle: monitor.0 as isize,
        });
    }
    true.into()
}

/// The displays there are now.
fn displays() -> Vec<Display> {
    let mut list: Vec<Display> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(each_display),
            LPARAM(&mut list as *mut Vec<Display> as isize),
        );
    }
    list
}

/// The display a surface opens on when it names none: where the pointer
/// is.
fn display_at_pointer(all: &[Display]) -> Option<&Display> {
    let mut at = POINT::default();
    let found = unsafe {
        let _ = GetCursorPos(&mut at);
        MonitorFromPoint(at, MONITOR_DEFAULTTOPRIMARY)
    };
    all.iter()
        .find(|display| display.handle == found.0 as isize)
        .or_else(|| all.iter().find(|display| display.primary))
        .or_else(|| all.first())
}

/// A window of the shell; destroyed with this.
struct Window {
    handle: HWND,
    /// Registered as an application bar.
    reserved: bool,
    /// Registered as a place to drop on.
    accepts: bool,
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.accepts {
            // (Before the window goes: Windows holds the target for it.)
            dnd::revoke(self.handle);
        }
        unsafe {
            if self.reserved {
                // The room it kept is given back.
                let mut data = bar_data(self.handle);
                SHAppBarMessage(ABM_REMOVE, &mut data);
            }
            FACTS.with(|facts| {
                if let Ok(mut facts) = facts.try_borrow_mut() {
                    facts.remove(&(self.handle.0 as isize));
                }
            });
            let _ = DestroyWindow(self.handle);
        }
    }
}

/// The graphics device every surface draws with.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: wgpu::Adapter,
}

/// What a surface is on the desktop.
enum Role {
    /// At a place of its own.
    Placed(Place),
    /// A popup: the surface it belongs to, the rectangle on that surface
    /// it lies next to and its size, both in points; and what it is for.
    Popup(SurfaceId, egui::Rect, egui::Vec2, Popped),
}

/// One surface and what is drawn on it.
/// A surface as screen readers and other aids know it (UI Automation,
/// through AccessKit): see the same on Linux (`layer.rs`). Nothing is
/// described until somebody listens.
struct Access {
    adapter: accesskit_windows::SubclassingAdapter,
    shared: std::sync::Arc<AccessShared>,
    /// The toolkit was told to describe this surface.
    described: bool,
}

struct AccessShared {
    wanted: std::sync::atomic::AtomicBool,
    asked: std::sync::Mutex<Vec<egui::accesskit::ActionRequest>>,
    /// The thread of the loop, to be woken.
    thread: u32,
}

struct AccessHears(std::sync::Arc<AccessShared>);

impl egui::accesskit::ActivationHandler for AccessHears {
    fn request_initial_tree(&mut self) -> Option<egui::accesskit::TreeUpdate> {
        // (The tree comes with the next frame, which the loop's next
        // pass asks for.)
        self.0
            .wanted
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = unsafe { PostThreadMessageW(self.0.thread, WAKE, WPARAM(0), LPARAM(0)) };
        None
    }
}

impl egui::accesskit::ActionHandler for AccessHears {
    fn do_action(&mut self, request: egui::accesskit::ActionRequest) {
        self.0
            .asked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        let _ = unsafe { PostThreadMessageW(self.0.thread, WAKE, WPARAM(0), LPARAM(0)) };
    }
}

impl Access {
    /// For a window that was not shown yet (the bridge takes its place
    /// in the window's handling of messages before the first showing).
    fn new(window: HWND) -> Option<Self> {
        if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(window) }.as_bool() {
            return None;
        }
        let shared = std::sync::Arc::new(AccessShared {
            wanted: std::sync::atomic::AtomicBool::new(false),
            asked: std::sync::Mutex::new(Vec::new()),
            thread: unsafe { GetCurrentThreadId() },
        });
        Some(Self {
            adapter: accesskit_windows::SubclassingAdapter::new(
                accesskit_windows::HWND(window.0 as _),
                AccessHears(std::sync::Arc::clone(&shared)),
                AccessHears(std::sync::Arc::clone(&shared)),
            ),
            shared,
            described: false,
        })
    }
}

struct Surface {
    id: SurfaceId,
    /// What it shows, as screen readers and other aids are told it.
    access: Option<Access>,
    // Dropped first: it draws on the window below.
    target: wgpu::Surface<'static>,
    renderer: egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    alpha: wgpu::CompositeAlphaMode,
    window: Window,
    role: Role,
    /// Windows' name of the display it is on.
    display: String,
    context: egui::Context,
    draw: Draw,
    /// Where it is on the desktop, in pixels.
    area: Area,
    /// Pixels to a logical pixel on its display.
    scale: f32,
    /// The size the picture was last made for.
    configured: (i32, i32),
    /// The last frame had nothing to draw into (said once, when it
    /// begins and when it ends).
    failing: bool,
    /// What happened since the last frame, for egui.
    events: Vec<egui::Event>,
    /// The surface has the keyboard.
    focused: bool,
    redraw: bool,
    /// egui asked to be drawn again at this time.
    again_at: Option<Instant>,
    /// Text that arrives in halves.
    text: input::Utf16,
    /// The pointer egui last asked for.
    pointer: egui::CursorIcon,
    /// A fullscreen window is on its display: a bar steps down.
    covered: bool,
    /// The strip Windows was last asked to keep clear for it.
    kept: Option<Area>,
    /// The rectangle it was put at by who opened it, where it stays.
    pinned: Option<Area>,
    /// The window that had the keyboard before this surface took it; 0:
    /// none, or the surface takes no keyboard.
    before: isize,
    /// The parts behind which the desktop is shown soft, in points, and
    /// the windows under the surface that show it.
    blur: Vec<BlurRegion>,
    backdrops: look::Backdrops,
    /// Left out of pictures of the screen.
    hidden_from_capture: bool,
}

impl Surface {
    fn configure_target(&mut self, device: &wgpu::Device) {
        let (width, height) = (self.area.width, self.area.height);
        if width <= 0 || height <= 0 {
            return;
        }
        self.target.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width: width as u32,
                height: height as u32,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: self.alpha,
                view_formats: Vec::new(),
            },
        );
        if self.configured != (width, height) {
            log::debug!("surface {:?} is drawn at {width}x{height}", self.id);
        }
        self.configured = (width, height);
    }

    /// A menu: a press elsewhere, or Escape, closes it.
    fn is_popup(&self) -> bool {
        matches!(self.role, Role::Popup(.., Popped::Menu | Popped::Sub))
    }

    /// It stands beside the windows and takes the keyboard only when it
    /// is pressed: a press elsewhere on the desktop is told to it.
    fn is_beside(&self) -> bool {
        matches!(&self.role, Role::Placed(place) if place.keyboard == Keyboard::OnDemand)
    }

    /// The surface a popup belongs to.
    fn parent(&self) -> Option<SurfaceId> {
        match &self.role {
            Role::Popup(parent, ..) => Some(*parent),
            Role::Placed(_) => None,
        }
    }

    /// A hint: only read; the pointer goes through it.
    fn is_hint(&self) -> bool {
        matches!(self.role, Role::Popup(.., Popped::Hint { .. }))
    }

    /// It lies next to another surface (a menu or a hint).
    fn is_owned(&self) -> bool {
        matches!(self.role, Role::Popup(..))
    }

    /// Where it stands among the windows now.
    fn order(&self) -> place::Order {
        match &self.role {
            Role::Placed(place) => place::order_of(place.level, self.covered),
            Role::Popup(..) => place::Order::Topmost,
        }
    }

    /// Put its window where it stands among the windows.
    fn restack(&self) {
        // SAFETY: a window of this thread; no pointers.
        unsafe {
            let _ = SetWindowPos(
                self.window.handle,
                match self.order() {
                    place::Order::Bottom => HWND_BOTTOM,
                    place::Order::Topmost => HWND_TOPMOST,
                },
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
    }
}

struct Host {
    // Dropped before the device they draw with.
    surfaces: Vec<Surface>,
    gpu: Option<Gpu>,
    instance: wgpu::Instance,
    /// Surfaces are layers of the desktop's compositor: what is not
    /// drawn on is seen through.
    see_through: bool,
    module: HINSTANCE,
    /// The window classes registered so far.
    classes: Vec<String>,
    watcher: Option<Window>,
    /// There while a popup is open.
    hooks: Option<Hooks>,
    /// The surface under the pointer.
    pointer_on: Option<SurfaceId>,
    /// Displays came, went or changed: every surface finds its place
    /// again.
    displays_changed: bool,
    /// When the bars that stepped down were last looked at again.
    covered_looked: Option<Instant>,
    started: Instant,
    timings: Timings,
    total_ms: f64,
    /// What handlers asked for and is not carried out yet.
    control: Control,
    /// Drag and drop on this thread, from the first surface that asks
    /// for it: `Some(None)` when Windows refused, which is not asked
    /// again. (After the surfaces, whose windows are places to drop on
    /// until they go.)
    ole: Option<Option<dnd::Ole>>,
    /// What the person set a double click to be in Windows: the time in
    /// milliseconds and the rectangle's sides in pixels. Read when the
    /// shell starts and when Windows says a setting changed.
    double_click: (u32, i32, i32),
}

/// What a double click is in Windows now (see [`input::double_click`]).
fn double_click_setting() -> (u32, i32, i32) {
    // SAFETY: questions without arguments of ours.
    unsafe {
        (
            GetDoubleClickTime(),
            GetSystemMetrics(SM_CXDOUBLECLK),
            GetSystemMetrics(SM_CYDOUBLECLK),
        )
    }
}

/// A drag that can begin.
struct ReadyDrag {
    object: IDataObject,
    allowed: DragActions,
    image: Option<DragImage>,
}

impl Host {
    /// Whether this thread can drag and be dropped on; set up at the
    /// first asking.
    fn ole(&mut self) -> bool {
        self.ole.get_or_insert_with(dnd::Ole::new).is_some()
    }

    /// The next drag a surface asked to begin.
    fn next_drag(&mut self) -> Option<(SurfaceId, Dragged)> {
        (!self.control.drags.is_empty()).then(|| self.control.drags.remove(0))
    }

    /// Make a drag ready: what is dragged as an object, and the surface
    /// it comes from told that the pointer is no longer its own. Windows
    /// takes the mouse for the drag, so the button's release never comes
    /// to the surface; told nothing, it would think the button held for
    /// ever.
    fn begin_drag(&mut self, from: SurfaceId, dragged: Dragged) -> Option<ReadyDrag> {
        if !self.surfaces.iter().any(|surface| surface.id == from) {
            return None;
        }
        let ready = if self.ole() {
            match dragged {
                Dragged::Plain(start) => match dnd::object_for(&start.payload) {
                    Ok(object) => Some(ReadyDrag {
                        object,
                        allowed: start.payload.allowed,
                        image: start.image,
                    }),
                    Err(error) => {
                        log::warn!("nothing to drag could be made: {error}");
                        None
                    }
                },
                Dragged::Object {
                    object,
                    allowed,
                    image,
                } => Some(ReadyDrag {
                    object,
                    allowed,
                    image,
                }),
            }
        } else {
            None
        };
        let Some(ready) = ready else {
            self.end_drag(from, DragOutcome::Cancelled);
            return None;
        };
        let surface = self.by_id(from)?;
        // SAFETY (both): a point of ours; no other pointers.
        let held = |key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
            GetKeyState(i32::from(key.0)) < 0
        };
        let mut pointer = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pointer);
            let _ = ReleaseCapture();
        }
        let at = crate::dnd::points(
            pointer.x - surface.area.x,
            pointer.y - surface.area.y,
            surface.scale,
        );
        for (key, button) in [
            (VK_LBUTTON, egui::PointerButton::Primary),
            (VK_RBUTTON, egui::PointerButton::Secondary),
            (VK_MBUTTON, egui::PointerButton::Middle),
        ] {
            if held(key) {
                surface.events.push(egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        surface.events.push(egui::Event::PointerGone);
        surface.redraw = true;
        if self.pointer_on == Some(from) {
            self.pointer_on = None;
        }
        Some(ready)
    }

    /// A drag out of a surface is over: the surface is told how.
    fn end_drag(&mut self, from: SurfaceId, outcome: DragOutcome) {
        if let Some(surface) = self.by_id(from) {
            surface.redraw = true;
            self.control
                .drag_news
                .push((from, DragEvent::Ended { outcome }));
        }
    }

    /// What the drop targets were told becomes what the surfaces are
    /// told: places in the surface's points, whatever its display's scale.
    fn take_in_drags(&mut self) {
        for told in dnd::told() {
            let window = match &told {
                dnd::Told::Entered { window, .. }
                | dnd::Told::Moved { window, .. }
                | dnd::Told::Left { window }
                | dnd::Told::Dropped { window, .. } => *window,
            };
            let Some(surface) = self
                .surfaces
                .iter_mut()
                .find(|surface| surface.window.handle.0 as isize == window)
            else {
                continue;
            };
            let (id, scale) = (surface.id, surface.scale);
            surface.redraw = true;
            let event = match told {
                dnd::Told::Entered { x, y, offered, .. } => {
                    // (What an earlier drag left is that drag's.)
                    self.control.dropped.retain(|(on, _)| *on != id);
                    DragEvent::Entered {
                        at: crate::dnd::points(x, y, scale),
                        offered,
                    }
                }
                dnd::Told::Moved { x, y, .. } => DragEvent::Moved {
                    at: crate::dnd::points(x, y, scale),
                },
                dnd::Told::Left { .. } => DragEvent::Left,
                dnd::Told::Dropped {
                    x,
                    y,
                    payload,
                    action,
                    object,
                    ..
                } => {
                    self.control.dropped.retain(|(on, _)| *on != id);
                    self.control.dropped.push((id, object));
                    DragEvent::Dropped {
                        at: crate::dnd::points(x, y, scale),
                        payload,
                        action,
                    }
                }
            };
            self.control.drag_news.push((id, event));
        }
    }

    fn by_id(&mut self, id: SurfaceId) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|surface| surface.id == id)
    }

    /// The window class of this name, registered when it is new; as
    /// Windows wants it, with a zero at its end.
    fn class(&mut self, name: &str) -> anyhow::Result<Vec<u16>> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        if !self.classes.iter().any(|known| known == name) {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(procedure),
                hInstance: self.module,
                lpszClassName: PCWSTR(wide.as_ptr()),
                ..Default::default()
            };
            if unsafe { RegisterClassExW(&class) } == 0 {
                anyhow::bail!(
                    "the window class {name}: {}",
                    windows::core::Error::from_win32()
                );
            }
            self.classes.push(name.to_string());
        }
        Ok(wide)
    }

    /// Do what handlers asked for: open, close, draw again.
    fn carry_out(&mut self) {
        for id in std::mem::take(&mut self.control.close) {
            if let Some(surface) = self.surfaces.iter().find(|surface| surface.id == id) {
                log::debug!("surface {id:?} is closed");
                give_back(surface.window.handle, surface.before);
            }
            // A surface's popups go with it (and before it).
            self.surfaces.retain(
                |surface| !matches!(&surface.role, Role::Popup(parent, ..) if *parent == id),
            );
            self.surfaces.retain(|surface| surface.id != id);
            if self.pointer_on == Some(id) {
                self.pointer_on = None;
            }
            self.control.drag_news.retain(|(on, _)| *on != id);
            self.control.dropped.retain(|(on, _)| *on != id);
            self.control.pressed.retain(|(on, _)| *on != id);
        }
        // (A menu that opened from a menu that is gone goes too.)
        loop {
            let open: Vec<SurfaceId> = self.surfaces.iter().map(|surface| surface.id).collect();
            let before = self.surfaces.len();
            self.surfaces
                .retain(|surface| surface.parent().is_none_or(|parent| open.contains(&parent)));
            if self.surfaces.len() == before {
                break;
            }
        }
        for (id, place, draw) in std::mem::take(&mut self.control.open) {
            let name = place.name.clone();
            match self.make(id, place, draw) {
                Ok(mut surface) => {
                    // Put somewhere before it is first shown: it never
                    // appears at the place it would have found itself.
                    if let Some(at) = self.control.put.iter().position(|(put, _)| *put == id) {
                        surface.pinned = Some(self.control.put.remove(at).1);
                    }
                    log::debug!("the surface {name} is opened as {id:?} on {}", surface.display);
                    self.surfaces.push(surface);
                    self.settle(self.surfaces.len() - 1);
                    self.show(self.surfaces.len() - 1);
                }
                Err(error) => log::error!("the surface {name} could not be opened: {error:#}"),
            }
        }
        for (id, parent, anchor, size, kind, draw) in std::mem::take(&mut self.control.popups) {
            match self.make_popup(id, parent, anchor, size, kind, draw) {
                Ok(surface) => {
                    log::debug!("a popup {id:?} is opened from surface {parent:?}");
                    self.surfaces.push(surface);
                    self.settle(self.surfaces.len() - 1);
                    self.show(self.surfaces.len() - 1);
                }
                Err(error) => log::error!("a popup could not be opened: {error:#}"),
            }
        }
        for (id, area) in std::mem::take(&mut self.control.put) {
            let Some(index) = self.surfaces.iter().position(|surface| surface.id == id) else {
                continue;
            };
            if self.surfaces[index].pinned != Some(area) {
                self.surfaces[index].pinned = Some(area);
                self.settle(index);
            }
        }
        for (id, width, height) in std::mem::take(&mut self.control.resize) {
            let Some(index) = self.surfaces.iter().position(|surface| surface.id == id) else {
                continue;
            };
            if let Role::Placed(place) = &mut self.surfaces[index].role {
                if (place.width, place.height) != (width, height) {
                    place.width = width;
                    place.height = height;
                    self.settle(index);
                }
            }
        }
        for id in std::mem::take(&mut self.control.redraw) {
            if let Some(surface) = self.by_id(id) {
                surface.redraw = true;
            }
        }
        for (id, standing) in std::mem::take(&mut self.control.drops) {
            let can = standing.is_none() || self.ole();
            let Some(surface) = self.by_id(id) else {
                continue;
            };
            let window = surface.window.handle;
            match standing {
                Some(standing) if can => {
                    if !surface.window.accepts {
                        surface.window.accepts = dnd::register(window);
                    }
                    dnd::set_standing(window, standing);
                }
                Some(_) => {}
                None => {
                    if std::mem::take(&mut surface.window.accepts) {
                        dnd::revoke(window);
                    }
                }
            }
        }
        for (id, answer) in std::mem::take(&mut self.control.answers) {
            if let Some(surface) = self.by_id(id) {
                dnd::set_answer(surface.window.handle, answer);
            }
        }
        for (id, level) in std::mem::take(&mut self.control.levels) {
            if let Some(index) = self.surfaces.iter().position(|surface| surface.id == id) {
                self.change_level(index, level);
            }
        }
        for (id, keyboard) in std::mem::take(&mut self.control.keyboards) {
            if let Some(index) = self.surfaces.iter().position(|surface| surface.id == id) {
                self.change_keyboard(index, keyboard);
            }
        }
        for (id, regions) in std::mem::take(&mut self.control.blurs) {
            if let Some(index) = self.surfaces.iter().position(|surface| surface.id == id) {
                self.surfaces[index].blur = regions;
                self.lay_backdrops(index);
            }
        }
        for (id, exclude) in std::mem::take(&mut self.control.captures) {
            if let Some(surface) = self.by_id(id) {
                if surface.hidden_from_capture != exclude {
                    surface.hidden_from_capture = exclude;
                    if !look::exclude_from_capture(surface.window.handle, exclude) {
                        log::warn!(
                            "Windows did not {} surface {id:?} of pictures of the screen",
                            if exclude { "leave out" } else { "put back into" }
                        );
                    }
                    surface.backdrops.exclude_from_capture(exclude);
                }
            }
        }
        // Listening to the whole desktop only while there is a popup to
        // close.
        // (And while a surface is open that stands beside the windows
        // without the keyboard: a press elsewhere puts it away too. The
        // Escape key is taken from the window that has the keyboard only
        // for a popup.)
        let popups = self.surfaces.iter().any(Surface::is_popup);
        let beside = self.surfaces.iter().any(Surface::is_beside);
        ESCAPE_IS_A_POPUPS.store(popups, std::sync::atomic::Ordering::Relaxed);
        if (popups || beside) && self.hooks.is_none() {
            self.hooks = Hooks::new(self.module);
            if self.hooks.is_none() {
                log::warn!("a press outside a popup will not close it: Windows refused the hooks");
            }
        } else if !(popups || beside) {
            self.hooks = None;
        }
        self.control.known = self.surfaces.iter().map(|surface| surface.id).collect();
        self.control.windows = self
            .surfaces
            .iter()
            .map(|surface| (surface.id, surface.window.handle.0 as isize))
            .collect();
    }

    fn make(&mut self, id: SurfaceId, place: Place, draw: Draw) -> anyhow::Result<Surface> {
        let all = displays();
        // On the display that is named, when it is there.
        let display = match &place.output {
            Some(wanted) => all
                .iter()
                .find(|display| place::is_display(wanted, &display.device))
                .with_context(|| format!("there is no display {wanted}"))?,
            None => display_at_pointer(&all).context("there is no display")?,
        }
        .device
        .clone();
        let class = self.class(&place::class_of(&place.name, false))?;
        let title: Vec<u16> = place
            .name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // Not in the taskbar or the window switcher.
        let mut style = WS_EX_TOOLWINDOW;
        if place::never_active(place.keyboard, place.level) {
            style |= WS_EX_NOACTIVATE;
        }
        if place::order_of(place.level, false) == place::Order::Topmost {
            style |= WS_EX_TOPMOST;
        }
        if place.level == Level::Marks {
            // The pointer and its presses go through it, to whatever
            // window is under it; and it is never the active window.
            style |= WS_EX_LAYERED | WS_EX_TRANSPARENT;
        }
        if self.see_through {
            // No picture of the window's own behind the layer drawn on.
            style |= WS_EX_NOREDIRECTIONBITMAP;
        }
        let handle = unsafe {
            CreateWindowExW(
                style,
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                self.module,
                None,
            )
        }
        .context("a window")?;
        let window = Window {
            handle,
            reserved: false,
            accepts: false,
        };
        change_facts(handle, |facts| {
            facts.activates = place.keyboard != Keyboard::Never;
            facts.bottom = place::stays_below(place.level);
            facts.pointer = arrow();
        });
        self.finish(id, window, Role::Placed(place), display, draw)
    }

    fn make_popup(
        &mut self,
        id: SurfaceId,
        parent: SurfaceId,
        anchor: egui::Rect,
        size: egui::Vec2,
        kind: Popped,
        draw: Draw,
    ) -> anyhow::Result<Surface> {
        let Some(owner) = self.surfaces.iter().find(|surface| surface.id == parent) else {
            anyhow::bail!("the surface it belongs to is gone");
        };
        let (owner_window, display) = (owner.window.handle, owner.display.clone());
        let name = match &owner.role {
            Role::Placed(place) => place.name.clone(),
            Role::Popup(..) => String::new(),
        };
        let hint = matches!(kind, Popped::Hint { .. });
        let class = self.class(&if hint {
            place::hint_class_of(&name)
        } else {
            place::class_of(&name, true)
        })?;
        // Above everything, and never the active window: the keyboard
        // stays where it is while a menu is open.
        let mut style = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
        if hint {
            // The pointer and its presses go through it, to whatever
            // window is under it, of this program or of another.
            style |= WS_EX_LAYERED | WS_EX_TRANSPARENT;
        }
        if self.see_through {
            style |= WS_EX_NOREDIRECTIONBITMAP;
        }
        // Owned by its surface's window: above it, and gone with it.
        let handle = unsafe {
            CreateWindowExW(
                style,
                PCWSTR(class.as_ptr()),
                PCWSTR(class.as_ptr()),
                WS_POPUP,
                0,
                0,
                1,
                1,
                owner_window,
                None,
                self.module,
                None,
            )
        }
        .context("a popup's window")?;
        let window = Window {
            handle,
            reserved: false,
            accepts: false,
        };
        if hint {
            // (A window of this kind is not shown until it is said how
            // much of it is seen: all of it.)
            unsafe {
                let _ = SetLayeredWindowAttributes(handle, COLORREF(0), 255, LWA_ALPHA);
            }
        }
        change_facts(handle, |facts| facts.pointer = arrow());
        self.finish(
            id,
            window,
            Role::Popup(parent, anchor, size, kind),
            display,
            draw,
        )
    }

    /// The graphics side of a surface that has its window.
    fn finish(
        &mut self,
        id: SurfaceId,
        window: Window,
        role: Role,
        display: String,
        draw: Draw,
    ) -> anyhow::Result<Surface> {
        let handle =
            raw_window_handle::RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(
                std::num::NonZeroIsize::new(window.handle.0 as isize).context("the window")?,
            ));
        // SAFETY: the window lives as long as `Surface`, whose graphics
        // surface is its first field and so is dropped before the window
        // it draws on.
        let target = unsafe {
            self.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_window_handle::RawDisplayHandle::Windows(
                        raw_window_handle::WindowsDisplayHandle::new(),
                    )),
                    raw_window_handle: handle,
                })
        }
        .context("a graphics surface")?;
        if self.gpu.is_none() {
            let adapter =
                pollster::block_on(self.instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&target),
                    ..Default::default()
                }))
                .context("no graphics adapter draws on this window")?;
            let (device, queue) =
                // A panel draws little, and is there all day: memory is
                // set aside in small pieces. (As the device comes, it
                // sets aside enough for a game: the spike held 322 MB of
                // its own so, and 144 MB with this.)
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    ..Default::default()
                }))
                    .context("the graphics device")?;
            let info = adapter.get_info();
            log::debug!("drawing with {} ({:?}, {:?})", info.name, info.backend, info.device_type);
            device.on_uncaptured_error(std::sync::Arc::new(|error| {
                log::warn!("the graphics device: {error}");
            }));
            self.gpu = Some(Gpu {
                device,
                queue,
                adapter,
            });
        }
        let gpu = self.gpu.as_ref().expect("just made");
        let capabilities = target.get_capabilities(&gpu.adapter);
        // A format without its own colour curve: egui gives colours as
        // they are to be shown.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or_else(|| capabilities.formats.first().copied())
            .context("the window has no format")?;
        let alpha = if capabilities
            .alpha_modes
            .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
        {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            capabilities
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
        let access = Access::new(window.handle);
        Ok(Surface {
            id,
            access,
            target,
            renderer,
            format,
            alpha,
            window,
            role,
            display,
            context: egui::Context::default(),
            draw,
            area: Area::default(),
            scale: 1.0,
            configured: (0, 0),
            failing: false,
            events: Vec::new(),
            focused: false,
            redraw: true,
            again_at: None,
            text: input::Utf16::default(),
            pointer: egui::CursorIcon::Default,
            covered: false,
            kept: None,
            pinned: None,
            before: 0,
            blur: Vec::new(),
            backdrops: look::Backdrops::default(),
            hidden_from_capture: false,
        })
    }

    /// Give a surface its place on its display as the display is now,
    /// and the picture the size that goes with it. A surface whose
    /// display is gone is closed, as a compositor does on Linux; one
    /// that named no display moves to another.
    fn settle(&mut self, index: usize) {
        let all = displays();
        let surface = &self.surfaces[index];
        let window = surface.window.handle;
        let mut kept_now = surface.kept;
        let (area, scale, display, reserve) = match &surface.role {
            Role::Placed(_) if surface.pinned.is_some() => {
                let pinned = surface.pinned.unwrap_or_default();
                let wholes: Vec<Area> = all.iter().map(|display| display.whole).collect();
                // Its display is gone and nothing of the rectangle is on
                // another: closed, as below; who put it there hears of the
                // displays too and opens it where it belongs now.
                let Some(display) = place::display_holding(pinned, &wholes).map(|at| &all[at])
                else {
                    self.control.close(surface.id);
                    return;
                };
                // A bar that is above the windows is told, as an
                // application bar is, when a window covers the whole
                // display (a film, a game), and steps down for it. It is
                // registered for the telling alone: it asks for no strip,
                // so nothing of the desktop is kept clear for it (who put
                // it here keeps its room).
                let listens =
                    matches!(&surface.role, Role::Placed(place) if place.level == Level::Above);
                if listens && !surface.window.reserved {
                    let mut data = bar_data(window);
                    unsafe {
                        SHAppBarMessage(ABM_NEW, &mut data);
                    }
                }
                (pinned, display.scale, display.device.clone(), listens)
            }
            Role::Placed(place) => {
                let found = all
                    .iter()
                    .find(|display| display.device == surface.display)
                    .or_else(|| match place.output {
                        Some(_) => None,
                        None => display_at_pointer(&all),
                    });
                let Some(display) = found else {
                    self.control.close(surface.id);
                    return;
                };
                match place::reserving_side(place) {
                    Some(side) => {
                        let depth = place::reserved_depth(place, side, display.scale);
                        let kept = keep_clear(
                            window,
                            surface.window.reserved,
                            surface.kept,
                            side,
                            display,
                            depth,
                        );
                        kept_now = Some(kept);
                        (
                            place::placed_in_strip(place, side, kept, display.scale),
                            display.scale,
                            display.device.clone(),
                            true,
                        )
                    }
                    None => {
                        // Everything but a cover keeps clear of the bars,
                        // the taskbar among them.
                        let room = if place.edge == Edge::Fill {
                            display.whole
                        } else {
                            display.work
                        };
                        (
                            place::placed(place, room, display.scale),
                            display.scale,
                            display.device.clone(),
                            false,
                        )
                    }
                }
            }
            Role::Popup(parent, anchor, size, kind) => {
                let Some(owner) = self.surfaces.iter().find(|owner| owner.id == *parent) else {
                    self.control.close(surface.id);
                    return;
                };
                let scale = owner.scale;
                let across = |points: f32| (points * scale).round() as i32;
                let anchor = Area::new(
                    owner.area.x + across(anchor.min.x),
                    owner.area.y + across(anchor.min.y),
                    across(anchor.width()).max(1),
                    across(anchor.height()).max(1),
                );
                let whole = all
                    .iter()
                    .find(|display| display.device == owner.display)
                    .map_or(owner.area, |display| display.whole);
                let size = (across(size.x), across(size.y));
                let at = match kind {
                    Popped::Hint { beside: true } => {
                        place::beside(anchor, size, whole, place::pixels(HINT_GAP, scale))
                    }
                    Popped::Sub => {
                        place::beside_line(anchor, size, whole, place::pixels(SUB_GAP, scale))
                    }
                    _ => place::popup_at(anchor, size, whole, place::pixels(POPUP_GAP, scale)),
                };
                (at, scale, owner.display.clone(), false)
            }
        };
        let surface = &mut self.surfaces[index];
        surface.window.reserved |= reserve;
        surface.kept = kept_now;
        if reserve {
            change_facts(window, |facts| facts.reserved = true);
        }
        surface.display = display;
        if surface.area != area {
            surface.area = area;
            unsafe {
                let _ = SetWindowPos(
                    window,
                    None,
                    area.x,
                    area.y,
                    area.width,
                    area.height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
        }
        if (surface.scale - scale).abs() > f32::EPSILON {
            surface.scale = scale;
            surface.redraw = true;
        }
        if surface.configured != (area.width, area.height) {
            if let Some(gpu) = &self.gpu {
                surface.configure_target(&gpu.device);
            }
            surface.redraw = true;
        }
        // What shows the desktop soft under it is where it is now, in
        // the pixels of the display it is on now.
        if !surface.backdrops.is_empty() {
            self.lay_backdrops(index);
        }
    }

    /// The windows under a surface that show the desktop soft, made to
    /// be the parts it last asked for, where it is now.
    fn lay_backdrops(&mut self, index: usize) {
        let (module, available) = (self.module, self.control.blur);
        let surface = &mut self.surfaces[index];
        let name = match &surface.role {
            Role::Placed(place) => place.name.clone(),
            Role::Popup(..) => String::new(),
        };
        let wanted = if available {
            look::windows_for(
                &surface.blur,
                (surface.area.x, surface.area.y),
                surface.scale,
            )
        } else {
            Vec::new()
        };
        surface
            .backdrops
            .set(module, surface.window.handle, &name, &wanted);
    }

    /// Put an open surface on another level among the windows.
    fn change_level(&mut self, index: usize, level: Level) {
        let surface = &mut self.surfaces[index];
        let Role::Placed(place) = &mut surface.role else {
            return;
        };
        if place.level == level {
            return;
        }
        if !place::can_change_level(place.level, level) {
            log::warn!(
                "surface {:?} stays {:?}: a surface is marks from the start or never",
                surface.id,
                place.level
            );
            return;
        }
        place.level = level;
        let window = surface.window.handle;
        // (First: its procedure keeps it below while this says so.)
        change_facts(window, |facts| facts.bottom = place::stays_below(level));
        // A bar above the windows listens for a window that covers the
        // display; that is asked for when it is given its place.
        self.settle(index);
        // (Its backdrops follow by its window's own news.)
        self.surfaces[index].restack();
    }

    /// Change how an open surface takes the keyboard.
    fn change_keyboard(&mut self, index: usize, keyboard: Keyboard) {
        let surface = &mut self.surfaces[index];
        let Role::Placed(place) = &mut surface.role else {
            return;
        };
        if place.keyboard == keyboard {
            return;
        }
        let was = std::mem::replace(&mut place.keyboard, keyboard);
        let never = place::never_active(keyboard, place.level);
        let window = surface.window.handle;
        // SAFETY: a window of this thread; its style as a number.
        unsafe {
            let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
            let quiet = WS_EX_NOACTIVATE.0 as isize;
            let wanted = if never { style | quiet } else { style & !quiet };
            if wanted != style {
                SetWindowLongPtrW(window, GWL_EXSTYLE, wanted);
            }
        }
        change_facts(window, |facts| facts.activates = keyboard != Keyboard::Never);
        if keyboard == Keyboard::Exclusive {
            let before = take_keyboard(window);
            let ours = self
                .surfaces
                .iter()
                .any(|surface| surface.window.handle.0 as isize == before);
            self.surfaces[index].before = if ours { 0 } else { before };
        } else if was == Keyboard::Exclusive {
            let before = std::mem::take(&mut self.surfaces[index].before);
            give_back(window, before);
        }
    }

    fn settle_all(&mut self) {
        // Popups after the surfaces they lie next to.
        for popups in [false, true] {
            for index in 0..self.surfaces.len() {
                if self.surfaces[index].is_owned() == popups {
                    self.settle(index);
                }
            }
        }
    }

    /// Show a surface that has its place, without taking the keyboard
    /// from the window that has it; unless the keyboard is what the
    /// surface is there for.
    fn show(&mut self, index: usize) {
        let surface = &self.surfaces[index];
        let window = surface.window.handle;
        unsafe {
            let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        }
        surface.restack();
        if matches!(&surface.role, Role::Placed(place) if place.keyboard == Keyboard::Exclusive) {
            let before = take_keyboard(window);
            // One of the shell's own surfaces is not gone back to: it may
            // be the one that is closing as this one opens.
            let ours = self
                .surfaces
                .iter()
                .any(|surface| surface.window.handle.0 as isize == before);
            self.surfaces[index].before = if ours { 0 } else { before };
        }
    }

    /// Close every popup (a press elsewhere, Escape).
    fn close_popups(&mut self) {
        for surface in &self.surfaces {
            if surface.is_popup() {
                self.control.close.push(surface.id);
            }
        }
    }

    /// Something a window was told becomes something egui is told.
    fn take_in(&mut self, news: News) {
        // Windows' shell started again (it was ended, or fell over): it
        // knows of no application bar any more. Every surface that was
        // one is one again when it is next given its place: else a panel
        // would no longer hear of windows that cover the display, and a
        // strip kept clear for one would be given away.
        if news.message != 0 && news.message == shell_started() {
            for surface in &mut self.surfaces {
                if surface.window.reserved {
                    surface.window.reserved = false;
                    surface.kept = None;
                    change_facts(surface.window.handle, |facts| facts.reserved = false);
                }
            }
            self.displays_changed = true;
            return;
        }
        match news.message {
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                log::debug!("the displays changed");
                self.displays_changed = true;
                return;
            }
            // What application bars leave of a display changed (the
            // number is `SPI_SETWORKAREA`).
            WM_SETTINGCHANGE => {
                if news.wparam == 0x2F {
                    self.displays_changed = true;
                }
                // (Whatever setting it was: how fast a double click is
                // may be it. Every surface takes it with its next frame.)
                self.double_click = double_click_setting();
                return;
            }
            PRESSED_AT => {
                // A press on one of the shell's own surfaces is for that
                // surface to act on (the button that opened a menu puts
                // it away); anywhere else it closes the popups.
                let (x, y) = (news.wparam as u32 as i32, news.lparam as i32);
                // (A hint is not in the way: the press went through it.)
                if !self
                    .surfaces
                    .iter()
                    .any(|surface| !surface.is_hint() && surface.area.holds(x, y))
                {
                    self.close_popups();
                    for surface in &mut self.surfaces {
                        if surface.is_beside() {
                            if !self.control.outside.contains(&surface.id) {
                                self.control.outside.push(surface.id);
                            }
                            surface.redraw = true;
                        }
                    }
                }
                return;
            }
            ESCAPE => {
                // One step back: of menus that opened from each other the
                // last one goes (a menu alone is the last), and the one it
                // opened from is drawn again, to see that it is gone.
                let last = self.surfaces.iter().rev().find(|surface| surface.is_popup());
                if let Some((id, parent)) = last.map(|surface| (surface.id, surface.parent())) {
                    self.control.close.push(id);
                    if let Some(parent) = parent.and_then(|parent| self.by_id(parent)) {
                        parent.redraw = true;
                    }
                }
                return;
            }
            IN_FRONT => {
                let ours = self
                    .surfaces
                    .iter()
                    .any(|surface| surface.window.handle.0 as usize == news.wparam);
                if !ours {
                    self.close_popups();
                }
                return;
            }
            _ => {}
        }
        let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| surface.window.handle.0 as isize == news.window)
        else {
            return;
        };
        let modifiers = news.modifiers;
        let surface = &mut self.surfaces[index];
        match news.message {
            WM_MOUSEMOVE => {
                self.pointer_on = Some(surface.id);
                surface.events.push(egui::Event::PointerMoved(input::point(
                    news.lparam,
                    surface.scale,
                )));
            }
            WM_MOUSELEAVE => {
                log::trace!("the pointer left surface {:?}", surface.id);
                if self.pointer_on == Some(surface.id) {
                    self.pointer_on = None;
                }
                surface.events.push(egui::Event::PointerGone);
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                // The wheel's message says where the pointer is (on the
                // desktop, not on the surface), and egui is told so first:
                // what the wheel turns over is found by the pointer's
                // place, and that is only known from the pointer's moves.
                // A pointer that came to rest here before the surface was
                // there, or over which another surface came and went (a
                // hint), has not moved; the wheel would turn over nothing.
                let at =
                    input::point_on(news.lparam, surface.area.x, surface.area.y, surface.scale);
                log::trace!("the wheel over surface {:?} at {at:?}", surface.id);
                self.pointer_on = Some(surface.id);
                surface.events.push(egui::Event::PointerMoved(at));
                surface.events.push(egui::Event::MouseWheel {
                    // Windows gives a wheel and a touchpad the same message.
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Line,
                    delta: input::wheel(news.wparam, news.message == WM_MOUSEHWHEEL),
                    phase: egui::TouchPhase::Move,
                    modifiers,
                });
            }
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
                let pressed = matches!(news.message, WM_KEYDOWN | WM_SYSKEYDOWN);
                // egui wants to hear of the keys held, too.
                surface
                    .events
                    .push(egui::Event::ModifiersChanged(modifiers));
                // The key for a menu has no name in egui: it is told as
                // Shift and F10, which Windows itself takes for the same.
                let (key, modifiers) = if input::is_menu_key(news.wparam as u16) {
                    (
                        Some(egui::Key::F10),
                        egui::Modifiers {
                            shift: true,
                            ..modifiers
                        },
                    )
                } else {
                    (input::key_of(news.wparam as u16), modifiers)
                };
                if let Some(key) = key {
                    surface.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: pressed && input::is_repeat(news.lparam),
                        modifiers,
                    });
                }
            }
            WM_CHAR => {
                if let Some(letter) = surface.text.push(news.wparam as u16) {
                    if input::is_text(letter) {
                        surface.events.push(egui::Event::Text(letter.to_string()));
                    }
                }
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                surface.focused = news.message == WM_SETFOCUS;
                if surface.focused {
                    log::debug!("surface {:?} has the keyboard", surface.id);
                } else {
                    log::debug!("surface {:?} lost the keyboard", surface.id);
                }
                surface
                    .events
                    .push(egui::Event::WindowFocused(surface.focused));
            }
            WM_CLOSE => {
                let id = surface.id;
                log::debug!("Windows asked surface {id:?} to close");
                self.control.close(id);
                return;
            }
            WM_PAINT => {}
            BAR_NEWS => {
                match news.wparam as u32 {
                    // Another bar came or went: the place may be another.
                    ABN_POSCHANGED => self.displays_changed = true,
                    // A window covers the whole display, or no longer: a
                    // bar is not shown over a film or a game.
                    ABN_FULLSCREENAPP => {
                        // (Not for the desktop or a surface of gevrek's
                        // own that has the keyboard: the one the zones
                        // are drawn on fills its display, and Windows
                        // takes it for a window that covers it.)
                        let covered = news.lparam != 0 && !front_is_no_programs();
                        // (Kept whatever its level is now: a surface
                        // that becomes a bar while a film runs steps
                        // down at once.)
                        if surface.covered != covered {
                            let before = surface.order();
                            surface.covered = covered;
                            if surface.order() != before {
                                surface.restack();
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }
            message => {
                let Some((button, pressed)) = input::button(message, news.wparam) else {
                    return;
                };
                let at = input::point(news.lparam, surface.scale);
                if pressed {
                    let press = crate::Press {
                        button,
                        at,
                        time_ms: news.time,
                    };
                    self.control.pressed.push((surface.id, press));
                    // (A surface that never asks leaves them lying.)
                    let over = self.control.pressed.len().saturating_sub(PRESSES_KEPT);
                    self.control.pressed.drain(..over);
                }
                surface.events.push(egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers,
                });
            }
        }
        surface.redraw = true;
    }

    fn draw(&mut self, index: usize) {
        let began = Instant::now();
        let Some(gpu) = &self.gpu else {
            return;
        };
        let surface = &mut self.surfaces[index];
        surface.redraw = false;
        if surface.area.width <= 0 || surface.area.height <= 0 {
            return;
        }
        let pixels = [surface.area.width as u32, surface.area.height as u32];
        let points = egui::vec2(
            pixels[0] as f32 / surface.scale,
            pixels[1] as f32 / surface.scale,
        );
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: std::mem::take(&mut surface.events),
            // A surface without the keyboard blinks no cursor.
            focused: surface.focused,
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(surface.scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };
        // A double click is what it is in Windows, not what the toolkit
        // thinks: on this surface's display, as the setting is now.
        let (delay, apart) = input::double_click(
            self.double_click.0,
            self.double_click.1,
            self.double_click.2,
            surface.scale,
        );
        surface.context.options_mut(|options| {
            options.input_options.max_double_click_delay = delay;
            options.input_options.max_click_dist = apart;
        });
        let mut flow = Flow::Continue;
        let draw = &mut surface.draw;
        let control = &mut self.control;
        let mut output = surface.context.run_ui(input, |root| {
            if draw(root, control) == Flow::Close {
                flow = Flow::Close;
            }
        });
        // The pictures egui made or changed (its font atlas first of
        // all) are taken in whatever becomes of this frame.
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                surface
                    .renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        output.textures_delta.set.clear();
        let freed = std::mem::take(&mut output.textures_delta.free);
        // What the frame shows, for a screen reader that listens.
        if let (Some(update), Some(access)) = (
            output.platform_output.accesskit_update.take(),
            surface.access.as_mut(),
        ) {
            if let Some(events) = access.adapter.update_if_active(|| update) {
                events.raise();
            }
        }
        if flow == Flow::Close {
            let id = surface.id;
            self.control.close(id);
            return;
        }
        // The pointer egui wants over this surface.
        let wanted = output.platform_output.cursor_icon;
        if wanted != surface.pointer {
            surface.pointer = wanted;
            let pointer = input::pointer_of(wanted).map_or(0, load_pointer);
            change_facts(surface.window.handle, |facts| facts.pointer = pointer);
            if self.pointer_on == Some(surface.id) {
                unsafe {
                    SetCursor(HCURSOR(pointer as *mut _));
                }
            }
        }
        // When egui wants the next frame.
        if let Some(viewport) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            if viewport.repaint_delay.is_zero() {
                surface.redraw = true;
            } else if viewport.repaint_delay < Duration::from_secs(3600) {
                surface.again_at = Some(Instant::now() + viewport.repaint_delay);
            }
        }
        let jobs = surface
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: pixels,
            pixels_per_point: output.pixels_per_point,
        };
        // With a frame asked for straight after another, this waits
        // until the display has shown the one before: the loop cannot
        // draw faster than the display shows. The wait is no work, and
        // is not counted as the frame's time (the Linux half waits for
        // the compositor outside its drawing, and does not count it).
        let asked = Instant::now();
        let acquired = surface.target.get_current_texture();
        let waited = asked.elapsed().as_secs_f64() * 1000.0;
        let frame = match acquired {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                if std::mem::take(&mut surface.failing) {
                    log::debug!("surface {:?} has frames to draw into again", surface.id);
                }
                frame
            }
            other => {
                if surface.failing {
                    log::trace!("still no frame to draw into on surface {:?}: {other:?}", surface.id);
                } else {
                    log::debug!("no frame to draw into on surface {:?}: {other:?}", surface.id);
                    surface.failing = true;
                }
                surface.configure_target(&gpu.device);
                // Not at once: a window that cannot be drawn on now (the
                // session is locked, the device is being reset) would
                // keep the loop turning.
                surface.again_at = Some(Instant::now() + Duration::from_millis(100));
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let before =
            surface
                .renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            surface.renderer.render(&mut pass, &jobs, &screen);
        }
        gpu.queue
            .submit(before.into_iter().chain(std::iter::once(encoder.finish())));
        for id in &freed {
            surface.renderer.free_texture(id);
        }
        gpu.queue.present(frame);

        let took = began.elapsed().as_secs_f64() * 1000.0 - waited;
        log::trace!("a frame in {took:.2} ms, after {waited:.2} ms of waiting for the display");
        if self.timings.frames == 0 {
            self.timings.first_frame_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        }
        self.timings.frames += 1;
        self.total_ms += took;
        self.timings.worst_ms = self.timings.worst_ms.max(took);
    }
}

/// Give a window the keyboard. The answer is the window that had it.
///
/// Windows lets a program do that only when it is the one the user works
/// in, or "received the last input event". The shell is neither when it
/// is asked to open its launcher: the key was pressed in another window
/// and heard by the window manager. `SetForegroundWindow` then returns
/// without having done anything. An input event of our own (a movement
/// of the mouse by nothing) makes this program the one that received the
/// last input, after which the call is honoured; the window manager does
/// the same for the windows it focuses. Where even that is refused (a
/// window of a program with more rights has the keyboard), a click on the
/// surface gives it the keyboard.
fn take_keyboard(window: HWND) -> isize {
    unsafe {
        let before = GetForegroundWindow();
        if !(SetForegroundWindow(window).as_bool() && GetForegroundWindow() == window) {
            let nothing = INPUT {
                r#type: INPUT_MOUSE,
                ..Default::default()
            };
            SendInput(&[nothing], std::mem::size_of::<INPUT>() as i32);
            if SetForegroundWindow(window).as_bool() && GetForegroundWindow() == window {
                log::debug!("the surface has the keyboard, at the second asking");
            } else {
                log::warn!("Windows did not give the surface the keyboard; a click on it does");
            }
        } else {
            log::debug!("the surface has the keyboard, at the first asking");
        }
        let _ = SetFocus(window);
        if before == window {
            0
        } else {
            before.0 as isize
        }
    }
}

/// A surface that took the keyboard closes: the window that had it before
/// has it again, when the surface still has it now (when the user went
/// elsewhere meanwhile, the keyboard is where the user put it).
fn give_back(window: HWND, before: isize) {
    if before == 0 {
        return;
    }
    unsafe {
        let before = HWND(before as *mut _);
        if GetForegroundWindow() == window && IsWindow(before).as_bool() {
            let _ = SetForegroundWindow(before);
        }
    }
}

/// Make a window an application bar at a side of a display and have
/// Windows keep `depth` pixels there clear of other windows. The answer
/// is the strip that is kept: next to the bars that were there first.
///
/// Asked again with nothing changed, Windows is not told again: it would
/// tell every window that the room changed, this one too, without end.
fn keep_clear(
    window: HWND,
    registered: bool,
    last: Option<Area>,
    side: Side,
    display: &Display,
    depth: i32,
) -> Area {
    let mut data = bar_data(window);
    unsafe {
        if !registered {
            SHAppBarMessage(ABM_NEW, &mut data);
        }
        data.uEdge = side.number();
        data.rc = rect_of(place::strip(side, display.whole, depth));
        SHAppBarMessage(ABM_QUERYPOS, &mut data);
        let kept = place::settled(side, area_of(data.rc), depth);
        if last == Some(kept) {
            return kept;
        }
        data.rc = rect_of(kept);
        SHAppBarMessage(ABM_SETPOS, &mut data);
        area_of(data.rc)
    }
}

/// The system's pointer of this number, as a number itself.
fn load_pointer(number: u16) -> isize {
    // A system pointer is named by a number in place of a name.
    unsafe { LoadCursorW(None, PCWSTR(number as usize as *const u16)) }
        .map_or(0, |pointer| pointer.0 as isize)
}

fn arrow() -> isize {
    input::pointer_of(egui::CursorIcon::Default).map_or(0, load_pointer)
}
